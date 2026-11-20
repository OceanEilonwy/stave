//! The headless Stave application: open a project folder and run the
//! pipeline. This is the only entry point the command line and the window
//! use. The pipeline itself ([`Opened`], [`open`], [`open_with`]) lives in
//! `stave_scene::pipeline`, so the browser viewer can run it too; it is
//! re-exported here.
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

pub mod candidates;
pub mod change;
pub mod check;
pub mod choices;
pub mod edit;
pub mod editing;
pub mod empty;
pub mod export_view;
pub mod finder;
pub mod focus;
pub mod format;
pub mod grip_values;
pub mod gripping;
pub mod grips;
pub mod hold;
pub mod i18n;
pub mod inspect;
pub mod item_offer;
pub mod link;
pub mod naming;
pub mod option_fits;
pub mod param_kind;
pub mod part_joints;
pub mod problems;
pub mod remove;
pub mod repeat;
pub mod save_failure;
pub mod scratch;
pub mod script;
pub mod session;
pub mod solve_edit;
pub mod spaces;
pub mod stops;
pub mod view;

use stave_lang::Diagnostic;
use stave_model::{Defs, ResolvedLibraries};
use stave_modules::{FileSource, ModuleSet};

pub use stave_check::{Contact, ContactItem, ContactMotion};
pub use stave_modules::LoadError;

pub use stave_scene::pipeline::{Opened, open, open_with};

/// A project opened once, kept so that changed copies of it open quickly:
/// the parsed files, and the standard modules and imported libraries
/// resolved. [`Prepared::reopen`] parses and resolves only what changed.
#[derive(Debug, Clone)]
pub struct Prepared {
    modules: ModuleSet,
    libraries: Option<ResolvedLibraries>,
}

/// [`open_with`], also keeping what a [`Prepared::reopen`] reuses.
pub fn prepare(folder: &Path, files: &dyn FileSource) -> Result<(Opened, Prepared), LoadError> {
    let modules = ModuleSet::load_with(folder, files)?;
    let kept = modules.clone();
    let mut libraries = None;
    let opened = run(modules, |modules| {
        let (defs, diagnostics, kept) = stave_model::resolve_keeping(modules);
        libraries = kept;
        (defs, diagnostics)
    })?;
    Ok((
        opened,
        Prepared {
            modules: kept,
            libraries,
        },
    ))
}

impl Prepared {
    /// Opens a changed copy of the prepared project, read through `files`
    /// (the project's own files changed, as in an editor's scratch copy):
    /// files whose text is unchanged are not parsed again, and when the
    /// libraries are unchanged only the project is resolved again. The
    /// result is the same as [`open_with`]'s.
    pub fn reopen(&self, folder: &Path, files: &dyn FileSource) -> Result<Opened, LoadError> {
        let modules = self.modules.reload_with(folder, files)?;
        run(modules, |modules| {
            self.libraries
                .as_ref()
                .and_then(|libraries| stave_model::resolve_reusing(modules, libraries))
                .unwrap_or_else(|| stave_model::resolve(modules))
        })
    }
}

/// Sorts diagnostics as [`open`] does.
fn sort(diagnostics: &mut Vec<Diagnostic>) {
    stave_scene::pipeline::sort_diagnostics(diagnostics);
}

/// Runs the pipeline on loaded modules ([`stave_scene::pipeline::run`]).
fn run(
    modules: ModuleSet,
    resolve: impl FnOnce(&ModuleSet) -> (Defs, Vec<Diagnostic>),
) -> Result<Opened, LoadError> {
    Ok(stave_scene::pipeline::run(modules, resolve))
}
