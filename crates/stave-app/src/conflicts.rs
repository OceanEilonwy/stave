//! Conflicts (grammar, "Conflicts") as the window and the AI tools use
//! them: each conflict in a file with the ways to settle it, each side of
//! a project checked on its own, a side shown as a change against its
//! base, and a resolution tried on a scratch copy before it is written.
//!
//! ```text
//! file text ──find_conflicts──▶ FileConflict { lines, actions: Keep yours, … }
//! project   ──SideFiles(k)───▶ open_with ──▶ side k's problems
//! resolve   ──Overlay────────▶ open_with ──▶ Tried { text, problems }
//! ```
//!
//! Nothing here writes a file: the window applies an action's edit to its
//! buffer, and the AI tools write through the project's history.

use std::io;
use std::ops::RangeInclusive;
use std::path::{Path, PathBuf};

use stave_lang::conflict::{
    self, Conflict, ConflictSide, Resolution, ResolveError, find_conflicts,
};
use stave_lang::patch::{TextEdit, changed_line_runs};
use stave_modules::{FileSource, LoadError};

use crate::i18n::{fill_in, tr};
use crate::{Opened, open_with};

/// One conflict in a file, with the ways to settle it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileConflict {
    /// Its number in the file, from 0 (an index into
    /// [`find_conflicts`]).
    pub index: usize,
    /// Its lines, counted from 1: the opening marker line to the closing
    /// one.
    pub lines: RangeInclusive<usize>,
    /// The conflict as the language reads it.
    pub conflict: Conflict,
    /// The ways to settle it, in the order an editor offers them.
    pub actions: Vec<ConflictAction>,
}

/// A way to settle one conflict, worded for the window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConflictAction {
    /// The text on its button, naming the side it keeps: "Keep yours",
    /// "Keep the green side", "Keep both" (see [`actions`]).
    pub label: String,
    /// What it does.
    pub resolution: Resolution,
    /// The same as one edit of the file's text.
    pub edit: TextEdit,
}

/// Every conflict in `text`, in order, with its actions.
pub fn conflicts(text: &str) -> Vec<FileConflict> {
    find_conflicts(text)
        .into_iter()
        .enumerate()
        .map(|(index, conflict)| FileConflict {
            index,
            lines: line_of(text, conflict.range.start)
                ..=line_of(text, conflict.range.end.saturating_sub(1)),
            actions: actions(text, index),
            conflict,
        })
        .collect()
}

/// The ways to settle conflict number `index` of `text` (from 0): keep
/// each side, then keep both when the sides are whole items with
/// different names. Empty when there is no such conflict.
///
/// Every link sits in one row above the conflict, so each "Keep" names
/// the side it keeps: by the label Stave's versions write ("Keep yours"
/// for the window's saved work, "Keep theirs" for the change applied
/// meanwhile, "Keep the suggestion's" and "Keep the project's" for a
/// suggestion moved onto main), else, in a two-sided conflict, by the
/// tint the window gives it (the first side green, the other blue), else
/// by its number ("Keep side 3").
pub fn actions(text: &str, index: usize) -> Vec<ConflictAction> {
    let found = find_conflicts(text);
    let sides = found
        .get(index)
        .map_or(&[][..], |conflict| conflict.sides.as_slice());
    conflict::fixes(text, index)
        .into_iter()
        .map(|fix| ConflictAction {
            label: action_label(sides, &fix.resolution),
            resolution: fix.resolution,
            edit: fix.edit,
        })
        .collect()
}

/// The words for a resolution on a button, `sides` being the conflict's.
fn action_label(sides: &[ConflictSide], resolution: &Resolution) -> String {
    match resolution {
        Resolution::Keep(side) => keep_label(sides, *side),
        Resolution::KeepBoth => tr("Keep both"),
        Resolution::Text(_) => tr("Use this text"),
    }
}

/// "Keep …" for side number `side` (from 0) of a conflict with `sides`.
fn keep_label(sides: &[ConflictSide], side: usize) -> String {
    let label = sides.get(side).map_or("", |found| found.label.as_str());
    if label == "yours" {
        return tr("Keep yours");
    }
    if label == "theirs" {
        return tr("Keep theirs");
    }
    if label.ends_with("(rebased revision)") {
        return tr("Keep the suggestion's");
    }
    if label.ends_with("(rebase destination)") {
        return tr("Keep the project's");
    }
    match (sides.len(), side) {
        (2, 0) => tr("Keep the green side"),
        (2, 1) => tr("Keep the blue side"),
        _ => fill_in(
            &tr("Keep side {number}"),
            &[("number", &side.saturating_add(1))],
        ),
    }
}

/// The line, counted from 1, that byte `offset` of `text` is on.
fn line_of(text: &str, offset: usize) -> usize {
    text.get(..offset)
        .map_or(0, |before| before.matches('\n').count())
        .saturating_add(1)
}

/// What one side of a conflict changes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SideChange {
    /// The lines the side changes from the conflict's base: each run of
    /// changed lines as the base's lines, each starting `- `, then the
    /// side's, each starting `+ `. Empty when the side is the base.
    Against {
        /// The changed lines, each ending in a line feed.
        lines: String,
    },
    /// The conflict doesn't say what the sides changed from (a git
    /// conflict without its base): the side's text, whole.
    Whole {
        /// The side's text.
        text: String,
    },
}

/// What side number `side` (from 0) of `conflict`, a conflict of `text`,
/// changes from the conflict's base. A side that doesn't exist is empty.
pub fn side_change(text: &str, conflict: &Conflict, side: usize) -> SideChange {
    let section = |range: &std::ops::Range<usize>| text.get(range.clone()).unwrap_or("");
    let side_text = conflict
        .sides
        .get(side)
        .map_or("", |chosen| section(&chosen.range));
    let Some(base) = &conflict.base else {
        return SideChange::Whole {
            text: side_text.to_owned(),
        };
    };
    let base_text = section(&base.range);
    let base_lines: Vec<&str> = base_text.lines().collect();
    let side_lines: Vec<&str> = side_text.lines().collect();
    let mut lines = String::new();
    for (old, new) in changed_line_runs(base_text, side_text) {
        for line in base_lines.get(old).unwrap_or_default() {
            lines.push_str("- ");
            lines.push_str(line);
            lines.push('\n');
        }
        for line in side_lines.get(new).unwrap_or_default() {
            lines.push_str("+ ");
            lines.push_str(line);
            lines.push('\n');
        }
    }
    SideChange::Against { lines }
}

/// A project's files with one side chosen in every conflict: the files
/// that side is checked as (grammar, "Conflicts", "Checking").
pub struct SideFiles<'a> {
    files: &'a dyn FileSource,
    side: usize,
}

impl<'a> SideFiles<'a> {
    /// `files` with side `side` (from 0) chosen in every conflict; a
    /// conflict with fewer sides gives its last.
    pub fn new(files: &'a dyn FileSource, side: usize) -> Self {
        Self { files, side }
    }
}

impl FileSource for SideFiles<'_> {
    fn list_stave_files(&self, folder: &Path) -> io::Result<Vec<PathBuf>> {
        self.files.list_stave_files(folder)
    }

    fn read(&self, path: &Path) -> io::Result<String> {
        let text = self.files.read(path)?;
        Ok(if find_conflicts(&text).is_empty() {
            text
        } else {
            conflict::side_text(&text, self.side)
        })
    }

    fn canonical(&self, folder: &Path) -> io::Result<PathBuf> {
        self.files.canonical(folder)
    }
}

/// The project in `folder` opened once per side, that side chosen in
/// every conflict of every file: one [`Opened`] for each side of the
/// conflict with the most sides. Empty when the project has no conflicts.
///
/// # Errors
///
/// When the project can't be loaded.
pub fn check_sides(folder: &Path, files: &dyn FileSource) -> Result<Vec<Opened>, LoadError> {
    let sides = side_count(folder, files)?;
    (0..sides)
        .map(|side| open_with(folder, &SideFiles::new(files, side)))
        .collect()
}

/// How many sides the conflict with the most sides has, in any of the
/// project's own `.stave` files; 0 without conflicts. Reads the files'
/// text only: the project is opened once per side afterwards.
fn side_count(folder: &Path, files: &dyn FileSource) -> Result<usize, LoadError> {
    let paths = files
        .list_stave_files(folder)
        .map_err(|source| LoadError::Unreadable {
            folder: folder.to_owned(),
            source,
        })?;
    let mut most = 0;
    for path in paths {
        let text = files
            .read(&path)
            .map_err(|source| LoadError::UnreadableFile {
                path: path.clone(),
                source,
            })?;
        for found in find_conflicts(&text) {
            most = most.max(found.sides.len());
        }
    }
    Ok(most)
}

/// A resolution tried on a scratch copy.
#[derive(Debug)]
pub struct Tried {
    /// The file's text with the conflict settled.
    pub text: String,
    /// The project opened with that text in place of the file's.
    pub opened: Opened,
}

/// Why a resolution can't be tried.
#[derive(Debug, thiserror::Error)]
pub enum ConflictError {
    /// The file couldn't be read.
    #[error("couldn't read {}: {source}", path.display())]
    Read {
        /// The file.
        path: PathBuf,
        /// What the system said.
        source: io::Error,
    },
    /// No such conflict or side.
    #[error(transparent)]
    Resolve(#[from] ResolveError),
    /// The project couldn't be loaded.
    #[error(transparent)]
    Load(#[from] LoadError),
}

/// Settles conflict number `index` (from 0) of the file at `path` in the
/// project in `folder` by `resolution`, on a scratch copy, and checks the
/// project as it would then be. Writes nothing.
///
/// # Errors
///
/// When the file can't be read, it has no such conflict or side, or the
/// project can't be loaded.
pub fn try_resolution(
    folder: &Path,
    files: &dyn FileSource,
    path: &Path,
    index: usize,
    resolution: &Resolution,
) -> Result<Tried, ConflictError> {
    let text = files.read(path).map_err(|source| ConflictError::Read {
        path: path.to_owned(),
        source,
    })?;
    let resolved = conflict::resolve(&text, index, resolution)?;
    let opened = open_with(folder, &Replaced::new(files, path, &resolved))?;
    Ok(Tried {
        text: resolved,
        opened,
    })
}

/// A project's files with one file's text replaced: the project as a
/// tried resolution would leave it, for [`check_sides`] or [`open_with`].
pub struct Replaced<'a> {
    files: &'a dyn FileSource,
    path: &'a Path,
    text: &'a str,
}

impl<'a> Replaced<'a> {
    /// `files` with the file at `path` reading `text`.
    pub fn new(files: &'a dyn FileSource, path: &'a Path, text: &'a str) -> Self {
        Self { files, path, text }
    }
}

impl FileSource for Replaced<'_> {
    fn list_stave_files(&self, folder: &Path) -> io::Result<Vec<PathBuf>> {
        self.files.list_stave_files(folder)
    }

    fn read(&self, path: &Path) -> io::Result<String> {
        if path == self.path {
            Ok(self.text.to_owned())
        } else {
            self.files.read(path)
        }
    }

    fn canonical(&self, folder: &Path) -> io::Result<PathBuf> {
        self.files.canonical(folder)
    }
}
