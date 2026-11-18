//! Keeping the libraries' resolution for the next version of a project.
//!
//! An editor checks many changed copies of one project. The standard
//! modules and the imported libraries are the same in every copy, and
//! nothing in a library can name the project, so resolving them again
//! would give the same result. [`ResolvedLibraries`] is that result: the
//! resolver's whole state after its first phase (every library collected
//! and lowered), with what it was computed from, to check that a new
//! module set has the same libraries before it is reused.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use stave_lang::{Diagnostic, FileId};
use stave_modules::{FileImport, ModuleHeader, ModuleId, ModuleSet, ModuleSource};

use super::{Arenas, Counters, Entry, Lazy, Resolver, libraries_of};
use crate::defs::{FileScope, ModuleDefs, RExpr, Symbol};
use crate::ids::{AssemblyDefId, ComponentId, KindId, LetDefId, ParamDefId};
use crate::types::Ty;

/// The libraries of a module set, resolved: what [`super::resolve_keeping`]
/// keeps and [`super::resolve_reusing`] starts from.
#[derive(Clone)]
pub struct ResolvedLibraries {
    inputs: Inputs,
    state: State,
}

impl std::fmt::Debug for ResolvedLibraries {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResolvedLibraries")
            .field("modules", &self.inputs.modules.len())
            .finish_non_exhaustive()
    }
}

impl ResolvedLibraries {
    pub(super) fn new(modules: &ModuleSet, state: State) -> Self {
        ResolvedLibraries {
            inputs: Inputs::of(modules),
            state,
        }
    }

    /// `true` if `modules` has exactly the libraries these were resolved
    /// from: the same modules with the same ids, names, sources and
    /// headers, and the same files with the same ids, paths, texts and
    /// imports. The project's own files may differ in anything but their
    /// number.
    pub fn matches(&self, modules: &ModuleSet) -> bool {
        self.inputs.matches(modules)
    }

    pub(super) fn state(&self) -> State {
        self.state.clone()
    }
}

/// What the libraries were resolved from.
#[derive(Debug, Clone, PartialEq)]
struct Inputs {
    root: ModuleId,
    prelude: Vec<ModuleId>,
    module_count: usize,
    /// The ids of the project's files: the libraries' files are numbered
    /// after them.
    root_files: Vec<FileId>,
    modules: Vec<LibraryInput>,
}

#[derive(Debug, Clone, PartialEq)]
struct LibraryInput {
    id: ModuleId,
    name: String,
    source: ModuleSource,
    header: Option<ModuleHeader>,
    files: Vec<FileInput>,
}

#[derive(Debug, Clone, PartialEq)]
struct FileInput {
    file: FileId,
    path: String,
    text: String,
    imports: Vec<FileImport>,
}

impl Inputs {
    fn of(modules: &ModuleSet) -> Self {
        Inputs {
            root: modules.root,
            prelude: modules.prelude.clone(),
            module_count: modules.modules.len(),
            root_files: root_files(modules).collect(),
            modules: libraries_of(modules)
                .map(|module| LibraryInput {
                    id: module.id,
                    name: module.name.clone(),
                    source: module.source.clone(),
                    header: module.header,
                    files: module
                        .files
                        .iter()
                        .map(|file| FileInput {
                            file: file.file,
                            path: file.path.clone(),
                            text: text(modules, file.file).to_owned(),
                            imports: file.imports.clone(),
                        })
                        .collect(),
                })
                .collect(),
        }
    }

    /// Compares without building a second [`Inputs`].
    fn matches(&self, modules: &ModuleSet) -> bool {
        let same_shape = self.root == modules.root
            && self.prelude == modules.prelude
            && self.module_count == modules.modules.len()
            && self.root_files.iter().copied().eq(root_files(modules));
        same_shape
            && self.modules.len() == libraries_of(modules).count()
            && self
                .modules
                .iter()
                .zip(libraries_of(modules))
                .all(|(kept, module)| {
                    kept.id == module.id
                        && kept.name == module.name
                        && kept.source == module.source
                        && kept.header == module.header
                        && kept.files.len() == module.files.len()
                        && kept.files.iter().zip(&module.files).all(|(kept, file)| {
                            kept.file == file.file
                                && kept.path == file.path
                                && kept.imports == file.imports
                                && kept.text == text(modules, file.file)
                        })
                })
    }
}

fn root_files(modules: &ModuleSet) -> impl Iterator<Item = FileId> + '_ {
    modules
        .module(modules.root)
        .into_iter()
        .flat_map(|module| module.files.iter().map(|file| file.file))
}

fn text(modules: &ModuleSet, file: FileId) -> &str {
    modules
        .sources
        .get(file)
        .map_or("", stave_lang::diagnostics::SourceFile::text)
}

/// Everything the resolver holds except the module set and the syntax
/// references, which collecting the libraries again gives back.
#[derive(Clone)]
pub(super) struct State {
    diagnostics: Vec<Diagnostic>,
    module_names: Vec<BTreeMap<String, Entry>>,
    component_names: Vec<BTreeMap<String, Entry>>,
    kind_names: HashMap<KindId, BTreeMap<String, Symbol>>,
    kind_cut_names: HashMap<KindId, Vec<String>>,
    imports: HashMap<FileId, BTreeMap<String, ModuleId>>,
    prelude: BTreeMap<String, Entry>,
    module_items: Vec<Vec<super::DefId>>,
    component_items: Vec<Vec<super::DefId>>,
    counters: Counters,
    out: Arenas,
    types: HashMap<super::DefId, Lazy<Ty>>,
    kind_param_types: HashMap<(KindId, usize), Lazy<Ty>>,
    early_params: HashMap<ParamDefId, (Ty, Option<RExpr>)>,
    early_lets: HashMap<LetDefId, RExpr>,
    files: BTreeMap<FileId, FileScope>,
    broken: BTreeMap<ModuleId, BTreeSet<String>>,
    poisoned: BTreeSet<super::DefId>,
    early_kind_params: HashMap<(KindId, usize), (Ty, Option<RExpr>)>,
    assembly_components: HashMap<AssemblyDefId, Lazy<Option<ComponentId>>>,
    headers: BTreeMap<ModuleId, ModuleDefs>,
}

impl Resolver<'_> {
    /// The resolver's state, to be restored by [`Resolver::restore`].
    pub(super) fn snapshot(&self) -> State {
        // Destructured in full, so a new field can't be left out.
        let Resolver {
            modules: _,
            origins: _,
            in_kind_require: _,
            diagnostics,
            module_names,
            component_names,
            kind_names,
            kind_cut_names,
            imports,
            prelude,
            module_items,
            component_items,
            counters,
            out,
            types,
            kind_param_types,
            early_params,
            early_lets,
            files,
            broken,
            poisoned,
            early_kind_params,
            assembly_components,
            headers,
        } = self;
        State {
            diagnostics: diagnostics.clone(),
            module_names: module_names.clone(),
            component_names: component_names.clone(),
            kind_names: kind_names.clone(),
            kind_cut_names: kind_cut_names.clone(),
            imports: imports.clone(),
            prelude: prelude.clone(),
            module_items: module_items.clone(),
            component_items: component_items.clone(),
            counters: counters.clone(),
            out: out.clone(),
            types: types.clone(),
            kind_param_types: kind_param_types.clone(),
            early_params: early_params.clone(),
            early_lets: early_lets.clone(),
            files: files.clone(),
            broken: broken.clone(),
            poisoned: poisoned.clone(),
            early_kind_params: early_kind_params.clone(),
            assembly_components: assembly_components.clone(),
            headers: headers.clone(),
        }
    }

    /// Puts back a [`State`] taken after the library phase. The syntax
    /// references must already be there: collect the libraries first.
    pub(super) fn restore(&mut self, state: State) {
        let State {
            diagnostics,
            module_names,
            component_names,
            kind_names,
            kind_cut_names,
            imports,
            prelude,
            module_items,
            component_items,
            counters,
            out,
            types,
            kind_param_types,
            early_params,
            early_lets,
            files,
            broken,
            poisoned,
            early_kind_params,
            assembly_components,
            headers,
        } = state;
        self.diagnostics = diagnostics;
        self.module_names = module_names;
        self.component_names = component_names;
        self.kind_names = kind_names;
        self.kind_cut_names = kind_cut_names;
        self.in_kind_require = false;
        self.imports = imports;
        self.prelude = prelude;
        self.module_items = module_items;
        self.component_items = component_items;
        self.counters = counters;
        self.out = out;
        self.types = types;
        self.kind_param_types = kind_param_types;
        self.early_params = early_params;
        self.early_lets = early_lets;
        self.files = files;
        self.broken = broken;
        self.poisoned = poisoned;
        self.early_kind_params = early_kind_params;
        self.assembly_components = assembly_components;
        self.headers = headers;
    }
}
