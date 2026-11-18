//! Module loading: the stage between parsing and name resolution.
//!
//! Given a project folder, [`ModuleSet::load`] discovers and parses every
//! module the project needs:
//!
//! - the **root module**: every `.stave` file directly in the folder
//!   (subfolders are separate modules);
//! - every module it **imports**, transitively. Relative sources
//!   (`"./casework"`, `"../shared"`) are folders relative to the importing
//!   file; `std:<name>` is a standard module. Remote sources
//!   (`github:`, `https:`, any other `scheme:`) are `error[not-supported]` in
//!   this version and are not loaded;
//! - the three **standard modules** (`std:joinery`, `std:hardware`,
//!   `std:tools`), embedded in the binary at compile time. They are always
//!   loaded, because they are the prelude.
//!
//! The result, a [`ModuleSet`], holds every parsed file and every diagnostic
//! this stage owns: parse errors, unknown and unsupported import sources,
//! module names that are keywords or not words, duplicate import names,
//! import cycles, and a missing or duplicated `project`/`library` header.
//! Item names are **not** resolved here; `stave-model` does that.
//!
//! # Module identity
//!
//! A local module is identified by its canonical folder
//! ([`FileSource::canonical`]), so a folder imported from several files or
//! modules is loaded once. A standard module is identified by its name. When
//! the root folder's `.stave` files are byte-identical to a standard module's
//! (same file names, same texts), the root *is* that standard module, so
//! `stave check std/joinery` loads it once.
//!
//! # Display paths
//!
//! Root files display as the root folder exactly as given joined with the
//! file name (`tests/broken/missing_value/project.stave`). An imported folder
//! is the importing file's folder joined with the source, normalised
//! lexically ([`normalize`]): `examples/japanese_joinery/library.stave`.
//! Standard files display as `std/<module>/<file>`.
//!
//! # Order: deterministic, run to run
//!
//! Modules (and so [`ModuleId`]s) and files (and so [`FileId`]s) are numbered
//! in the order they are first reached by a depth-first walk:
//!
//! 1. the root module, its files sorted by file name;
//! 2. each imported module when its first import is reached: the importing
//!    module's files in file-name order, each file's imports in source
//!    order, descending into a newly found module before the next import;
//! 3. the prelude's standard modules not reached by an import, in
//!    [`PRELUDE_MODULES`] order (each followed by whatever it imports). A
//!    standard module outside the prelude (`std:casework`) is loaded only
//!    when something imports it.
//!
//! Within a module, files are sorted by file name. Diagnostics are sorted by
//! (file id, span start, span end) with a stable sort, so ties keep the
//! order they were found in.

#![forbid(unsafe_code)]

mod loader;
mod source;
mod std_library;

use std::io;
use std::path::{Path, PathBuf};

use stave_lang::ast::{File, Span};
use stave_lang::diagnostics::SourceMapError;
use stave_lang::{Diagnostic, FileId, SourceMap};

pub use source::{DiskFiles, FileSource, MemoryFiles, normalize};
pub use std_library::{PRELUDE_MODULES, STD_MODULES, std_files};

/// Identifies a module in a [`ModuleSet`]: its index in
/// [`ModuleSet::modules`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ModuleId(u32);

impl ModuleId {
    /// The module's index in [`ModuleSet::modules`].
    pub fn index(self) -> usize {
        // A `u32` always fits in `usize` on the 32- and 64-bit targets Stave
        // supports.
        usize::try_from(self.0).unwrap_or(usize::MAX)
    }
}

/// Where a module's files come from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModuleSource {
    /// A folder on this computer.
    Local {
        /// The folder's canonical path ([`FileSource::canonical`]).
        folder: PathBuf,
    },
    /// A standard module embedded in Stave.
    Std {
        /// One of [`STD_MODULES`]: `"joinery"`, `"hardware"`, `"tools"`,
        /// `"casework"` or `"frames"`.
        name: &'static str,
    },
}

/// A module's single `project` or `library` item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModuleHeader {
    /// The module is a project.
    Project {
        /// The file the item is in.
        file: FileId,
        /// Index of the item in that file's `ast.items`.
        item: usize,
    },
    /// The module is a library.
    Library {
        /// The file the item is in.
        file: FileId,
        /// Index of the item in that file's `ast.items`.
        item: usize,
    },
}

/// One module: every `.stave` file directly inside one folder.
#[derive(Debug, Clone, PartialEq)]
pub struct Module {
    /// The module's id.
    pub id: ModuleId,
    /// The module's default name: the folder's last segment (`"casework"`,
    /// `"joinery"`). An import may give it another name with `as`.
    pub name: String,
    /// Where the files come from.
    pub source: ModuleSource,
    /// The module's single `project` or `library` item; `None` when it is
    /// missing or there are several (both diagnosed).
    pub header: Option<ModuleHeader>,
    /// The module's files, sorted by file name. Patch files are not part of
    /// a module.
    pub files: Vec<ModuleFile>,
}

/// One file of a module.
#[derive(Debug, Clone, PartialEq)]
pub struct ModuleFile {
    /// The file's id in [`ModuleSet::sources`].
    pub file: FileId,
    /// The display path, the same string given to the [`SourceMap`].
    pub path: String,
    /// The parsed file; `None` when the file has no readable header.
    pub ast: Option<File>,
    /// The imports that resolved to a module and have a usable, unique name,
    /// in source order. An import that closes a cycle is still listed.
    pub imports: Vec<FileImport>,
    /// Items left out of [`ModuleFile::ast`] because of a syntax error.
    /// Later stages skip uses of their names instead of reporting them
    /// as unknown.
    pub broken: Vec<stave_lang::BrokenItem>,
}

/// An import that resolved to a module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileImport {
    /// The name used to qualify the module's items in this file: the alias
    /// if given, else the module's default name.
    pub name: String,
    /// The alias's span, or the source string's span when there is none.
    pub name_span: Span,
    /// The imported module.
    pub module: ModuleId,
    /// The whole import.
    pub span: Span,
}

/// Every module a project needs, parsed, with this stage's diagnostics.
#[derive(Debug, Clone)]
pub struct ModuleSet {
    /// Every file loaded, standard files included, patch files included.
    pub sources: SourceMap,
    /// Every module, indexed by [`ModuleId`].
    pub modules: Vec<Module>,
    /// The project folder's module.
    pub root: ModuleId,
    /// The standard modules joinery, hardware and tools, in that order.
    pub prelude: Vec<ModuleId>,
    /// Parse diagnostics of every file plus this stage's, sorted by (file
    /// id, span start, span end).
    pub diagnostics: Vec<Diagnostic>,
}

impl ModuleSet {
    /// The module with this id.
    pub fn module(&self, id: ModuleId) -> Option<&Module> {
        self.modules.get(id.index())
    }

    /// Loads the project in `folder` from disk.
    pub fn load(folder: &Path) -> Result<ModuleSet, LoadError> {
        Self::load_with(folder, &DiskFiles)
    }

    /// Loads the project in `folder`, reading files from `files`.
    pub fn load_with(folder: &Path, files: &dyn FileSource) -> Result<ModuleSet, LoadError> {
        loader::load(folder, files, None)
    }

    /// Loads the project in `folder` again, reading files from `files`,
    /// and reuses this load's parse of every file whose id, display path
    /// and text are unchanged and that had no diagnostic. The result is
    /// the same as [`ModuleSet::load_with`]'s; only the work differs (an
    /// editor checking a changed copy of a project parses the standard
    /// library once).
    pub fn reload_with(
        &self,
        folder: &Path,
        files: &dyn FileSource,
    ) -> Result<ModuleSet, LoadError> {
        loader::load(folder, files, Some(self))
    }

    /// `true` if any diagnostic is an error.
    pub fn has_errors(&self) -> bool {
        self.diagnostics.iter().any(Diagnostic::is_error)
    }
}

/// Why the project folder itself can't be loaded. Problems with imported
/// modules are diagnostics, not errors.
#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    /// The folder doesn't exist or can't be listed.
    #[error("can't read the folder '{}': {source}", folder.display())]
    Unreadable {
        /// The folder as given.
        folder: PathBuf,
        /// What went wrong.
        source: io::Error,
    },
    /// The path is a file, not a folder.
    #[error("'{}' is not a folder", folder.display())]
    NotAFolder {
        /// The path as given.
        folder: PathBuf,
    },
    /// The folder has no `.stave` files directly inside it.
    #[error("there are no .stave files in '{}'", folder.display())]
    NoStaveFiles {
        /// The folder as given.
        folder: PathBuf,
    },
    /// A `.stave` file in the folder can't be read.
    #[error("can't read '{}': {source}", path.display())]
    UnreadableFile {
        /// The file's path.
        path: PathBuf,
        /// What went wrong.
        source: io::Error,
    },
    /// A file is too large, or there are too many files, for the source map.
    #[error(transparent)]
    Sources(#[from] SourceMapError),
}
