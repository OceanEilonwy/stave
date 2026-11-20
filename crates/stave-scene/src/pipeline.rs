//! A project folder through the whole pipeline to the scene a viewer
//! draws, with nothing a browser can't run: the files come through a
//! [`FileSource`] (the disk, or memory), and nothing here watches files,
//! translates words or calls the solver for smart edits. `stave-app` opens
//! projects through this module; the browser viewer (`stave-web`) does too.
//!
//! ```text
//! folder ──stave-modules──▶ ModuleSet ──stave-model──▶ Defs ──▶ Model ──stave-solve──▶ SolvedDesign
//!        ──stave-check──▶ fit, overlap and scene diagnostics
//!        ──▶ SceneGraph (every part as cut)
//! ```
//!
//! Checking runs in layers (grammar, "Checks and require"), per item: an
//! item with a syntax error is left out, an item with a name or type error
//! is "poisoned", and anything that depends on either is skipped without
//! further errors, while everything independent still resolves, is placed
//! and reports its own problems. Only a folder without its project or
//! library item, or modules that import each other, stop the pipeline
//! early. A solved design is produced only when nothing failed up to
//! placement; the fit checks then run on it, and it is kept whatever they
//! find. Diagnostics come back sorted by file and position, so the output
//! is the same on every run.

use std::path::Path;

use stave_check::{Contact, ContactItem};
use stave_lang::{Diagnostic, SourceMap};
use stave_model::solved::SolvedDesign;
use stave_model::{Defs, Model};
use stave_modules::{FileSource, LoadError, ModuleSet};

use crate::{BuildOptions, Extent, Highlight, Item, MeshCache, SceneGraph};

/// A project folder after the pipeline ran.
#[derive(Debug)]
pub struct Opened {
    /// Every source file read, the standard library included.
    pub sources: SourceMap,
    /// The resolved model, when names and types checked.
    pub model: Option<Model>,
    /// The solved design, when everything placed without errors. `None`
    /// for a library folder, which has nothing to place.
    pub design: Option<SolvedDesign>,
    /// Every diagnostic, sorted.
    pub diagnostics: Vec<Diagnostic>,
    /// What the fit and motion diagnostics are about in the design: the
    /// items, the shared wood, the pose at contact.
    pub contacts: Vec<(Diagnostic, Contact)>,
    /// How many `.stave` files the folder itself holds.
    pub root_files: usize,
}

impl Opened {
    /// What a diagnostic is about in the design, when the checks know.
    pub fn contact(&self, diagnostic: &Diagnostic) -> Option<&Contact> {
        self.contacts
            .iter()
            .find(|(d, _)| d == diagnostic)
            .map(|(_, contact)| contact)
    }

    /// The number of errors.
    pub fn error_count(&self) -> usize {
        self.diagnostics.iter().filter(|d| d.is_error()).count()
    }
}

/// Opens a project (or library) folder from disk and runs the pipeline.
pub fn open(folder: &Path) -> Result<Opened, LoadError> {
    Ok(run(ModuleSet::load(folder)?, stave_model::resolve))
}

/// Opens a folder through a [`FileSource`] (for editors, tests and the
/// browser).
pub fn open_with(folder: &Path, files: &dyn FileSource) -> Result<Opened, LoadError> {
    Ok(run(
        ModuleSet::load_with(folder, files)?,
        stave_model::resolve,
    ))
}

/// Sorts diagnostics by file and position (then code and message) and
/// drops copies, so the output is the same on every run.
pub fn sort_diagnostics(diagnostics: &mut Vec<Diagnostic>) {
    diagnostics.sort_by(|a, b| {
        let key = |d: &Diagnostic| {
            d.primary()
                .map(|label| (label.file, label.span.start(), label.span.end()))
        };
        key(a)
            .cmp(&key(b))
            .then_with(|| a.code.cmp(&b.code))
            .then_with(|| a.message.cmp(&b.message))
    });
    diagnostics.dedup();
}

/// Runs the pipeline on loaded modules, resolving them with `resolve`
/// (which may reuse libraries resolved before).
pub fn run(
    modules: ModuleSet,
    resolve: impl FnOnce(&ModuleSet) -> (Defs, Vec<Diagnostic>),
) -> Opened {
    let root_files = modules
        .module(modules.root)
        .map_or(0, |module| module.files.len());
    let finish = |sources: SourceMap,
                  model,
                  design,
                  mut diagnostics: Vec<Diagnostic>,
                  contacts: Vec<(Diagnostic, Contact)>| {
        sort_diagnostics(&mut diagnostics);
        Opened {
            sources,
            model,
            design,
            diagnostics,
            contacts,
            root_files,
        }
    };
    // Only problems that make the whole project meaningless stop here: a
    // folder without its project or library item, or modules importing
    // each other. Every other error poisons just the item it is in.
    let root_has_header = modules
        .module(modules.root)
        .is_some_and(|module| module.header.is_some());
    let cycle = modules
        .diagnostics
        .iter()
        .any(|d| d.code == stave_lang::Code::ImportCycle);
    if !root_has_header || cycle {
        return finish(modules.sources, None, None, modules.diagnostics, Vec::new());
    }
    // Resolving reads the loader's diagnostics (to know whether errors came
    // before it), so they are moved out only afterwards.
    let (defs, resolve_diagnostics) = resolve(&modules);
    let mut diagnostics = modules.diagnostics;
    diagnostics.extend(resolve_diagnostics);
    let (model, instance_diagnostics) = stave_model::instance::instantiate(defs, modules.root);
    diagnostics.extend(instance_diagnostics);
    let (design, solve_diagnostics) = stave_solve::solve(&model);
    diagnostics.extend(solve_diagnostics);
    // Fit and geometry (layer 4) and scene poses run only when nothing
    // before them found an error, so one mistake is one error. The design
    // is kept even when they find some, so a viewer can show the problem.
    let mut contacts = Vec::new();
    if let Some(design) = &design
        && diagnostics.iter().all(|d| !d.is_error())
    {
        for (diagnostic, contact) in stave_check::check_with_contacts(&model, design) {
            if let Some(contact) = contact {
                contacts.push((diagnostic.clone(), contact));
            }
            diagnostics.push(diagnostic);
        }
    }
    finish(modules.sources, Some(model), design, diagnostics, contacts)
}

/// The scene graph a viewer draws.
///
/// Every part is drawn as cut, with its cuts made: tenons sit inside
/// their mortises, shelves in their dados, and the joinery shows wherever
/// it is visible. Parts that fit meet face to face without overlapping, so
/// nothing flickers.
pub fn scene(design: &SolvedDesign) -> SceneGraph {
    scene_with(design, &MeshCache::new())
}

/// [`scene`], taking the part meshes whose cuts haven't changed from
/// `meshes` (the last build that used it) and leaving this build's in it.
pub fn scene_with(design: &SolvedDesign, meshes: &MeshCache) -> SceneGraph {
    SceneGraph::build_with(design, viewer_options(), meshes)
}

/// How a viewer's scene is built: every part as cut.
pub fn viewer_options() -> BuildOptions {
    BuildOptions {
        extent: Extent::Cut,
    }
}

/// The scene item a problem's item is.
pub fn contact_item(item: ContactItem) -> Item {
    match item {
        ContactItem::Part(id) => Item::Part(id),
        ContactItem::Hardware(id) => Item::Hardware(id),
        ContactItem::Member(member) => Item::Member(member),
    }
}

/// What to tint and draw in red for a problem: every node of the items it
/// is about, and the shared wood (measured in the pose at contact).
pub fn highlight(graph: &SceneGraph, contact: &Contact) -> Highlight {
    let nodes = graph
        .node_ids()
        .filter(|id| {
            graph.node(*id).is_some_and(|node| {
                contact
                    .items
                    .iter()
                    .any(|item| contact_item(*item) == node.item)
            })
        })
        .collect();
    let volumes = contact
        .regions
        .iter()
        .filter_map(|region| {
            stave_geom::Aabb::around([
                stave_geom::render::point(region.min()),
                stave_geom::render::point(region.max()),
            ])
        })
        .collect();
    Highlight { nodes, volumes }
}
