//! The query layer: [`AnalysisHost`] keeps what it worked out for one
//! [`Analysis`] and reuses it for the next, so a change to one file re-runs
//! only what depends on it (grammar, "Tooling and editors").
//!
//! The queries, what keeps each, and why this is not the `salsa` crate
//! are in the crate docs ("Incrementality").
//!
//! [`AnalysisHost::stats`] says which queries the last change ran.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::Arc;

use stave_lang::{FileId, GreenNode};
use stave_modules::{FileSource, LoadError, ModuleId, ModuleSet, ModuleSource, Parses};

use crate::analysis::Analysis;
use crate::index::{
    FileData, FileItems, FileResolution, Index, ModuleData, ModuleNames, SymbolId, TokensMemo,
    module_names,
};

/// Which queries the last [`AnalysisHost::analyse`] ran rather than
/// reused, each list in order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct QueryStats {
    /// The files parsed.
    pub parsed: Vec<FileId>,
    /// The files whose declarations were worked out.
    pub items: Vec<FileId>,
    /// The modules whose top-level names were worked out.
    pub module_names: Vec<ModuleId>,
    /// The files resolved.
    pub resolved: Vec<FileId>,
    /// The files whose coloured tokens were kept from the analysis before,
    /// worked out there and not needed again.
    pub tokens_kept: Vec<FileId>,
}

/// What one file gave last time.
#[derive(Debug, Clone)]
struct KeptFile {
    module: ModuleId,
    items: Arc<FileItems>,
    imports: Vec<(String, stave_lang::ast::Span, SymbolId)>,
    resolution: Arc<FileResolution>,
    tokens: TokensMemo,
}

/// The layout of a project: its modules, their files and the files' ids
/// and paths, the root and the prelude.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Layout {
    modules: Vec<ModuleLayout>,
    root: ModuleId,
    prelude: Vec<ModuleId>,
}

/// One module of a [`Layout`].
#[derive(Debug, Clone, PartialEq, Eq)]
struct ModuleLayout {
    id: ModuleId,
    name: String,
    standard: bool,
    /// Its files' ids and display paths.
    files: Vec<(FileId, String)>,
}

impl Layout {
    fn of(set: &ModuleSet) -> Layout {
        Layout {
            modules: set
                .modules
                .iter()
                .map(|module| ModuleLayout {
                    id: module.id,
                    name: module.name.clone(),
                    standard: matches!(module.source, ModuleSource::Std { .. }),
                    files: module
                        .files
                        .iter()
                        .map(|file| (file.file, file.path.clone()))
                        .collect(),
                })
                .collect(),
            root: set.root,
            prelude: set.prelude.clone(),
        }
    }
}

/// Keeps the work of one [`Analysis`] for the next: hold one per project
/// and call [`AnalysisHost::analyse`] after each change instead of
/// [`Analysis::load`]. The analysis it gives is the same as
/// [`Analysis::load`]'s; only the work differs.
///
/// ```
/// use std::path::Path;
/// use stave_ide::{AnalysisHost, Document, Overlay};
///
/// let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/clothing_cabinet");
/// let folder = folder.canonicalize().unwrap();
/// let mut host = AnalysisHost::new();
/// let mut overlay = Overlay::new();
/// host.analyse(&folder, &overlay).unwrap();
///
/// // Type a comment into one file: only it is parsed and resolved again.
/// let cabinet = folder.join("cabinet.stave");
/// let text = std::fs::read_to_string(&cabinet).unwrap();
/// overlay.insert(&cabinet, format!("{text}// a note\n"));
/// let analysis = host.analyse(&folder, &overlay).unwrap();
/// let file = analysis.file(&Document::File(cabinet)).unwrap();
/// assert_eq!(host.stats().parsed, [file]);
/// assert_eq!(host.stats().resolved, [file]);
/// ```
#[derive(Debug, Default)]
pub struct AnalysisHost {
    parses: Parses,
    layout: Option<Layout>,
    files: BTreeMap<FileId, KeptFile>,
    modules: BTreeMap<ModuleId, KeptModule>,
    instance_of: BTreeMap<SymbolId, SymbolId>,
    stats: QueryStats,
}

/// What one module gave last time: its files' items, its names and its
/// own symbol's id.
#[derive(Debug, Clone)]
struct KeptModule {
    items: Vec<Arc<FileItems>>,
    names: Arc<ModuleNames>,
    symbol: Option<SymbolId>,
}

impl AnalysisHost {
    /// A host that has worked nothing out yet.
    pub fn new() -> AnalysisHost {
        AnalysisHost::default()
    }

    /// Loads and indexes a project folder, reusing whatever the last call
    /// worked out that the change did not touch.
    ///
    /// # Errors
    ///
    /// When the project folder itself can't be loaded ([`LoadError`]).
    pub fn analyse(
        &mut self,
        folder: &Path,
        files: &dyn FileSource,
    ) -> Result<Analysis, LoadError> {
        let mut set = ModuleSet::load_cached(folder, files, &mut self.parses)?;
        // Indexing reads the lossless trees, not the typed ones.
        self.parses.reclaim(&mut set);
        let parsed = self.parses.parsed().to_vec();
        let parses = &self.parses;
        let index = build(
            folder,
            set,
            |file, _| parses.green(file).cloned(),
            &parsed,
            &mut Kept {
                layout: &mut self.layout,
                files: &mut self.files,
                modules: &mut self.modules,
                instance_of: &mut self.instance_of,
            },
            &mut self.stats,
        );
        self.stats.parsed = parsed;
        Ok(Analysis::from_index(index))
    }

    /// Which queries the last [`AnalysisHost::analyse`] ran.
    pub fn stats(&self) -> &QueryStats {
        &self.stats
    }
}

/// What a host keeps, borrowed for one build.
struct Kept<'a> {
    layout: &'a mut Option<Layout>,
    files: &'a mut BTreeMap<FileId, KeptFile>,
    modules: &'a mut BTreeMap<ModuleId, KeptModule>,
    instance_of: &'a mut BTreeMap<SymbolId, SymbolId>,
}

/// What changed since the last build.
#[derive(Debug, Default)]
struct Changes {
    /// The files whose items were worked out again.
    files: BTreeSet<FileId>,
    /// The modules whose names are not what they were.
    modules: BTreeSet<ModuleId>,
    /// Some assembly is an instance of another component, or of none.
    instances: bool,
    /// Some module's own symbol moved, came or went.
    module_symbols: bool,
}

impl Changes {
    /// `true` when nothing a resolution read can have changed.
    fn leaves(&self, resolution: &FileResolution) -> bool {
        let depends = &resolution.depends;
        depends.files.is_disjoint(&self.files)
            && depends.modules.is_disjoint(&self.modules)
            && !(depends.instances && self.instances)
            && !(depends.module_symbols && self.module_symbols)
    }
}

/// Indexes modules already loaded, with nothing kept: every query runs.
pub(crate) fn build_fresh(folder: &Path, set: ModuleSet) -> Index {
    let mut layout = None;
    let mut files = BTreeMap::new();
    let mut modules = BTreeMap::new();
    let mut instance_of = BTreeMap::new();
    build(
        folder,
        set,
        |file, text| Some(stave_lang::parse_file(file, text).green),
        &[],
        &mut Kept {
            layout: &mut layout,
            files: &mut files,
            modules: &mut modules,
            instance_of: &mut instance_of,
        },
        &mut QueryStats::default(),
    )
}

/// Puts an index together from a loaded module set, reusing what `kept`
/// holds where its inputs are unchanged, and records in `kept` what the
/// next build may reuse.
fn build(
    folder: &Path,
    set: ModuleSet,
    green_of: impl Fn(FileId, &str) -> Option<GreenNode>,
    parsed: &[FileId],
    kept: &mut Kept<'_>,
    stats: &mut QueryStats,
) -> Index {
    *stats = QueryStats::default();
    let layout = Layout::of(&set);
    if kept.layout.as_ref() != Some(&layout) {
        kept.files.clear();
        kept.modules.clear();
        kept.instance_of.clear();
    }
    // What changed since the last build, for telling quickly that a kept
    // resolution's reads still hold.
    let mut changed = Changes::default();
    let reparsed = |file: FileId| parsed.contains(&file);

    // Items, file by file; then each module's names and own symbol.
    let mut files: BTreeMap<FileId, FileData> = BTreeMap::new();
    let mut modules: BTreeMap<ModuleId, ModuleData> = BTreeMap::new();
    for module in &set.modules {
        let mut module_items: Vec<Arc<FileItems>> = Vec::with_capacity(module.files.len());
        for file in &module.files {
            let Some(text) = set.sources.get(file.file).map(|source| source.text()) else {
                continue;
            };
            let Some(green) = green_of(file.file, text) else {
                continue;
            };
            let items = match kept.files.get(&file.file) {
                Some(before) if before.module == module.id && !reparsed(file.file) => {
                    Arc::clone(&before.items)
                }
                _ => {
                    stats.items.push(file.file);
                    changed.files.insert(file.file);
                    Arc::new(FileItems::new(
                        file.file,
                        module.id,
                        &stave_lang::SyntaxNode::new_root(green.clone()),
                    ))
                }
            };
            module_items.push(Arc::clone(&items));
            files.insert(
                file.file,
                FileData {
                    path: file.path.clone(),
                    module: Some(module.id),
                    green,
                    items,
                    imports: Vec::new(),
                    resolution: Arc::default(),
                    tokens: Arc::default(),
                },
            );
        }
        let before = kept.modules.get(&module.id);
        let names = match before {
            Some(before)
                if before.items.len() == module_items.len()
                    && before
                        .items
                        .iter()
                        .zip(&module_items)
                        .all(|(old, new)| Arc::ptr_eq(old, new)) =>
            {
                Arc::clone(&before.names)
            }
            _ => {
                stats.module_names.push(module.id);
                let names = module_names(module_items.iter().map(|items| items.as_ref()));
                // Unchanged names keep their old value (an early cut-off).
                match before {
                    Some(before) if *before.names == names => Arc::clone(&before.names),
                    _ => {
                        changed.modules.insert(module.id);
                        Arc::new(names)
                    }
                }
            }
        };
        let symbol = module_items
            .iter()
            .find(|items| items.has_header())
            .and_then(|items| items.module_symbol(&module.name));
        let symbol_id = symbol.as_ref().map(|symbol| symbol.id);
        if before.is_none_or(|before| before.symbol != symbol_id) {
            changed.module_symbols = true;
        }
        kept.modules.insert(
            module.id,
            KeptModule {
                items: module_items,
                names: Arc::clone(&names),
                symbol: symbol_id,
            },
        );
        modules.insert(
            module.id,
            ModuleData {
                symbol,
                files: module.files.iter().map(|file| file.file).collect(),
                standard: matches!(module.source, ModuleSource::Std { .. }),
                names,
            },
        );
    }

    // Import names, which the loader already resolved.
    for module in &set.modules {
        for file in &module.files {
            let imports = file
                .imports
                .iter()
                .filter_map(|import| {
                    let symbol = modules.get(&import.module)?.symbol.as_ref()?.id;
                    Some((import.name.clone(), import.name_span, symbol))
                })
                .collect();
            if let Some(data) = files.get_mut(&file.file) {
                data.imports = imports;
            }
        }
    }

    let mut index = Index::new(
        folder.to_path_buf(),
        set.sources,
        files,
        modules,
        set.root,
        set.prelude,
        set.diagnostics,
    );
    index.link_assemblies();
    changed.instances = *kept.instance_of != index.instance_of;

    // Resolution: keep a file's when its tree and imports are the same and
    // every lookup it made still finds the same thing.
    let mut resolutions: Vec<(FileId, Arc<FileResolution>, TokensMemo)> =
        Vec::with_capacity(index.files.len());
    for (file, data) in &index.files {
        let reusable = kept.files.get(file).filter(|before| {
            Arc::ptr_eq(&before.items, &data.items)
                && before.imports == data.imports
                && (changed.leaves(&before.resolution)
                    || before
                        .resolution
                        .reads
                        .iter()
                        .all(|(read, seen)| index.still_holds(read, seen)))
        });
        match reusable {
            Some(before) => {
                if before.tokens.get().is_some() {
                    stats.tokens_kept.push(*file);
                }
                resolutions.push((
                    *file,
                    Arc::clone(&before.resolution),
                    Arc::clone(&before.tokens),
                ));
            }
            None => {
                stats.resolved.push(*file);
                resolutions.push((*file, Arc::new(index.resolve_file(*file)), Arc::default()));
            }
        }
    }
    for (file, resolution, tokens) in resolutions {
        if let Some(data) = index.files.get_mut(&file) {
            data.resolution = resolution;
            data.tokens = tokens;
        }
    }

    // Keep this build's parts for the next.
    kept.files.clear();
    for (file, data) in &index.files {
        let Some(module) = data.module else {
            continue;
        };
        kept.files.insert(
            *file,
            KeptFile {
                module,
                items: Arc::clone(&data.items),
                imports: data.imports.clone(),
                resolution: Arc::clone(&data.resolution),
                tokens: Arc::clone(&data.tokens),
            },
        );
    }
    kept.modules
        .retain(|module, _| index.modules.contains_key(module));
    kept.instance_of.clone_from(&index.instance_of);
    *kept.layout = Some(layout);
    index
}
