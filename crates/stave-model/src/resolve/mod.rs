//! Name resolution and type checking (layer 2).
//!
//! [`resolve`] turns the syntax trees of a [`ModuleSet`] into [`Defs`]:
//! every name is resolved to a [`Symbol`] following the four namespaces of
//! the grammar ("Names"), every expression gets a static [`Ty`], and the
//! declaration tables (fields, roles, settings) are checked.
//!
//! It runs in two passes over the modules, in module order and then file
//! and source order:
//!
//! 1. **Collect**: give every declaration its id and enter its name in the
//!    scope of its module or component, reporting duplicates, items a
//!    library can't hold, and kinds or recipes that hide the prelude.
//! 2. **Lower**: resolve and type-check each declaration's body. A param or
//!    `let` whose type is needed before its turn is lowered early and the
//!    result kept, so nothing is reported twice.

mod expr;
mod hardware;
mod items;
mod kinds;
mod mechanism;

use std::collections::{BTreeMap, HashMap};

use stave_lang::ast::{self, Span};
use stave_lang::{Code, Diagnostic, FileId, Label};
use stave_modules::{ModuleHeader, ModuleId, ModuleSet, ModuleSource};
use stave_schema::functions::Builtin;
use stave_schema::kinds::RoleType;

use crate::defs::{
    AssemblyDef, ComponentDef, DefId, Defs, HardwareDef, JointDef, KindDef, LetDef, ModuleDefs,
    ParamDef, PartDef, ProfileDef, RExpr, RecipeDef, RegionDef, RequireDef, SceneDef, Scope,
    StockDef, Symbol,
};
use crate::ids::{
    AssemblyDefId, ComponentId, HardwareDefId, JointDefId, KindId, LetDefId, ParamDefId, PartDefId,
    ProfileId, RecipeId, RegionDefId, RequireDefId, SceneDefId, StockId,
};
use crate::types::{KindTy, Ty};

/// Resolves and type-checks every module of `modules`. Returns the
/// definitions and the diagnostics of this stage (not the module set's).
pub fn resolve(modules: &ModuleSet) -> (Defs, Vec<Diagnostic>) {
    let mut resolver = Resolver::new(modules);
    resolver.collect();
    resolver.lower_all();
    resolver.finish()
}

/// A reference from a declaration id back to its syntax.
#[derive(Clone, Copy)]
enum Syntax<'a> {
    Param(&'a ast::Param),
    Let(&'a ast::Let),
    Region(&'a ast::Region),
    Part(&'a ast::Part),
    Joint(&'a ast::Joint),
    Hardware(&'a ast::Hardware),
    Assembly(&'a ast::Assembly),
    Require(&'a ast::Require),
    Stock(&'a ast::Stock),
    Profile(&'a ast::Profile),
    Kind(&'a ast::Kind),
    Recipe(&'a ast::Recipe),
    Component(&'a ast::Component),
    Scene(&'a ast::Scene),
}

/// Where an item is declared, for lowering it later.
#[derive(Clone, Copy)]
pub(crate) struct Origin<'a> {
    syntax: Syntax<'a>,
    module: ModuleId,
    file: FileId,
    scope: Scope,
    docs: &'a [String],
    private: bool,
    span: Span,
}

/// A name in a scope table.
#[derive(Debug, Clone, Copy)]
struct Entry {
    def: DefId,
    private: bool,
    file: FileId,
    span: Span,
}

/// Where an expression is written: what its names resolve against.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Cx {
    module: ModuleId,
    file: FileId,
    body: Body,
    /// The array being declared, for `[same]` and its own unindexed name.
    array: Option<DefId>,
}

/// The innermost body a name is looked up in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Body {
    Module,
    Component(ComponentId),
    Kind(KindId),
}

/// Memo state of something worked out on demand.
#[derive(Debug, Clone)]
enum Lazy<T> {
    Busy,
    Done(T),
}

pub(crate) struct Resolver<'a> {
    modules: &'a ModuleSet,
    diagnostics: Vec<Diagnostic>,
    origins: HashMap<DefId, Origin<'a>>,
    /// Item names per module.
    module_names: Vec<BTreeMap<String, Entry>>,
    /// Item names per component.
    component_names: Vec<BTreeMap<String, Entry>>,
    /// Role, param and made-part names per kind.
    kind_names: HashMap<KindId, BTreeMap<String, Symbol>>,
    /// Imports per file.
    imports: HashMap<FileId, BTreeMap<String, ModuleId>>,
    /// Public items of the standard modules.
    prelude: BTreeMap<String, Entry>,
    /// Items in order per module, and per component.
    module_items: Vec<Vec<DefId>>,
    component_items: Vec<Vec<DefId>>,
    counters: Counters,
    out: Arenas,
    /// Types of declarations, worked out on demand.
    types: HashMap<DefId, Lazy<Ty>>,
    kind_param_types: HashMap<(KindId, usize), Lazy<Ty>>,
    /// Lowered param defaults and let values, kept when lowered early.
    early_params: HashMap<ParamDefId, (Ty, Option<RExpr>)>,
    early_lets: HashMap<LetDefId, RExpr>,
    /// Per file: module and imports, for [`Defs::files`].
    files: BTreeMap<FileId, crate::defs::FileScope>,
    /// Names of items with syntax errors, per module.
    broken: BTreeMap<ModuleId, std::collections::BTreeSet<String>>,
    /// Declarations with a name or type error.
    poisoned: std::collections::BTreeSet<DefId>,
    early_kind_params: HashMap<(KindId, usize), (Ty, Option<RExpr>)>,
    assembly_components: HashMap<AssemblyDefId, Lazy<Option<ComponentId>>>,
}

#[derive(Default)]
struct Counters {
    params: usize,
    lets: usize,
    regions: usize,
    parts: usize,
    joints: usize,
    hardware: usize,
    assemblies: usize,
    requires: usize,
    stocks: usize,
    profiles: usize,
    kinds: usize,
    recipes: usize,
    components: usize,
    scenes: usize,
}

#[derive(Default)]
struct Arenas {
    params: Vec<Option<ParamDef>>,
    lets: Vec<Option<LetDef>>,
    regions: Vec<Option<RegionDef>>,
    parts: Vec<Option<PartDef>>,
    joints: Vec<Option<JointDef>>,
    hardware: Vec<Option<HardwareDef>>,
    assemblies: Vec<Option<AssemblyDef>>,
    requires: Vec<Option<RequireDef>>,
    stocks: Vec<Option<StockDef>>,
    profiles: Vec<Option<ProfileDef>>,
    kinds: Vec<Option<KindDef>>,
    recipes: Vec<Option<RecipeDef>>,
    components: Vec<Option<ComponentDef>>,
    scenes: Vec<Option<SceneDef>>,
    modules: Vec<ModuleDefs>,
}

fn store<T>(arena: &mut Vec<Option<T>>, index: usize, value: T) {
    if arena.len() <= index {
        arena.resize_with(index + 1, || None);
    }
    if let Some(slot) = arena.get_mut(index) {
        *slot = Some(value);
    }
}

fn next<T>(counter: &mut usize, make: impl Fn(usize) -> Option<T>) -> Option<T> {
    let id = make(*counter)?;
    *counter += 1;
    Some(id)
}

/// The kind of declaration an item keyword makes, for messages.
pub(crate) fn def_word(def: DefId) -> &'static str {
    match def {
        DefId::Param(_) => "param",
        DefId::Let(_) => "let",
        DefId::Region(_) => "region",
        DefId::Part(_) => "part",
        DefId::Joint(_) => "joint",
        DefId::Hardware(_) => "hardware",
        DefId::Assembly(_) => "assembly",
        DefId::Require(_) => "require",
        DefId::Stock(_) => "stock",
        DefId::Profile(_) => "profile",
        DefId::Kind(_) => "kind",
        DefId::Recipe(_) => "recipe",
        DefId::Component(_) => "component",
        DefId::Scene(_) => "scene",
    }
}

impl<'a> Resolver<'a> {
    fn new(modules: &'a ModuleSet) -> Self {
        let count = modules.modules.len();
        Resolver {
            modules,
            diagnostics: Vec::new(),
            origins: HashMap::new(),
            module_names: vec![BTreeMap::new(); count],
            component_names: Vec::new(),
            kind_names: HashMap::new(),
            imports: HashMap::new(),
            prelude: BTreeMap::new(),
            module_items: vec![Vec::new(); count],
            component_items: Vec::new(),
            counters: Counters::default(),
            out: Arenas::default(),
            types: HashMap::new(),
            kind_param_types: HashMap::new(),
            early_params: HashMap::new(),
            early_lets: HashMap::new(),
            files: BTreeMap::new(),
            broken: BTreeMap::new(),
            poisoned: std::collections::BTreeSet::new(),
            early_kind_params: HashMap::new(),
            assembly_components: HashMap::new(),
        }
    }

    // -----------------------------------------------------------------------
    // Diagnostics
    // -----------------------------------------------------------------------

    pub(crate) fn report(&mut self, diagnostic: Diagnostic) {
        self.diagnostics.push(diagnostic);
    }

    pub(crate) fn error(
        &mut self,
        code: Code,
        message: impl Into<String>,
        file: FileId,
        span: Span,
    ) {
        let help = match code {
            Code::UnitMismatch => {
                "lengths go with lengths and angles with angles; a plain number scales them, as in 2 * 19mm"
            }
            Code::TypeMismatch => "check what this name refers to and what the field needs",
            Code::UnknownName => "declare it in this module, or check its spelling",
            Code::DuplicateName => "rename one of them",
            Code::UnknownField => {
                "check the field's name against the declaration's table in the grammar"
            }
            Code::MissingField => "add the missing entry",
            _ => "see the grammar for what is allowed here",
        };
        self.report(Diagnostic::error(code, message, Label::new(file, span)).with_help(help));
    }

    pub(crate) fn error_help(
        &mut self,
        code: Code,
        message: impl Into<String>,
        file: FileId,
        span: Span,
        help: impl Into<String>,
    ) {
        self.report(Diagnostic::error(code, message, Label::new(file, span)).with_help(help));
    }

    fn finish(mut self) -> (Defs, Vec<Diagnostic>) {
        fn take<T>(arena: Vec<Option<T>>) -> Vec<T> {
            arena.into_iter().flatten().collect()
        }
        let out = std::mem::take(&mut self.out);
        let defs = Defs {
            modules: out.modules,
            prelude: self.modules.prelude.clone(),
            files: std::mem::take(&mut self.files),
            poisoned: std::mem::take(&mut self.poisoned),
            broken: std::mem::take(&mut self.broken),
            params: take(out.params),
            lets: take(out.lets),
            regions: take(out.regions),
            parts: take(out.parts),
            joints: take(out.joints),
            hardware: take(out.hardware),
            assemblies: take(out.assemblies),
            requires: take(out.requires),
            stocks: take(out.stocks),
            profiles: take(out.profiles),
            kinds: take(out.kinds),
            recipes: take(out.recipes),
            components: take(out.components),
            scenes: take(out.scenes),
        };
        let mut diagnostics = self.diagnostics;
        diagnostics.sort_by_key(|d| {
            d.primary()
                .map(|label| (label.file, label.span.start(), label.span.end()))
        });
        diagnostics.dedup();
        (defs, diagnostics)
    }

    // -----------------------------------------------------------------------
    // Pass 1: collect names
    // -----------------------------------------------------------------------

    fn allocate(&mut self, item: &ast::DesignItem) -> Option<DefId> {
        let c = &mut self.counters;
        Some(match item {
            ast::DesignItem::Param(_) => DefId::Param(next(&mut c.params, ParamDefId::from_index)?),
            ast::DesignItem::Let(_) => DefId::Let(next(&mut c.lets, LetDefId::from_index)?),
            ast::DesignItem::Region(_) => {
                DefId::Region(next(&mut c.regions, RegionDefId::from_index)?)
            }
            ast::DesignItem::Part(_) => DefId::Part(next(&mut c.parts, PartDefId::from_index)?),
            ast::DesignItem::Joint(_) => DefId::Joint(next(&mut c.joints, JointDefId::from_index)?),
            ast::DesignItem::Hardware(_) => {
                DefId::Hardware(next(&mut c.hardware, HardwareDefId::from_index)?)
            }
            ast::DesignItem::Assembly(_) => {
                DefId::Assembly(next(&mut c.assemblies, AssemblyDefId::from_index)?)
            }
            ast::DesignItem::Require(_) => {
                DefId::Require(next(&mut c.requires, RequireDefId::from_index)?)
            }
        })
    }

    fn design_syntax(item: &'a ast::DesignItem) -> Syntax<'a> {
        match item {
            ast::DesignItem::Param(p) => Syntax::Param(p),
            ast::DesignItem::Let(l) => Syntax::Let(l),
            ast::DesignItem::Region(r) => Syntax::Region(r),
            ast::DesignItem::Part(p) => Syntax::Part(p),
            ast::DesignItem::Joint(j) => Syntax::Joint(j),
            ast::DesignItem::Hardware(h) => Syntax::Hardware(h),
            ast::DesignItem::Assembly(a) => Syntax::Assembly(a),
            ast::DesignItem::Require(r) => Syntax::Require(r),
        }
    }

    fn enter(&mut self, table: TableRef, name: &ast::Ident, entry: Entry) {
        let names = match table {
            TableRef::Module(module) => self.module_names.get_mut(module.index()),
            TableRef::Component(component) => self.component_names.get_mut(component.index()),
        };
        let Some(names) = names else { return };
        if let Some(first) = names.get(name.as_str()).copied() {
            self.report(
                Diagnostic::error(
                    Code::DuplicateName,
                    format!("'{}' is declared twice", name.as_str()),
                    Label::new(entry.file, name.span()),
                )
                .with_label(Label::with_message(
                    first.file,
                    first.span,
                    "first declared here",
                ))
                .with_help("rename one of them; names are shared by every file of a module"),
            );
            return;
        }
        names.insert(name.as_str().to_owned(), entry);
    }

    fn collect(&mut self) {
        let modules = self.modules;
        for module in &modules.modules {
            let is_library = matches!(module.header, Some(ModuleHeader::Library { .. }));
            for file in &module.files {
                let mut file_imports = BTreeMap::new();
                for import in &file.imports {
                    file_imports.insert(import.name.clone(), import.module);
                }
                self.files.insert(
                    file.file,
                    crate::defs::FileScope {
                        module: Some(module.id),
                        imports: file_imports.clone(),
                    },
                );
                self.imports.insert(file.file, file_imports);
                for broken in &file.broken {
                    if let Some(name) = &broken.name {
                        self.broken
                            .entry(module.id)
                            .or_default()
                            .insert(name.clone());
                    }
                }
                let Some(ast) = &file.ast else { continue };
                for item in &ast.items {
                    self.collect_item(module.id, file.file, item, is_library);
                }
            }
        }
        self.collect_prelude();
        self.check_prelude_shadowing();
    }

    fn collect_item(
        &mut self,
        module: ModuleId,
        file: FileId,
        item: &'a ast::Item,
        is_library: bool,
    ) {
        let scope = Scope::Module(module);
        let private = item.private.is_some();
        let def = match &item.kind {
            ast::ItemKind::Project(_) | ast::ItemKind::Library(_) => return,
            ast::ItemKind::Kind(kind) => {
                let Some(id) = next(&mut self.counters.kinds, KindId::from_index) else {
                    return;
                };
                self.check_builtin_clash(&kind.name, file, "kind");
                (DefId::Kind(id), Syntax::Kind(kind))
            }
            ast::ItemKind::Recipe(recipe) => {
                let Some(id) = next(&mut self.counters.recipes, RecipeId::from_index) else {
                    return;
                };
                (DefId::Recipe(id), Syntax::Recipe(recipe))
            }
            ast::ItemKind::Stock(stock) => {
                let Some(id) = next(&mut self.counters.stocks, StockId::from_index) else {
                    return;
                };
                (DefId::Stock(id), Syntax::Stock(stock))
            }
            ast::ItemKind::Profile(profile) => {
                let Some(id) = next(&mut self.counters.profiles, ProfileId::from_index) else {
                    return;
                };
                (DefId::Profile(id), Syntax::Profile(profile))
            }
            ast::ItemKind::Component(component) => {
                let Some(id) = next(&mut self.counters.components, ComponentId::from_index) else {
                    return;
                };
                self.check_builtin_clash(&component.name, file, "component");
                self.component_names.push(BTreeMap::new());
                self.component_items.push(Vec::new());
                for inner in &component.items {
                    let Some(inner_def) = self.allocate(&inner.node) else {
                        continue;
                    };
                    let name = inner.node.name();
                    self.origins.insert(
                        inner_def,
                        Origin {
                            syntax: Self::design_syntax(&inner.node),
                            module,
                            file,
                            scope: Scope::Component(id),
                            docs: &inner.docs,
                            private: false,
                            span: inner.span,
                        },
                    );
                    if let Some(items) = self.component_items.get_mut(id.index()) {
                        items.push(inner_def);
                    }
                    self.enter(
                        TableRef::Component(id),
                        name,
                        Entry {
                            def: inner_def,
                            private: false,
                            file,
                            span: name.span(),
                        },
                    );
                }
                (DefId::Component(id), Syntax::Component(component))
            }
            ast::ItemKind::Scene(scene) => {
                let Some(id) = next(&mut self.counters.scenes, SceneDefId::from_index) else {
                    return;
                };
                if scene.name.as_str() == stave_schema::REST_SCENE {
                    self.error_help(
                        Code::DuplicateName,
                        format!("'{}' is declared twice", scene.name.as_str()),
                        file,
                        scene.name.span(),
                        "every project already has the scene 'rest' (as built); choose another name",
                    );
                }
                (DefId::Scene(id), Syntax::Scene(scene))
            }
            ast::ItemKind::Design(design) => {
                let Some(def) = self.allocate(design) else {
                    return;
                };
                (def, Self::design_syntax(design))
            }
        };
        let (def, syntax) = def;
        if is_library {
            let not_allowed = match def {
                DefId::Part(_) => Some("part"),
                DefId::Assembly(_) => Some("assembly"),
                DefId::Region(_) => Some("region"),
                DefId::Joint(_) => Some("joint"),
                DefId::Hardware(_) => Some("hardware"),
                DefId::Scene(_) => Some("scene"),
                _ => None,
            };
            if let Some(word) = not_allowed {
                let article = if word == "assembly" { "an" } else { "a" };
                self.error_help(
                    Code::NotInLibrary,
                    format!("a library can't contain {article} {word} at the top level"),
                    file,
                    item.span,
                    format!("put the {word} inside a component, or move it to a project"),
                );
            }
        }
        self.origins.insert(
            def,
            Origin {
                syntax,
                module,
                file,
                scope,
                docs: &item.docs,
                private,
                span: item.span,
            },
        );
        if let Some(items) = self.module_items.get_mut(module.index()) {
            items.push(def);
        }
        if let Some(name) = item.kind.name() {
            self.enter(
                TableRef::Module(module),
                name,
                Entry {
                    def,
                    private,
                    file,
                    span: name.span(),
                },
            );
        }
    }

    fn check_builtin_clash(&mut self, name: &ast::Ident, file: FileId, what: &str) {
        if Builtin::from_word(name.as_str()).is_some() {
            self.error_help(
                Code::DuplicateName,
                format!("'{}' is declared twice", name.as_str()),
                file,
                name.span(),
                format!(
                    "'{}' is a built-in function; a {what} can't take its name because both could be called",
                    name.as_str()
                ),
            );
        }
    }

    fn collect_prelude(&mut self) {
        let prelude = self.modules.prelude.clone();
        for module in prelude {
            let Some(names) = self.module_names.get(module.index()) else {
                continue;
            };
            let entries: Vec<(String, Entry)> = names
                .iter()
                .filter(|(_, entry)| !entry.private)
                .map(|(name, entry)| (name.clone(), *entry))
                .collect();
            for (name, entry) in entries {
                if let Some(first) = self.prelude.get(&name).copied() {
                    self.report(
                        Diagnostic::error(
                            Code::DuplicateName,
                            format!("'{name}' is declared twice"),
                            Label::new(entry.file, entry.span),
                        )
                        .with_label(Label::with_message(
                            first.file,
                            first.span,
                            "first declared here",
                        ))
                        .with_help(
                            "the standard modules may not declare the same name twice between them",
                        ),
                    );
                } else {
                    self.prelude.insert(name, entry);
                }
            }
        }
    }

    fn check_prelude_shadowing(&mut self) {
        let mut warnings = Vec::new();
        for (index, names) in self.module_names.iter().enumerate() {
            let is_std = self
                .modules
                .modules
                .get(index)
                .is_some_and(|m| self.modules.prelude.contains(&m.id));
            if is_std {
                continue;
            }
            for (name, entry) in names {
                let Some(prelude) = self.prelude.get(name) else {
                    continue;
                };
                let clash = matches!(
                    (entry.def, prelude.def),
                    (DefId::Kind(_), DefId::Kind(_)) | (DefId::Recipe(_), DefId::Recipe(_))
                );
                if clash {
                    warnings.push(
                        Diagnostic::warning(
                            Code::ShadowsPrelude,
                            format!("'{name}' hides the standard {} of the same name", def_word(entry.def)),
                            Label::new(entry.file, entry.span),
                        )
                        .with_help(format!(
                            "uses of '{name}' in this module now mean this one; rename it to keep the standard one"
                        )),
                    );
                }
            }
        }
        self.diagnostics.extend(warnings);
    }

    // -----------------------------------------------------------------------
    // Lookups
    // -----------------------------------------------------------------------

    /// Looks a bare name up as an item: the kind or component body, the
    /// module, then the prelude (namespace 1, without imports).
    pub(crate) fn lookup(&self, cx: &Cx, name: &str) -> Option<Symbol> {
        match cx.body {
            Body::Kind(kind) => {
                if let Some(symbol) = self.kind_names.get(&kind).and_then(|names| names.get(name)) {
                    return Some(*symbol);
                }
            }
            Body::Component(component) => {
                if let Some(entry) = self
                    .component_names
                    .get(component.index())
                    .and_then(|names| names.get(name))
                {
                    return Some(Symbol::Def(entry.def));
                }
            }
            Body::Module => {}
        }
        if let Some(entry) = self
            .module_names
            .get(cx.module.index())
            .and_then(|names| names.get(name))
        {
            return Some(Symbol::Def(entry.def));
        }
        self.prelude.get(name).map(|entry| Symbol::Def(entry.def))
    }

    /// The module a name in this file imports, if it is an import name.
    pub(crate) fn import(&self, cx: &Cx, name: &str) -> Option<ModuleId> {
        self.imports.get(&cx.file)?.get(name).copied()
    }

    /// A public item of a module, or `Err(true)` when it exists but is
    /// private.
    pub(crate) fn public_item(&self, module: ModuleId, name: &str) -> Result<DefId, bool> {
        match self
            .module_names
            .get(module.index())
            .and_then(|n| n.get(name))
        {
            Some(entry) if !entry.private => Ok(entry.def),
            Some(_) => Err(true),
            None => Err(false),
        }
    }

    /// The display name of a module (for messages).
    pub(crate) fn module_name(&self, module: ModuleId) -> String {
        self.modules
            .module(module)
            .map(|m| m.name.clone())
            .unwrap_or_default()
    }

    /// The name of a declaration.
    pub(crate) fn def_name(&self, def: DefId) -> String {
        match self.origins.get(&def).map(|o| o.syntax) {
            Some(Syntax::Param(p)) => p.name.to_string(),
            Some(Syntax::Let(l)) => l.name.to_string(),
            Some(Syntax::Region(r)) => r.name.to_string(),
            Some(Syntax::Part(p)) => p.name.to_string(),
            Some(Syntax::Joint(j)) => j.name.to_string(),
            Some(Syntax::Hardware(h)) => h.name.to_string(),
            Some(Syntax::Assembly(a)) => a.name.to_string(),
            Some(Syntax::Require(r)) => r.name.to_string(),
            Some(Syntax::Stock(s)) => s.name.to_string(),
            Some(Syntax::Profile(p)) => p.name.to_string(),
            Some(Syntax::Kind(k)) => k.name.to_string(),
            Some(Syntax::Recipe(r)) => r.name.to_string(),
            Some(Syntax::Component(c)) => c.name.to_string(),
            Some(Syntax::Scene(s)) => s.name.to_string(),
            None => String::new(),
        }
    }

    /// The labels of an array declaration, when it is labelled.
    pub(crate) fn array_labels(&self, def: DefId) -> Option<ArrayShape> {
        let array = match self.origins.get(&def)?.syntax {
            Syntax::Region(r) => r.array.as_ref(),
            Syntax::Part(p) => p.array.as_ref(),
            Syntax::Joint(j) => j.array.as_ref(),
            Syntax::Hardware(h) => h.array.as_ref(),
            Syntax::Assembly(a) => a.array.as_ref(),
            _ => None,
        }?;
        Some(match array {
            ast::ArraySpec::Labels(labels) => ArrayShape::Labels(
                labels
                    .as_slice()
                    .iter()
                    .map(|l| l.as_str().to_owned())
                    .collect(),
            ),
            ast::ArraySpec::Count(_) => ArrayShape::Count,
        })
    }

    /// The context an item's body is resolved in.
    fn cx_of(&self, def: DefId) -> Option<Cx> {
        let origin = self.origins.get(&def)?;
        let body = match origin.scope {
            Scope::Module(_) => Body::Module,
            Scope::Component(component) => Body::Component(component),
        };
        let array = self.array_labels(def).map(|_| def);
        Some(Cx {
            module: origin.module,
            file: origin.file,
            body,
            array,
        })
    }

    // -----------------------------------------------------------------------
    // Types of declarations
    // -----------------------------------------------------------------------

    /// The static type of what a symbol names.
    pub(crate) fn symbol_ty(&mut self, symbol: Symbol) -> Ty {
        match symbol {
            Symbol::Def(def) => self.def_ty(def),
            Symbol::Role(kind, index) => {
                let role = self
                    .kind_roles(kind)
                    .and_then(|roles| roles.get(index).copied());
                match role {
                    Some(RoleType::Region) => Ty::Region,
                    Some(RoleType::Part | RoleType::Parts) | None => Ty::Part,
                }
            }
            Symbol::KindParam(kind, index) => self.kind_param_ty(kind, index),
            Symbol::MadePart(..) => Ty::Part,
            Symbol::Mount(..) => Ty::Anchor,
        }
    }

    fn def_ty(&mut self, def: DefId) -> Ty {
        let wrap = |this: &Self, ty: Ty| {
            if this.array_labels(def).is_some() {
                Ty::Array(Box::new(ty))
            } else {
                ty
            }
        };
        match def {
            DefId::Param(id) => self.param_ty(id),
            DefId::Let(id) => self.let_ty(id),
            DefId::Region(_) => wrap(self, Ty::Region),
            DefId::Part(_) => wrap(self, Ty::Part),
            DefId::Joint(_) => wrap(self, Ty::Joint),
            DefId::Hardware(_) => wrap(self, Ty::Hardware),
            DefId::Assembly(id) => match self.assembly_component(id) {
                Some(component) => wrap(self, Ty::Assembly(component)),
                None => Ty::Unknown,
            },
            DefId::Require(_) => Ty::Bool,
            DefId::Stock(_) => Ty::Stock,
            DefId::Profile(_) => Ty::Profile,
            DefId::Kind(id) => match self.origins.get(&def).map(|o| o.syntax) {
                Some(Syntax::Kind(kind)) if kind.base.value == stave_lang::ast::BaseKind::Tool => {
                    Ty::Tool
                }
                Some(Syntax::Kind(kind)) => Ty::Kind(KindTy {
                    base: Some(kind.base.value),
                    kind: Some(id),
                }),
                _ => Ty::Unknown,
            },
            DefId::Recipe(_) => Ty::Recipe,
            DefId::Component(_) | DefId::Scene(_) => Ty::Unknown,
        }
    }

    fn kind_roles(&self, kind: KindId) -> Option<Vec<RoleType>> {
        let Some(Syntax::Kind(syntax)) = self.origins.get(&DefId::Kind(kind)).map(|o| o.syntax)
        else {
            return None;
        };
        Some(
            kinds::role_specs(syntax)
                .into_iter()
                .map(|(_, ty, _)| ty)
                .collect(),
        )
    }

    fn param_ty(&mut self, id: ParamDefId) -> Ty {
        if let Some((ty, _)) = self.early_params.get(&id) {
            return ty.clone();
        }
        let def = DefId::Param(id);
        match self.types.get(&def) {
            Some(Lazy::Done(ty)) => return ty.clone(),
            Some(Lazy::Busy) => return self.cycle(def),
            None => {}
        }
        let Some(Syntax::Param(param)) = self.origins.get(&def).map(|o| o.syntax) else {
            return Ty::Unknown;
        };
        let Some(cx) = self.cx_of(def) else {
            return Ty::Unknown;
        };
        self.types.insert(def, Lazy::Busy);
        let before = self.error_count();
        let (ty, default) = self.lower_param_spec(&cx, param);
        self.poison_if(def, before);
        self.types.insert(def, Lazy::Done(ty.clone()));
        self.early_params.insert(id, (ty.clone(), default));
        ty
    }

    fn let_ty(&mut self, id: LetDefId) -> Ty {
        if let Some(value) = self.early_lets.get(&id) {
            return value.ty.clone();
        }
        let def = DefId::Let(id);
        match self.types.get(&def) {
            Some(Lazy::Done(ty)) => return ty.clone(),
            Some(Lazy::Busy) => return self.cycle(def),
            None => {}
        }
        let Some(Syntax::Let(binding)) = self.origins.get(&def).map(|o| o.syntax) else {
            return Ty::Unknown;
        };
        let Some(cx) = self.cx_of(def) else {
            return Ty::Unknown;
        };
        self.types.insert(def, Lazy::Busy);
        let before = self.error_count();
        let value = self.value(&cx, &binding.value, &expr::Expect::any());
        self.poison_if(def, before);
        let ty = value.ty.clone();
        self.types.insert(def, Lazy::Done(ty.clone()));
        self.early_lets.insert(id, value);
        ty
    }

    fn kind_param_ty(&mut self, kind: KindId, index: usize) -> Ty {
        if let Some((ty, _)) = self.early_kind_params.get(&(kind, index)) {
            return ty.clone();
        }
        match self.kind_param_types.get(&(kind, index)) {
            Some(Lazy::Done(ty)) => return ty.clone(),
            Some(Lazy::Busy) => {
                return self.cycle(DefId::Kind(kind));
            }
            None => {}
        }
        let Some(origin) = self.origins.get(&DefId::Kind(kind)).copied() else {
            return Ty::Unknown;
        };
        let Syntax::Kind(syntax) = origin.syntax else {
            return Ty::Unknown;
        };
        let Some(param) = kinds::kind_params(syntax).get(index).copied() else {
            return Ty::Unknown;
        };
        self.ensure_kind_names(kind, syntax);
        let cx = Cx {
            module: origin.module,
            file: origin.file,
            body: Body::Kind(kind),
            array: None,
        };
        self.kind_param_types.insert((kind, index), Lazy::Busy);
        let before = self.error_count();
        let (ty, default) = self.lower_param_spec(&cx, param);
        self.poison_if(DefId::Kind(kind), before);
        self.kind_param_types
            .insert((kind, index), Lazy::Done(ty.clone()));
        self.early_kind_params
            .insert((kind, index), (ty.clone(), default));
        ty
    }

    fn cycle(&mut self, def: DefId) -> Ty {
        if let Some(origin) = self.origins.get(&def).copied() {
            let name = self.def_name(def);
            self.error_help(
                Code::TypeMismatch,
                format!("'{name}' is worked out from itself"),
                origin.file,
                origin.span,
                "a value can only refer to values declared without referring back to it",
            );
        }
        Ty::Unknown
    }

    /// The component an assembly instantiates (following mirrors).
    pub(crate) fn assembly_component(&mut self, id: AssemblyDefId) -> Option<ComponentId> {
        match self.assembly_components.get(&id) {
            Some(Lazy::Done(component)) => return *component,
            Some(Lazy::Busy) => return None,
            None => {}
        }
        let def = DefId::Assembly(id);
        let Some(Syntax::Assembly(syntax)) = self.origins.get(&def).map(|o| o.syntax) else {
            return None;
        };
        let cx = self.cx_of(def)?;
        self.assembly_components.insert(id, Lazy::Busy);
        let component = match &syntax.def {
            ast::AssemblyDef::Instance { component, .. } => match self.lookup_ref(&cx, component) {
                Some(Symbol::Def(DefId::Component(c))) => Some(c),
                _ => None,
            },
            ast::AssemblyDef::Mirror(mirror) => {
                // The source is an assembly name, possibly indexed.
                let root = mirror.source.as_path().map(|path| path.root);
                match root.and_then(|r| self.lookup(&cx, r.as_str())) {
                    Some(Symbol::Def(DefId::Assembly(source))) => self.assembly_component(source),
                    _ => None,
                }
            }
        };
        self.assembly_components.insert(id, Lazy::Done(component));
        component
    }

    /// Resolves a `ref` (`oak_19`, `casework.Drawer`) without reporting.
    pub(crate) fn lookup_ref(&self, cx: &Cx, reference: &ast::Ref) -> Option<Symbol> {
        match reference.rest.as_slice() {
            [] => self.lookup(cx, reference.root.as_str()),
            [member] => {
                if self.lookup(cx, reference.root.as_str()).is_none() {
                    let module = self.import(cx, reference.root.as_str())?;
                    self.public_item(module, member.as_str())
                        .ok()
                        .map(Symbol::Def)
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    pub(crate) fn ensure_kind_names(&mut self, kind: KindId, syntax: &ast::Kind) {
        if self.kind_names.contains_key(&kind) {
            return;
        }
        let mut names = BTreeMap::new();
        for (index, (name, _, _)) in kinds::role_specs(syntax).into_iter().enumerate() {
            names.entry(name).or_insert(Symbol::Role(kind, index));
        }
        for (index, param) in kinds::kind_params(syntax).into_iter().enumerate() {
            names
                .entry(param.name.as_str().to_owned())
                .or_insert(Symbol::KindParam(kind, index));
        }
        for (index, made) in kinds::made_parts(syntax).into_iter().enumerate() {
            names
                .entry(made.name.as_str().to_owned())
                .or_insert(Symbol::MadePart(kind, index));
        }
        if syntax.base.value.is_mounted_hardware() {
            // `mount`: the mount face, as an anchor (grammar, "Adjustment").
            names
                .entry("mount".to_owned())
                .or_insert(Symbol::Mount(kind));
        }
        self.kind_names.insert(kind, names);
    }

    // -----------------------------------------------------------------------
    // Pass 2: lower everything
    // -----------------------------------------------------------------------

    fn lower_all(&mut self) {
        let modules = self.modules;
        for module in &modules.modules {
            let (project, library) = self.lower_header(module);
            self.out.modules.push(ModuleDefs {
                id: module.id,
                name: module.name.clone(),
                folder: match &module.source {
                    ModuleSource::Local { folder } => Some(folder.clone()),
                    ModuleSource::Std { .. } => None,
                },
                items: self
                    .module_items
                    .get(module.id.index())
                    .cloned()
                    .unwrap_or_default(),
                project,
                library,
            });
        }
        let mut order: Vec<DefId> = self.origins.keys().copied().collect();
        order.sort();
        for def in order {
            self.lower_def(def);
        }
        // Components need their item lists.
        for (index, items) in self.component_items.clone().into_iter().enumerate() {
            let Some(id) = ComponentId::from_index(index) else {
                continue;
            };
            let def = DefId::Component(id);
            let Some(origin) = self.origins.get(&def).copied() else {
                continue;
            };
            let Syntax::Component(syntax) = origin.syntax else {
                continue;
            };
            let params = items
                .iter()
                .filter_map(|item| match item {
                    DefId::Param(p) => Some(*p),
                    _ => None,
                })
                .collect();
            let decl = self.decl(def, &syntax.name, origin);
            store(
                &mut self.out.components,
                index,
                ComponentDef {
                    decl,
                    items,
                    params,
                },
            );
        }
    }

    fn decl(&self, _def: DefId, name: &ast::Ident, origin: Origin<'_>) -> crate::defs::Decl {
        crate::defs::Decl {
            name: name.as_str().to_owned(),
            name_span: name.span(),
            file: origin.file,
            span: origin.span,
            scope: origin.scope,
            docs: origin.docs.to_vec(),
            private: origin.private,
        }
    }

    /// Reports an unknown name or field, suggesting the closest candidates
    /// when one is close enough, and `fallback` otherwise.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn unknown<S: AsRef<str>>(
        &mut self,
        code: Code,
        message: impl Into<String>,
        file: FileId,
        span: Span,
        word: &str,
        candidates: impl IntoIterator<Item = S>,
        fallback: impl Into<String>,
    ) {
        let help =
            crate::suggest::did_you_mean(word, candidates).unwrap_or_else(|| fallback.into());
        self.report(Diagnostic::error(code, message, Label::new(file, span)).with_help(help));
    }

    /// Every name a bare word could mean here: items in scope, import
    /// names and the prelude.
    pub(crate) fn names_in_scope(&self, cx: &Cx) -> Vec<String> {
        let mut names: Vec<String> = Vec::new();
        match cx.body {
            Body::Kind(kind) => {
                if let Some(table) = self.kind_names.get(&kind) {
                    names.extend(table.keys().cloned());
                }
            }
            Body::Component(component) => {
                if let Some(table) = self.component_names.get(component.index()) {
                    names.extend(table.keys().cloned());
                }
            }
            Body::Module => {}
        }
        if let Some(table) = self.module_names.get(cx.module.index()) {
            names.extend(table.keys().cloned());
        }
        if let Some(imports) = self.imports.get(&cx.file) {
            names.extend(imports.keys().cloned());
        }
        names.extend(self.prelude.keys().cloned());
        names
    }

    /// The public item names of a module.
    pub(crate) fn public_names(&self, module: ModuleId) -> Vec<String> {
        self.module_names
            .get(module.index())
            .map(|table| {
                table
                    .iter()
                    .filter(|(_, e)| !e.private)
                    .map(|(n, _)| n.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// A kind's name as `file` writes it (see [`Defs::kind_display`]).
    pub(crate) fn kind_display(&self, kind: KindId, file: FileId) -> String {
        let def = DefId::Kind(kind);
        let name = self.def_name(def);
        let Some(module) = self.origins.get(&def).map(|o| o.module) else {
            return name;
        };
        let own = self.files.get(&file).and_then(|f| f.module) == Some(module);
        if own || self.modules.prelude.contains(&module) {
            return name;
        }
        match self
            .imports
            .get(&file)
            .and_then(|i| i.iter().find(|(_, m)| **m == module))
        {
            Some((alias, _)) => format!("{alias}.{name}"),
            None => format!("{}.{name}", self.module_name(module)),
        }
    }

    /// The number of errors reported so far.
    pub(crate) fn error_count(&self) -> usize {
        self.diagnostics.iter().filter(|d| d.is_error()).count()
    }

    /// Marks a declaration poisoned if errors were reported since `before`.
    pub(crate) fn poison_if(&mut self, def: DefId, before: usize) {
        if self.error_count() > before {
            self.poisoned.insert(def);
        }
    }

    /// `true` if `name` is an item of this scope left out for a syntax
    /// error: its uses are skipped silently.
    pub(crate) fn is_broken(&self, cx: &Cx, name: &str) -> bool {
        let Some(names) = self.broken.get(&cx.module) else {
            return false;
        };
        let in_component = match cx.body {
            Body::Component(component) => {
                self.component_names.get(component.index()).is_some()
                    && names.contains(&format!(
                        "{}.{name}",
                        self.def_name(DefId::Component(component))
                    ))
            }
            Body::Module | Body::Kind(_) => false,
        };
        in_component || names.contains(name)
    }

    fn lower_def(&mut self, def: DefId) {
        let before = self.error_count();
        self.lower_def_inner(def);
        self.poison_if(def, before);
    }

    fn lower_def_inner(&mut self, def: DefId) {
        let Some(origin) = self.origins.get(&def).copied() else {
            return;
        };
        let Some(cx) = self.cx_of(def) else { return };
        match (def, origin.syntax) {
            (DefId::Param(id), Syntax::Param(syntax)) => {
                let (ty, default) = match self.early_params.remove(&id) {
                    Some(early) => early,
                    None => {
                        let ty = self.param_ty(id);
                        let default = self.early_params.remove(&id).and_then(|(_, d)| d);
                        (ty, default)
                    }
                };
                if default.is_none() && matches!(origin.scope, Scope::Module(_)) {
                    self.error_help(
                        Code::MissingField,
                        format!("param '{}' is missing a default", syntax.name.as_str()),
                        origin.file,
                        syntax.name.span(),
                        format!("give it a value: param {} = ...", syntax.name.as_str()),
                    );
                }
                let domain = syntax
                    .within
                    .as_ref()
                    .map(|domain| self.lower_domain(&cx, domain, &ty));
                let decl = self.decl(def, &syntax.name, origin);
                store(
                    &mut self.out.params,
                    id.index(),
                    ParamDef {
                        decl,
                        ty,
                        default,
                        domain,
                        locked: syntax.locked.is_some(),
                    },
                );
            }
            (DefId::Let(id), Syntax::Let(syntax)) => {
                let value = match self.early_lets.remove(&id) {
                    Some(value) => value,
                    None => {
                        self.let_ty(id);
                        self.early_lets
                            .remove(&id)
                            .unwrap_or_else(|| RExpr::error(syntax.value.span))
                    }
                };
                let decl = self.decl(def, &syntax.name, origin);
                store(&mut self.out.lets, id.index(), LetDef { decl, value });
            }
            (DefId::Region(id), Syntax::Region(syntax)) => {
                let region = self.lower_region(&cx, def, syntax, origin);
                store(&mut self.out.regions, id.index(), region);
            }
            (DefId::Part(id), Syntax::Part(syntax)) => {
                let part = self.lower_part(&cx, def, syntax, origin);
                store(&mut self.out.parts, id.index(), part);
            }
            (DefId::Joint(id), Syntax::Joint(syntax)) => {
                let joint = self.lower_joint(&cx, def, syntax, origin);
                store(&mut self.out.joints, id.index(), joint);
            }
            (DefId::Hardware(id), Syntax::Hardware(syntax)) => {
                let hardware = self.lower_hardware(&cx, def, syntax, origin);
                store(&mut self.out.hardware, id.index(), hardware);
            }
            (DefId::Assembly(id), Syntax::Assembly(syntax)) => {
                let assembly = self.lower_assembly(&cx, def, syntax, origin);
                store(&mut self.out.assemblies, id.index(), assembly);
            }
            (DefId::Require(id), Syntax::Require(syntax)) => {
                let require = self.lower_require(&cx, def, syntax, origin);
                store(&mut self.out.requires, id.index(), require);
            }
            (DefId::Stock(id), Syntax::Stock(syntax)) => {
                let stock = self.lower_stock(&cx, def, syntax, origin);
                store(&mut self.out.stocks, id.index(), stock);
            }
            (DefId::Profile(id), Syntax::Profile(syntax)) => {
                let profile = self.lower_profile(&cx, def, syntax, origin);
                store(&mut self.out.profiles, id.index(), profile);
            }
            (DefId::Kind(id), Syntax::Kind(syntax)) => {
                let kind = self.lower_kind(id, syntax, origin);
                store(&mut self.out.kinds, id.index(), kind);
            }
            (DefId::Recipe(id), Syntax::Recipe(syntax)) => {
                let recipe = self.lower_recipe(&cx, def, syntax, origin);
                store(&mut self.out.recipes, id.index(), recipe);
            }
            (DefId::Scene(id), Syntax::Scene(syntax)) => {
                let scene = self.lower_scene(&cx, def, syntax, origin);
                store(&mut self.out.scenes, id.index(), scene);
            }
            (DefId::Component(_), _) => {}
            _ => {}
        }
    }
}

/// Which name table an item goes into.
#[derive(Clone, Copy)]
enum TableRef {
    Module(ModuleId),
    Component(ComponentId),
}

/// The shape of an array declaration as far as resolution needs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ArrayShape {
    Labels(Vec<String>),
    Count,
}
