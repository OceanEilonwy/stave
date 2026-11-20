//! The window's versions (`docs/gui.md`, "Versions"): a save merges the
//! window's files into the project's main line, the window catches up when
//! someone else moved main, the question about turning versions on, and
//! the suggestions agents made, which the person shows, applies or
//! discards from the window.
//!
//! Everything here is headless and blocking (each call opens the project's
//! versions from its folder, so it reads them as they are on disk); the
//! window runs it off its main thread. The revision system itself is
//! `stave-history`; this module decides what the window does with it.
//!
//! **Catching up never overwrites an unsaved edit.** When main moved, the
//! files it changed are written into the folder (and the window reloads
//! their clean buffers, as for any file changed on disk) at once, unless the
//! window holds one of them unsaved: then the whole catch-up waits for the
//! next save, which merges the person's files with main
//! ([`History::save`]) and catches up after.

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, PoisonError, mpsc};
use std::time::Duration;

use stave_lang::{Code, Severity, SourceMap, parse_patch_file, render_diagnostics};
use stave_modules::MemoryFiles;

use crate::edit::Verdict;
use crate::i18n::{fill_in, ntr, tr};
use crate::scratch::{EditBase, one_patch_each};

pub use stave_history::{
    Actor, ActorKind, CatchUp, History, HistoryError, PathError, ProjectFiles, RelativePath,
    RevisionId, Suggestion, SuggestionId, Version, VersionsStatus, Written,
};

/// The id the person's revisions carry: there is one window per person.
const WINDOW_ID: &str = "window";

/// How long `.stave/history` must be quiet after a change before the
/// window hears of it.
const QUIET: Duration = Duration::from_millis(250);

/// The person using the window, as their revisions name them.
pub fn user_actor(name: &str) -> Actor {
    Actor {
        kind: ActorKind::User,
        name: name.to_owned(),
        id: WINDOW_ID.to_owned(),
    }
}

/// Why something the window asked of the versions didn't happen.
#[derive(Debug, thiserror::Error)]
pub enum VersionsError {
    /// The revision system said no, or couldn't read or write. Worded
    /// here, for the window.
    #[error("{}", history_words(.0))]
    History(#[from] HistoryError),
    /// A file in the folder couldn't be written or removed.
    #[error("{}", fill_in(&tr("couldn't write {file}: {reason}"), &[("file", &path.display()), ("reason", source)]))]
    Io {
        /// The file.
        path: PathBuf,
        /// What the system said.
        #[source]
        source: std::io::Error,
    },
    /// Catching up would write or remove a file through a symbolic link,
    /// which may lead out of the project: nothing was written.
    #[error("{}", fill_in(&tr("{file} is a link, which may lead out of the project, so the project's changes weren't written into its folder"), &[("file", &path.display())]))]
    Link {
        /// The link, inside the project.
        path: PathBuf,
    },
    /// The project as main holds it couldn't be read as a design.
    #[error("{}", fill_in(&tr("the project couldn't be read: {reason}"), &[("reason", source)]))]
    Load {
        /// What the loader said.
        #[source]
        source: stave_modules::LoadError,
    },
    /// A suggestion's patch no longer applies to the project as it is now,
    /// or the design wouldn't check without errors after it. Main is
    /// unchanged.
    #[error("{}", fill_in(&tr("\"{title}\" can't be applied: {why}"), &[("title", title), ("why", why)]))]
    Refused {
        /// The suggestion's title.
        title: String,
        /// Why, with the problems it would cause.
        why: String,
    },
    /// A suggestion's patch no longer fits the project, and moving the
    /// suggestion onto the project as it is now left clashing changes in
    /// it, to resolve before it can be applied.
    #[error("{}", fill_in(&tr("\"{title}\" no longer fits the project, and its changes clash in {files}; resolve them first"), &[("title", title), ("files", &list(conflicted))]))]
    Clashes {
        /// The suggestion's title.
        title: String,
        /// The files with clashing changes.
        conflicted: Vec<RelativePath>,
    },
    /// No suggestion has this id (applied or discarded meanwhile).
    #[error("{}", fill_in(&tr("there is no suggestion {id}; it may have been applied or discarded already"), &[("id", id)]))]
    UnknownSuggestion {
        /// The id asked for.
        id: SuggestionId,
    },
}

/// What the revision system said, in the window's words: translated, and
/// never naming the tools underneath. What the store itself said stays on
/// the error's source, out of the toast.
fn history_words(error: &HistoryError) -> String {
    match error {
        HistoryError::Nested { repository_root } => fill_in(
            &tr(
                "this project is inside another project's versions (at {folder}); versions can't be turned on here, but a copy of the project in a folder of its own can have them",
            ),
            &[("folder", &repository_root.display())],
        ),
        HistoryError::Off => tr("versions are off for this project; turn them on first"),
        HistoryError::Io { path, source } => fill_in(
            &tr("couldn't use {file}: {reason}"),
            &[("file", &path.display()), ("reason", source)],
        ),
        HistoryError::Path(_) => tr("a file's name isn't a name inside the project"),
        HistoryError::Store(_) => tr("the project's versions couldn't be read or written"),
        HistoryError::UnknownRevision(id) => {
            fill_in(&tr("there is no version {id}"), &[("id", id)])
        }
        HistoryError::UnknownSuggestion(id) => fill_in(
            &tr("there is no suggestion {id}; it may have been applied or discarded already"),
            &[("id", id)],
        ),
        HistoryError::SuggestionConflicted { files, .. } => fill_in(
            &tr("the suggestion has clashing changes in {files}; resolve them before applying it"),
            &[("files", &list(files))],
        ),
        HistoryError::MainMoved { files } => fill_in(
            &tr(
                "the project changed while this was being prepared, and the changes clash in {files}; try again",
            ),
            &[("files", &list(files))],
        ),
        HistoryError::MainMovedSince { .. } => {
            tr("the project changed while this was being checked; try again")
        }
        HistoryError::SuggestionMoved { .. } => {
            tr("the suggestion changed while this was being prepared; try again")
        }
        HistoryError::SettingsProtected { path } => fill_in(
            &tr("a suggestion can't change {file}: the project's settings are changed only by you"),
            &[("file", path)],
        ),
        HistoryError::BranchMoved { .. } => {
            tr("the project's folder changed while this was being recorded; try again")
        }
        HistoryError::NothingToUndo => tr("there is nothing to undo"),
        HistoryError::UndoBlocked { undone, later } => fill_in(
            &tr("can't undo “{undone}”: {later} came after it and builds on it"),
            &[("undone", undone), ("later", later)],
        ),
    }
}

fn list(files: &[RelativePath]) -> String {
    files
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

/// What a save does about versions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnSave {
    /// Versions are on: the save is recorded and merged into main.
    Record,
    /// Versions are off and the person hasn't said "Not now" for this
    /// project: ask "Turn on versions for this project?".
    AskToTurnOn,
    /// Nothing: the person said "Not now", or the project sits inside a
    /// bigger repository, where versions can't be turned on.
    Nothing,
}

/// What a save does about versions, given whether they are on and whether
/// the person said "Not now" for this project.
pub fn on_save(status: &VersionsStatus, declined: bool) -> OnSave {
    match status {
        VersionsStatus::On => OnSave::Record,
        VersionsStatus::Off if declined => OnSave::Nothing,
        VersionsStatus::Off => OnSave::AskToTurnOn,
        VersionsStatus::Nested { .. } => OnSave::Nothing,
    }
}

/// The projects whose question about versions the person answered "Not
/// now": one canonical folder per line in a file under the configuration
/// folder (`~/.config/stave/versions-not-now`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Declined {
    file: PathBuf,
}

impl Declined {
    /// The list kept in `file`.
    pub fn new(file: PathBuf) -> Self {
        Self { file }
    }

    /// Whether the person said "Not now" for `folder`. A list that can't
    /// be read holds nothing.
    pub fn contains(&self, folder: &Path) -> bool {
        let folder = canonical(folder);
        std::fs::read_to_string(&self.file)
            .is_ok_and(|text| text.lines().any(|line| Path::new(line) == folder))
    }

    /// Remembers "Not now" for `folder`.
    ///
    /// # Errors
    ///
    /// [`VersionsError::Io`] when the list can't be written.
    pub fn add(&self, folder: &Path) -> Result<(), VersionsError> {
        if self.contains(folder) {
            return Ok(());
        }
        let io = |source| VersionsError::Io {
            path: self.file.clone(),
            source,
        };
        if let Some(parent) = self.file.parent() {
            std::fs::create_dir_all(parent).map_err(io)?;
        }
        let mut text = std::fs::read_to_string(&self.file).unwrap_or_default();
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        let _ = writeln!(text, "{}", canonical(folder).display());
        std::fs::write(&self.file, text).map_err(io)
    }
}

/// `folder` made absolute and free of links, or as given when it can't be.
fn canonical(folder: &Path) -> PathBuf {
    std::fs::canonicalize(folder).unwrap_or_else(|_| folder.to_path_buf())
}

/// When the window catches up with main.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CatchUpWhen {
    /// Now: no file main changed has unsaved edits in the window.
    Now,
    /// At the next save: the window holds these files, which main changed,
    /// unsaved.
    AtNextSave {
        /// The unsaved files main changed, sorted.
        waiting: Vec<RelativePath>,
    },
}

/// Decides when to catch up: `main_files` are main's, `folder_files` the
/// folder's as saved, `unsaved` the files the window holds unsaved. A file
/// main changed is one whose contents (or presence) differ between the two;
/// that over-counts the window's own saves not yet in step, which only
/// makes the window wait when it needn't, never write over an edit.
pub fn when_to_catch_up(
    main_files: &ProjectFiles,
    folder_files: &ProjectFiles,
    unsaved: &BTreeSet<RelativePath>,
) -> CatchUpWhen {
    let waiting: Vec<RelativePath> = unsaved
        .iter()
        .filter(|path| main_files.get(path) != folder_files.get(path))
        .cloned()
        .collect();
    if waiting.is_empty() {
        CatchUpWhen::Now
    } else {
        CatchUpWhen::AtNextSave { waiting }
    }
}

/// What catching up did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CaughtUp {
    /// The folder already holds main.
    UpToDate,
    /// Main's changes were written into the folder.
    Wrote {
        /// The files written, as full paths, sorted: the window reloads
        /// their buffers.
        written: Vec<PathBuf>,
        /// The files removed, as full paths, sorted.
        removed: Vec<PathBuf>,
        /// Files among `written` where the window's changes clash with
        /// main's: they hold conflict markers.
        conflicted: Vec<RelativePath>,
    },
    /// Nothing was written: the window holds these files, which main
    /// changed, unsaved. The next save merges them.
    Waiting {
        /// The unsaved files main changed.
        files: Vec<RelativePath>,
    },
}

/// The files the window holds unsaved, as paths inside `folder`; files
/// outside it (the standard library) are left out.
pub fn unsaved_inside(folder: &Path, unsaved: &BTreeSet<PathBuf>) -> BTreeSet<RelativePath> {
    unsaved
        .iter()
        .filter_map(|file| {
            let relative = file.strip_prefix(folder).ok()?;
            RelativePath::new(relative).ok()
        })
        .collect()
}

/// The files the window holds unsaved, as a catch-up reads them: once to
/// decide, under the versions' lock, and again just before it writes.
pub trait UnsavedFiles {
    /// The unsaved files inside `folder`, as they are now.
    fn inside(&self, folder: &Path) -> BTreeSet<RelativePath>;
}

impl UnsavedFiles for BTreeSet<RelativePath> {
    fn inside(&self, _folder: &Path) -> BTreeSet<RelativePath> {
        self.clone()
    }
}

/// The files the window holds unsaved, shared between the window, which
/// keeps it up to date as people type and save, and the work done off its
/// main thread, which reads it when it is about to write. A copy taken
/// when the work was queued could be stale by the time it runs.
#[derive(Debug, Clone, Default)]
pub struct Unsaved(Arc<Mutex<BTreeSet<PathBuf>>>);

impl Unsaved {
    /// None yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// The window now holds `files` unsaved (full paths).
    pub fn set(&self, files: BTreeSet<PathBuf>) {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = files;
    }

    /// The full paths held unsaved now.
    pub fn files(&self) -> BTreeSet<PathBuf> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl UnsavedFiles for Unsaved {
    fn inside(&self, folder: &Path) -> BTreeSet<RelativePath> {
        let files = self.files();
        let mut inside = unsaved_inside(folder, &files);
        // The window may name the folder through a link; the versions
        // name it free of links.
        inside.extend(unsaved_inside(&canonical(folder), &files));
        inside
    }
}

/// Catches the window up with main when no unsaved edit stands in the way,
/// writing main's changes into the folder. The decision and the writing
/// happen under the versions' lock, so main can't move in between; a file
/// that changed on disk, or became unsaved, since the folder was read is
/// never written over (the catch-up waits instead).
///
/// # Errors
///
/// [`VersionsError::History`] when the versions can't be read or written;
/// [`VersionsError::Io`] when a file can't be written.
pub fn catch_up(folder: &Path, unsaved: &impl UnsavedFiles) -> Result<CaughtUp, VersionsError> {
    let mut history = History::open(folder)?;
    catch_up_with(&mut history, folder, unsaved)
}

fn catch_up_with(
    history: &mut History,
    given: &Path,
    unsaved: &impl UnsavedFiles,
) -> Result<CaughtUp, VersionsError> {
    let folder = history.folder().to_path_buf();
    let folder_files = history.read_folder()?;
    let now = unsaved.inside(given);
    let caught_up = history.catch_up(&folder_files, &now, |decided| {
        write_caught_up(&folder, given, &folder_files, unsaved, decided)
    })?;
    Ok(match caught_up {
        CatchUp::UpToDate => CaughtUp::UpToDate,
        CatchUp::Waiting { files } => CaughtUp::Waiting { files },
        CatchUp::Write {
            files,
            removed,
            conflicted,
        } => {
            let mut written: Vec<PathBuf> = files
                .paths()
                .map(|relative| folder.join(relative.as_path()))
                .collect();
            let mut gone: Vec<PathBuf> = removed
                .iter()
                .map(|relative| folder.join(relative.as_path()))
                .collect();
            written.sort();
            gone.sort();
            CaughtUp::Wrote {
                written,
                removed: gone,
                conflicted,
            }
        }
    })
}

/// Writes what a catch-up decided into `folder`, under the versions' lock.
/// Every path is checked first, so a link leaves the folder as it was; and
/// so is every file's contents, so a file saved (or made unsaved) since
/// `folder_files` was read holds the whole catch-up back rather than being
/// written over.
fn write_caught_up(
    folder: &Path,
    given: &Path,
    folder_files: &ProjectFiles,
    unsaved: &impl UnsavedFiles,
    decided: &CatchUp,
) -> Result<Written, VersionsError> {
    let CatchUp::Write { files, removed, .. } = decided else {
        return Ok(Written::All);
    };
    let mut writes = Vec::with_capacity(files.len());
    for (relative, contents) in files.iter() {
        writes.push((relative, inside(folder, relative)?, contents));
    }
    let mut removals = Vec::with_capacity(removed.len());
    for relative in removed {
        removals.push((relative, inside(folder, relative)?));
    }
    let now = unsaved.inside(given);
    let mut changed = Vec::new();
    let targets = writes
        .iter()
        .map(|(relative, full, _)| (*relative, full))
        .chain(removals.iter().map(|(relative, full)| (*relative, full)));
    for (relative, full) in targets {
        if now.contains(relative) {
            changed.push(relative.clone());
            continue;
        }
        // A file the folder doesn't show (one `.gitignore` excludes) was
        // never read, so there is nothing to compare it with.
        let Some(read) = folder_files.get(relative) else {
            continue;
        };
        let on_disk = match std::fs::read(full) {
            Ok(contents) => Some(contents),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(source) => {
                return Err(VersionsError::Io {
                    path: full.clone(),
                    source,
                });
            }
        };
        if on_disk.as_deref() != Some(read) {
            changed.push(relative.clone());
        }
    }
    if !changed.is_empty() {
        changed.sort();
        changed.dedup();
        return Ok(Written::HeldBack { files: changed });
    }
    for (_, full, contents) in writes {
        write_file(&full, contents)?;
    }
    for (_, full) in removals {
        match std::fs::remove_file(&full) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(source) => return Err(VersionsError::Io { path: full, source }),
        }
    }
    Ok(Written::All)
}

/// `relative`'s full path in `folder`, or [`VersionsError::Link`] when it,
/// or a folder on the way to it, is a symbolic link: writing through it
/// could change a file outside the project. Parts not there yet are fine;
/// they are made as plain folders.
fn inside(folder: &Path, relative: &RelativePath) -> Result<PathBuf, VersionsError> {
    let mut full = folder.to_path_buf();
    for component in relative.as_path().components() {
        full.push(component);
        match std::fs::symlink_metadata(&full) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(VersionsError::Link { path: full });
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
            Err(source) => return Err(VersionsError::Io { path: full, source }),
        }
    }
    Ok(folder.join(relative.as_path()))
}

fn write_file(full: &Path, contents: &[u8]) -> Result<(), VersionsError> {
    let io = |source| VersionsError::Io {
        path: full.to_path_buf(),
        source,
    };
    if let Some(parent) = full.parent() {
        std::fs::create_dir_all(parent).map_err(io)?;
    }
    std::fs::write(full, contents).map_err(io)
}

/// What a save did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Saved {
    /// Main's revision after the save; `None` when the files were the same
    /// as main's and nothing was recorded.
    pub revision: Option<RevisionId>,
    /// Catching up after it.
    pub caught_up: CaughtUp,
}

/// The message a save is recorded with: the files saved, by their path in
/// `folder` (the window's folder, as the saved paths are given; failing
/// that, both made free of links). A file found in neither is named by its
/// file name alone, so no full path reaches the versions, which AI apps
/// read too.
fn save_message(folder: &Path, saved: &[PathBuf]) -> String {
    let canonical_folder = canonical(folder);
    let names: Vec<String> = saved
        .iter()
        .map(|file| {
            let name = file
                .strip_prefix(folder)
                .map(Path::to_path_buf)
                .or_else(|_| {
                    canonical(file)
                        .strip_prefix(&canonical_folder)
                        .map(Path::to_path_buf)
                })
                .ok()
                .or_else(|| file.file_name().map(PathBuf::from))
                .unwrap_or_default();
            name.display().to_string()
        })
        .collect();
    match names.as_slice() {
        [] => tr("Saved"),
        [one] => fill_in(&tr("Saved {file}"), &[("file", one)]),
        many => fill_in(
            &ntr("Saved {count} file", "Saved {count} files", many.len()),
            &[("count", &many.len())],
        ),
    }
}

/// Records the folder's files as saved by `actor` and merges them into
/// main, then catches up with main (unless `unsaved`, the files the window
/// still holds unsaved, stand in the way). `saved` are the files just
/// saved, full paths, for the revision's message.
///
/// # Errors
///
/// [`VersionsError::History`] when the versions can't be read or written;
/// [`VersionsError::Io`] when catching up can't write a file.
pub fn save(
    folder: &Path,
    actor: &Actor,
    saved: &[PathBuf],
    unsaved: &impl UnsavedFiles,
) -> Result<Saved, VersionsError> {
    let mut history = History::open(folder)?;
    let before = history.main()?;
    let files = history.read_folder()?;
    let message = save_message(folder, saved);
    let main = history.save(&files, actor, &message)?;
    let caught_up = catch_up_with(&mut history, folder, unsaved)?;
    Ok(Saved {
        revision: (main != before).then_some(main),
        caught_up,
    })
}

/// Turns versions on for `folder` and records the project as it is, as
/// `actor`'s first revision; returns main's revision.
///
/// # Errors
///
/// [`VersionsError::History`]: [`HistoryError::Nested`] when the folder
/// sits inside a bigger repository (nothing is written).
pub fn turn_on(folder: &Path, actor: &Actor) -> Result<RevisionId, VersionsError> {
    let mut history = History::start(folder)?;
    let files = history.read_folder()?;
    Ok(history.save(&files, actor, &tr("Turned on versions"))?)
}

/// What the Versions panel shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shown {
    /// Whether versions are on.
    pub status: VersionsStatus,
    /// The suggestions waiting, sorted by id.
    pub suggestions: Vec<Suggestion>,
    /// Main's latest revisions, newest first.
    pub versions: Vec<Version>,
}

/// What the Versions panel shows for `folder`: up to `limit` revisions.
/// Reading an off project writes nothing.
///
/// # Errors
///
/// [`VersionsError::History`] when the versions can't be read.
pub fn read(folder: &Path, limit: usize) -> Result<Shown, VersionsError> {
    let status = History::status(folder)?;
    match status {
        VersionsStatus::On => {
            let history = History::open(folder)?;
            Ok(Shown {
                status,
                suggestions: history.suggestions()?,
                versions: history.versions(limit)?,
            })
        }
        VersionsStatus::Off | VersionsStatus::Nested { .. } => Ok(Shown {
            status,
            suggestions: Vec::new(),
            versions: Vec::new(),
        }),
    }
}

/// What applying a suggestion did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Applied {
    /// The suggestion's title.
    pub title: String,
    /// Main's new revision.
    pub revision: RevisionId,
}

/// Applies a suggestion from the window: its patches are tried again, in
/// order, on main as it is now; when every one fits and the design checks
/// with no errors, the result becomes main's latest revision, by `actor`.
/// When a patch no longer fits, the suggestion is moved onto main as it is
/// now and any clash stays on it. The folder catches up afterwards, as for
/// any change to main.
///
/// # Errors
///
/// [`VersionsError::UnknownSuggestion`]; [`VersionsError::Refused`] when
/// the result has errors; [`VersionsError::Clashes`] when the patch no
/// longer fits and moving the suggestion left clashes;
/// [`VersionsError::History`] when the versions say no (the suggestion
/// still holds clashes, or main kept moving while it was checked).
pub fn apply(folder: &Path, id: &SuggestionId, actor: &Actor) -> Result<Applied, VersionsError> {
    let mut history = History::open(folder)?;
    let mut attempts = APPLY_ATTEMPTS;
    loop {
        attempts = attempts.saturating_sub(1);
        match apply_once(&mut history, id, actor) {
            // Main moved while the result was checked: check again on main
            // as it is now.
            Err(VersionsError::History(HistoryError::MainMovedSince { .. })) if attempts > 0 => {}
            result => return result,
        }
    }
}

/// How many times applying reads main again when it moved meanwhile.
const APPLY_ATTEMPTS: u32 = 3;

fn apply_once(
    history: &mut History,
    id: &SuggestionId,
    actor: &Actor,
) -> Result<Applied, VersionsError> {
    let suggestion = history
        .suggestions()?
        .into_iter()
        .find(|suggestion| &suggestion.id == id)
        .ok_or_else(|| VersionsError::UnknownSuggestion { id: id.clone() })?;
    let base = history.main()?;
    let base_files = history.files_at(&base)?;
    let (base, merged) = match replay(history.folder(), &base_files, &suggestion.patches)? {
        Replayed::Clean(merged) => (base, merged),
        Replayed::Errors(why) => {
            return Err(VersionsError::Refused {
                title: suggestion.title,
                why,
            });
        }
        // What a patch names is no longer there: the suggestion's own
        // files are moved onto main, and applied when that is clean.
        Replayed::DoesNotFit => {
            let conflicted = history.rebase_suggestion(id, actor)?;
            if !conflicted.is_empty() {
                return Err(VersionsError::Clashes {
                    title: suggestion.title,
                    conflicted,
                });
            }
            let rebased = history
                .suggestions()?
                .into_iter()
                .find(|suggestion| &suggestion.id == id)
                .ok_or_else(|| VersionsError::UnknownSuggestion { id: id.clone() })?;
            let files = history.files_at(&rebased.tip)?;
            if let Some(why) = errors_in(history.folder(), &files)? {
                return Err(VersionsError::Refused {
                    title: suggestion.title,
                    why,
                });
            }
            (rebased.base, files)
        }
    };
    let revision = history.apply(id, actor, &base, &merged)?;
    Ok(Applied {
        title: suggestion.title,
        revision,
    })
}

/// A suggestion's files, read to settle its clashes in the editor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToResolve {
    /// The suggestion.
    pub id: SuggestionId,
    /// Its title.
    pub title: String,
    /// The latest revision the files were read from: saving them back is
    /// refused when the suggestion moved on since.
    pub tip: RevisionId,
    /// Its files; those with clashes hold conflict markers.
    pub files: ProjectFiles,
    /// The files with clashes.
    pub conflicted: Vec<RelativePath>,
}

/// Reads suggestion `id`'s files to resolve them in the editor.
///
/// # Errors
///
/// [`VersionsError::UnknownSuggestion`]; [`VersionsError::History`] when
/// the versions can't be read.
pub fn to_resolve(folder: &Path, id: &SuggestionId) -> Result<ToResolve, VersionsError> {
    let history = History::open(folder)?;
    let suggestion = history
        .suggestions()?
        .into_iter()
        .find(|suggestion| &suggestion.id == id)
        .ok_or_else(|| VersionsError::UnknownSuggestion { id: id.clone() })?;
    let files = history.files_at(&suggestion.tip)?;
    Ok(ToResolve {
        id: suggestion.id,
        title: suggestion.title,
        tip: suggestion.tip,
        files,
        conflicted: suggestion.conflicted,
    })
}

/// Saves resolved files back to the suggestion they were read from (never
/// to the folder), as `actor`; `resolved.files` hold the edited files.
/// Returns the files still holding clashes, and the new latest revision.
///
/// # Errors
///
/// [`VersionsError::History`]: [`HistoryError::SuggestionMoved`] when the
/// suggestion moved on since it was read, [`HistoryError::UnknownSuggestion`]
/// when it is gone.
pub fn save_resolved(
    folder: &Path,
    resolved: &ToResolve,
    actor: &Actor,
) -> Result<ToResolve, VersionsError> {
    let mut history = History::open(folder)?;
    history.write_suggestion_files(&resolved.id, &resolved.tip, &resolved.files, actor)?;
    drop(history);
    to_resolve(folder, &resolved.id)
}

/// Merges a file's text as the person changed it (`yours`, edited from
/// `before`) with the text it has on disk now (changed meanwhile by
/// someone else, from the same `before`), for saving: where both changed
/// the same lines, both are kept between conflict markers. Returns the
/// merged text and whether it holds markers.
pub fn merge_for_save(before: &str, yours: &str, on_disk: &str) -> (String, bool) {
    let merged = stave_history::merge_text(before.as_bytes(), yours.as_bytes(), on_disk.as_bytes());
    (
        String::from_utf8_lossy(&merged.contents).into_owned(),
        merged.conflicted,
    )
}

/// Discards a suggestion, as `actor`.
///
/// # Errors
///
/// [`VersionsError::History`] when there is no such suggestion or the
/// versions can't be written.
pub fn discard(folder: &Path, id: &SuggestionId, actor: &Actor) -> Result<(), VersionsError> {
    History::open(folder)?.discard(id, actor)?;
    Ok(())
}

/// What the window's Undo did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Undone {
    /// What was undone.
    pub what: UndoneChange,
    /// Catching up after it.
    pub caught_up: CaughtUp,
}

/// The window's Undo: undoes the last change by anyone, then catches the
/// folder up (unless `unsaved` stands in the way).
///
/// # Errors
///
/// [`VersionsError::History`]: [`HistoryError::NothingToUndo`] when there
/// is nothing to undo.
pub fn undo(
    folder: &Path,
    actor: &Actor,
    unsaved: &impl UnsavedFiles,
) -> Result<Undone, VersionsError> {
    let mut history = History::open(folder)?;
    let action = history.undo(actor, stave_history::UndoScope::Any)?;
    let caught_up = catch_up_with(&mut history, folder, unsaved)?;
    Ok(Undone {
        what: UndoneChange::of(&action),
        caught_up,
    })
}

/// A change Undo undid, by kind, with the words its revision carried.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UndoneChange {
    /// A save, with its message ("Saved shelf.stave").
    Save(String),
    /// Applying a suggestion, with its title.
    Applied(String),
    /// Discarding a suggestion (which is back), with its title.
    Discarded(String),
    /// Making a suggestion (which is gone), with its title.
    Suggested(String),
    /// A change the versions describe in no way the window knows.
    Other,
}

impl UndoneChange {
    /// The kind of change from the versions' own description of it ("save:
    /// First cut", "apply suggestion: Widen the dresser").
    fn of(action: &str) -> UndoneChange {
        type Kind = fn(String) -> UndoneChange;
        let kinds: [(&str, Kind); 4] = [
            ("save: ", UndoneChange::Save),
            ("apply suggestion: ", UndoneChange::Applied),
            ("discard suggestion: ", UndoneChange::Discarded),
            ("suggest: ", UndoneChange::Suggested),
        ];
        kinds
            .into_iter()
            .find_map(|(prefix, kind)| {
                action
                    .strip_prefix(prefix)
                    .map(|words| kind(words.to_owned()))
            })
            .unwrap_or(UndoneChange::Other)
    }

    /// What the window says after undoing it, in plain words.
    pub fn said(&self) -> String {
        match self {
            UndoneChange::Save(message) => fill_in(&tr("Undid “{change}”"), &[("change", message)]),
            UndoneChange::Applied(title) => {
                fill_in(&tr("Undid applying “{title}”"), &[("title", title)])
            }
            UndoneChange::Discarded(title) => fill_in(
                &tr("Brought back the suggestion “{title}”"),
                &[("title", title)],
            ),
            UndoneChange::Suggested(title) => fill_in(
                &tr("Took back the suggestion “{title}”"),
                &[("title", title)],
            ),
            UndoneChange::Other => tr("Undid the last change"),
        }
    }
}

/// How replaying a suggestion's patches went.
enum Replayed {
    /// Every patch fit and the design has no errors: the whole project
    /// after them.
    Clean(ProjectFiles),
    /// A patch no longer fits: something it changes isn't in the project
    /// as it is now (renamed, removed, or added meanwhile).
    DoesNotFit,
    /// A patch can't be read, causes errors, or the design has errors
    /// after them: why.
    Errors(String),
}

/// The name a patch's own diagnostics point into.
const PATCH_NAME: &str = "suggestion";

/// The patch text as a patch file: an AI's patch may leave out the format
/// line, `stave 1`, which is then added.
fn with_header(text: &str) -> String {
    let first = text
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with("//"));
    if first.is_some_and(|line| line.starts_with("stave ")) {
        text.to_owned()
    } else {
        format!("stave {}\n\n{text}", stave_lang::FORMAT_VERSION)
    }
}

/// A suggestion's patches as one patch file, for showing it as a ghost:
/// one format line, then each patch with its own format line (which an
/// AI's patch may or may not have) left out.
pub fn shown_patch(patches: &[String]) -> String {
    let mut text = format!("stave {}\n", stave_lang::FORMAT_VERSION);
    for patch in patches {
        let headed = with_header(patch);
        let mut lines = headed.lines();
        // `with_header` makes sure the first line that isn't blank or a
        // comment is the format line: everything before it, and it, goes.
        for line in lines.by_ref() {
            let line = line.trim();
            if !line.is_empty() && !line.starts_with("//") {
                break;
            }
        }
        text.push('\n');
        for line in lines {
            text.push_str(line);
            text.push('\n');
        }
    }
    text
}

/// Replays `patches` in order on `files` (the project in `folder`), on a
/// scratch copy in memory: nothing is written.
fn replay(
    folder: &Path,
    files: &ProjectFiles,
    patches: &[String],
) -> Result<Replayed, VersionsError> {
    let memory = in_memory(folder, files);
    let mut base =
        EditBase::new(folder, &memory).map_err(|source| VersionsError::Load { source })?;
    let cancel = AtomicBool::new(false);
    let mut merged = files.clone();
    for patch in patches {
        let text = with_header(patch);
        let mut sources = SourceMap::new();
        let Ok(file) = sources.add(PATCH_NAME, text.as_str()) else {
            return Ok(Replayed::Errors(tr("the patch couldn't be read")));
        };
        for one in one_patch_each(file, &parse_patch_file(file, &text)) {
            let tried = match base.try_patch(PATCH_NAME, &one, &cancel) {
                Ok(tried) => tried,
                Err(crate::scratch::TryError::Load(source)) => {
                    return Err(VersionsError::Load { source });
                }
                Err(crate::scratch::TryError::Cancelled(_)) => {
                    return Ok(Replayed::Errors(tr("checking it was stopped")));
                }
            };
            match tried.verdict() {
                Verdict::Fits(_) => {}
                Verdict::Refused(_) | Verdict::Checking => {
                    let shown = tried.shown();
                    // Not applied at all, for a reason other than its own
                    // syntax: what it names isn't there any more.
                    if tried.opened().is_none()
                        && !shown.iter().any(|problem| problem.code == Code::Syntax)
                    {
                        return Ok(Replayed::DoesNotFit);
                    }
                    return Ok(Replayed::Errors(
                        render_diagnostics(tried.sources(), &shown)
                            .trim_end()
                            .to_owned(),
                    ));
                }
            }
            for changed in tried.changed() {
                let Ok(relative) = changed.path.strip_prefix(folder) else {
                    continue;
                };
                let relative = RelativePath::new(relative).map_err(HistoryError::from)?;
                merged.insert(relative, changed.text.as_bytes().to_vec());
            }
            // It fits, so the base always moves; were it not to, the
            // patch didn't apply.
            if base.advance(tried).is_err() {
                return Ok(Replayed::Errors(tr(
                    "the edit was refused, so nothing was written",
                )));
            }
        }
    }
    Ok(match errors_of(base.opened()) {
        None => Replayed::Clean(merged),
        Some(why) => Replayed::Errors(why),
    })
}

/// `files` (the project in `folder`) as files in memory, for a scratch
/// copy; files that aren't text are left out, as the design never reads
/// them.
fn in_memory(folder: &Path, files: &ProjectFiles) -> MemoryFiles {
    let mut memory = MemoryFiles::new();
    for (relative, contents) in files.iter() {
        if let Ok(text) = std::str::from_utf8(contents) {
            memory.insert(folder.join(relative.as_path()), text);
        }
    }
    memory
}

/// The errors of `files` (the project in `folder`) as a design, rendered;
/// `None` when it has none.
fn errors_in(folder: &Path, files: &ProjectFiles) -> Result<Option<String>, VersionsError> {
    let memory = in_memory(folder, files);
    let base = EditBase::new(folder, &memory).map_err(|source| VersionsError::Load { source })?;
    Ok(errors_of(base.opened()))
}

/// The errors of an opened design, rendered; `None` when it has none.
fn errors_of(opened: &crate::Opened) -> Option<String> {
    let errors: Vec<_> = opened
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.severity == Severity::Error)
        .cloned()
        .collect();
    (!errors.is_empty()).then(|| {
        render_diagnostics(&opened.sources, &errors)
            .trim_end()
            .to_owned()
    })
}

/// A job for a window's versions, run on its [`Worker`].
type Job = Box<dyn FnOnce() + Send>;

/// Runs a window's versions jobs (saving, catching up, applying,
/// discarding, undoing) one at a time, in the order they were given, on
/// one background thread.
///
/// They must never overlap: a save reads the folder and records it, and a
/// catch-up moves the window in step with main and then writes main's
/// changes into the folder. A save that read the folder before a catch-up
/// wrote it, and recorded after, would record the old files on top of
/// main and quietly undo the change it caught up with.
///
/// The thread starts with the first job and ends once the worker is
/// dropped and the jobs given have run.
#[derive(Default)]
pub struct Worker {
    sender: std::sync::OnceLock<mpsc::Sender<Job>>,
}

impl std::fmt::Debug for Worker {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("Worker").finish_non_exhaustive()
    }
}

/// The worker's thread couldn't be started, or has stopped: the job
/// didn't run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("{}", tr("the work on the project's versions stopped"))]
pub struct WorkerStopped;

impl Worker {
    /// A worker; its thread starts with the first job.
    pub fn new() -> Self {
        Self::default()
    }

    /// Runs `job` after every job given before it.
    ///
    /// # Errors
    ///
    /// [`WorkerStopped`] when the thread can't be started or has stopped.
    pub fn run(&self, job: impl FnOnce() + Send + 'static) -> Result<(), WorkerStopped> {
        let sender = match self.sender.get() {
            Some(sender) => sender,
            None => {
                let (sender, receiver) = mpsc::channel::<Job>();
                std::thread::Builder::new()
                    .name("stave-versions-work".to_owned())
                    .spawn(move || {
                        for job in receiver {
                            // A job that panics ends alone (whoever waits
                            // on it hears nothing back); the ones after it
                            // still run.
                            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(job));
                        }
                    })
                    .map_err(|_| WorkerStopped)?;
                self.sender.get_or_init(|| sender)
            }
        };
        sender.send(Box::new(job)).map_err(|_| WorkerStopped)
    }
}

/// Watches a project's versions (`.stave/history`) and calls back, on a
/// background thread, once they have been quiet for a moment after a
/// change: someone saved, applied, discarded, suggested or undid. Dropping
/// it stops watching.
pub struct HistoryWatcher {
    /// Kept alive: dropping it stops the notifications, which ends the
    /// debouncing thread.
    _watcher: notify::RecommendedWatcher,
}

impl std::fmt::Debug for HistoryWatcher {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("HistoryWatcher")
            .finish_non_exhaustive()
    }
}

impl HistoryWatcher {
    /// Starts watching `folder`'s versions; `None` when they are off (there
    /// is nothing to watch) or can't be watched.
    pub fn start(folder: &Path, changed: impl Fn() + Send + 'static) -> Option<Self> {
        use notify::Watcher as _;
        let repository = folder.join(".stave").join("history").join("repo");
        if !repository.is_dir() {
            return None;
        }
        let (sender, receiver) = mpsc::channel::<()>();
        let mut watcher =
            notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
                let Ok(event) = event else { return };
                if !matches!(event.kind, notify::EventKind::Access(_)) {
                    // The debouncing thread may be gone; then nobody needs
                    // to know.
                    let _ = sender.send(());
                }
            })
            .ok()?;
        watcher
            .watch(&repository, notify::RecursiveMode::Recursive)
            .ok()?;
        std::thread::Builder::new()
            .name("stave-versions".to_owned())
            .spawn(move || crate::check::debounce(&receiver, QUIET, changed))
            .ok()?;
        Some(Self { _watcher: watcher })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_save_is_named_by_its_files() {
        let folder = Path::new("/p");
        assert_eq!(save_message(folder, &[]), "Saved");
        assert_eq!(
            save_message(folder, &[PathBuf::from("/p/doors/door.stave")]),
            "Saved doors/door.stave"
        );
        assert_eq!(
            save_message(
                folder,
                &[PathBuf::from("/p/a.stave"), PathBuf::from("/p/b.stave")]
            ),
            "Saved 2 files"
        );
        // A file the folder doesn't hold is named by its file name, never
        // its full path.
        assert_eq!(
            save_message(folder, &[PathBuf::from("/elsewhere/home/shelf.stave")]),
            "Saved shelf.stave"
        );
    }

    #[test]
    fn a_save_through_a_linked_folder_is_named_inside_it() {
        let real = tempfile::tempdir().expect("temporary folder");
        let links = tempfile::tempdir().expect("temporary folder");
        std::fs::write(real.path().join("shelf.stave"), "").expect("write");
        let linked = links.path().join("project");
        std::os::unix::fs::symlink(real.path(), &linked).expect("link");
        // The saved path through the link, the folder without it.
        assert_eq!(
            save_message(real.path(), &[linked.join("shelf.stave")]),
            "Saved shelf.stave"
        );
        assert_eq!(
            save_message(&linked, &[real.path().join("shelf.stave")]),
            "Saved shelf.stave"
        );
    }

    #[test]
    fn a_suggestion_is_shown_as_one_patch_file() {
        let shown = shown_patch(&[
            "patch \"One\" {\n  set a = 1mm\n}\n".to_owned(),
            "stave 1\n\npatch \"Two\" {\n  set b = 2mm\n}\n".to_owned(),
            "// by an AI\nstave 1\npatch \"Three\" {\n  set c = 3mm\n}\n".to_owned(),
        ]);
        assert_eq!(shown.matches("stave ").count(), 1, "{shown}");
        assert!(shown.starts_with("stave "), "{shown}");
        for title in ["One", "Two", "Three"] {
            assert!(shown.contains(&format!("patch \"{title}\"")), "{shown}");
        }
        let mut sources = SourceMap::new();
        let file = sources.add(PATCH_NAME, shown.as_str()).expect("added");
        let parsed = parse_patch_file(file, &shown);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        assert!(parsed.ast.is_some());
        assert_eq!(one_patch_each(file, &parsed).len(), 3);
    }

    #[test]
    fn what_undo_undid_is_said_plainly() {
        assert_eq!(
            UndoneChange::of("save: Saved shelf.stave"),
            UndoneChange::Save("Saved shelf.stave".to_owned())
        );
        assert_eq!(
            UndoneChange::of("apply suggestion: Widen the drawer").said(),
            "Undid applying “Widen the drawer”"
        );
        assert_eq!(
            UndoneChange::of("discard suggestion: Widen").said(),
            "Brought back the suggestion “Widen”"
        );
        assert_eq!(UndoneChange::of("something new"), UndoneChange::Other);
    }

    #[test]
    fn the_worker_runs_jobs_one_at_a_time_in_order() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::{Arc, Mutex};
        let worker = Worker::new();
        let running = Arc::new(AtomicUsize::new(0));
        let overlapped = Arc::new(AtomicBool::new(false));
        let order = Arc::new(Mutex::new(Vec::new()));
        let (done, finished) = mpsc::channel();
        for index in 0..8 {
            let running = Arc::clone(&running);
            let overlapped = Arc::clone(&overlapped);
            let order = Arc::clone(&order);
            let done = done.clone();
            worker
                .run(move || {
                    if running.fetch_add(1, Ordering::SeqCst) != 0 {
                        overlapped.store(true, Ordering::SeqCst);
                    }
                    std::thread::sleep(Duration::from_millis(5));
                    order.lock().expect("lock").push(index);
                    running.fetch_sub(1, Ordering::SeqCst);
                    done.send(()).expect("send");
                })
                .expect("run");
        }
        for _ in 0..8 {
            finished
                .recv_timeout(Duration::from_secs(10))
                .expect("a job finished");
        }
        assert!(!overlapped.load(Ordering::SeqCst));
        assert_eq!(*order.lock().expect("lock"), (0..8).collect::<Vec<_>>());
    }

    #[test]
    fn unsaved_files_outside_the_folder_are_left_out() {
        let unsaved: BTreeSet<PathBuf> = [
            PathBuf::from("/p/a.stave"),
            PathBuf::from("/std/hinge.stave"),
        ]
        .into_iter()
        .collect();
        let inside = unsaved_inside(Path::new("/p"), &unsaved);
        assert_eq!(
            inside.into_iter().collect::<Vec<_>>(),
            vec![RelativePath::new(Path::new("a.stave")).expect("path")]
        );
    }

    #[test]
    fn a_patch_without_its_format_line_gets_one() {
        assert!(with_header("patch \"x\" {}\n").starts_with("stave "));
        assert_eq!(
            with_header("stave 1\npatch \"x\" {}\n"),
            "stave 1\npatch \"x\" {}\n"
        );
    }
}
