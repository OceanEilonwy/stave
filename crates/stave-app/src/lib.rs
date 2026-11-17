//! The headless Stave application: open a project folder and run the
//! pipeline. This is the only entry point the command line and the viewer
//! use.
//!
//! ```text
//! folder ──stave-modules──▶ ModuleSet ──stave-model──▶ Defs ──▶ Model ──stave-solve──▶ SolvedDesign
//!        ──stave-check──▶ fit, overlap and scene diagnostics
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
//! find.
//! Diagnostics come back sorted by file and position, so the output is the
//! same on every run.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::path::Path;

pub mod apart;
pub mod check;
pub mod empty;
pub mod focus;
pub mod inspect;
pub mod link;
pub mod problems;
pub mod session;
pub mod view;

use stave_lang::{Diagnostic, SourceMap};
use stave_model::Model;
use stave_model::solved::SolvedDesign;
use stave_modules::{FileSource, ModuleSet};

pub use stave_check::{Contact, ContactItem, ContactMotion};
pub use stave_modules::LoadError;

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
    run(ModuleSet::load(folder)?)
}

/// Opens a folder through a [`FileSource`] (for editors and tests).
pub fn open_with(folder: &Path, files: &dyn FileSource) -> Result<Opened, LoadError> {
    run(ModuleSet::load_with(folder, files)?)
}

fn sort(diagnostics: &mut Vec<Diagnostic>) {
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

fn run(modules: ModuleSet) -> Result<Opened, LoadError> {
    let root_files = modules
        .module(modules.root)
        .map_or(0, |module| module.files.len());
    let mut diagnostics = modules.diagnostics.clone();
    let finish = |sources: SourceMap,
                  model,
                  design,
                  mut diagnostics: Vec<Diagnostic>,
                  contacts: Vec<(Diagnostic, Contact)>| {
        sort(&mut diagnostics);
        Ok(Opened {
            sources,
            model,
            design,
            diagnostics,
            contacts,
            root_files,
        })
    };
    // Only problems that make the whole project meaningless stop here: a
    // folder without its project or library item, or modules importing
    // each other. Every other error poisons just the item it is in.
    let root_has_header = modules
        .module(modules.root)
        .is_some_and(|module| module.header.is_some());
    let cycle = diagnostics
        .iter()
        .any(|d| d.code == stave_lang::Code::ImportCycle);
    if !root_has_header || cycle {
        return finish(modules.sources, None, None, diagnostics, Vec::new());
    }
    let (defs, resolve_diagnostics) = stave_model::resolve(&modules);
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
