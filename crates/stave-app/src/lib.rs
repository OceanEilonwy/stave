//! The headless Stave application: open a project folder and run the
//! pipeline. This is the only entry point the command line and the viewer
//! use.
//!
//! ```text
//! folder ──stave-modules──▶ ModuleSet ──stave-model──▶ Defs ──▶ Model ──stave-solve──▶ SolvedDesign
//! ```
//!
//! Checking runs in layers (grammar, "Checks and require"): syntax and
//! module errors stop before names are resolved, name and type errors stop
//! before anything is placed. Within a layer every problem is reported.
//! Diagnostics come back sorted by file and position, so the output is the
//! same on every run.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::path::Path;

use stave_lang::{Diagnostic, SourceMap};
use stave_model::Model;
use stave_model::solved::SolvedDesign;
use stave_modules::{FileSource, ModuleSet};

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
    /// How many `.stave` files the folder itself holds.
    pub root_files: usize,
}

impl Opened {
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
    let has_errors = |diagnostics: &[Diagnostic]| diagnostics.iter().any(Diagnostic::is_error);
    let finish = |sources: SourceMap, model, design, mut diagnostics: Vec<Diagnostic>| {
        sort(&mut diagnostics);
        Ok(Opened {
            sources,
            model,
            design,
            diagnostics,
            root_files,
        })
    };
    if has_errors(&diagnostics) {
        return finish(modules.sources, None, None, diagnostics);
    }
    let (defs, resolve_diagnostics) = stave_model::resolve(&modules);
    diagnostics.extend(resolve_diagnostics);
    if has_errors(&diagnostics) {
        return finish(modules.sources, None, None, diagnostics);
    }
    let (model, instance_diagnostics) = stave_model::instance::instantiate(defs, modules.root);
    diagnostics.extend(instance_diagnostics);
    if has_errors(&diagnostics) {
        return finish(modules.sources, Some(model), None, diagnostics);
    }
    let (design, solve_diagnostics) = stave_solve::solve(&model);
    diagnostics.extend(solve_diagnostics);
    finish(modules.sources, Some(model), design, diagnostics)
}
