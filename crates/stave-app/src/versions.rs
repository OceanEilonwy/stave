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
use std::sync::mpsc;
use std::time::Duration;

use stave_lang::{Severity, SourceMap, parse_patch_file, render_diagnostics};
use stave_modules::MemoryFiles;

use crate::edit::Verdict;
use crate::i18n::{fill_in, ntr, tr};
use crate::scratch::{EditBase, one_patch_each};

pub use stave_history::{
    Actor, ActorKind, CatchUp, History, HistoryError, PathError, ProjectFiles, RelativePath,
    RevisionId, Suggestion, SuggestionId, Version, VersionsStatus,
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
    /// The revision system said no, or couldn't read or write.
    #[error(transparent)]
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

/// Catches the window up with main when no unsaved edit stands in the way
/// (see [`when_to_catch_up`]), writing main's changes into the folder.
///
/// # Errors
///
/// [`VersionsError::History`] when the versions can't be read or written;
/// [`VersionsError::Io`] when a file can't be written.
pub fn catch_up(
    folder: &Path,
    unsaved: &BTreeSet<RelativePath>,
) -> Result<CaughtUp, VersionsError> {
    let mut history = History::open(folder)?;
    catch_up_with(&mut history, unsaved)
}

fn catch_up_with(
    history: &mut History,
    unsaved: &BTreeSet<RelativePath>,
) -> Result<CaughtUp, VersionsError> {
    let folder_files = history.read_folder()?;
    let main_files = history.files_at(&history.main()?)?;
    if let CatchUpWhen::AtNextSave { waiting } =
        when_to_catch_up(&main_files, &folder_files, unsaved)
    {
        return Ok(CaughtUp::Waiting { files: waiting });
    }
    match history.catch_up(&folder_files)? {
        CatchUp::UpToDate => Ok(CaughtUp::UpToDate),
        CatchUp::Write {
            files,
            removed,
            conflicted,
        } => {
            let folder = history.folder().to_path_buf();
            let mut written = Vec::with_capacity(files.len());
            for (relative, contents) in files.iter() {
                let full = folder.join(relative.as_path());
                write_file(&full, contents)?;
                written.push(full);
            }
            let mut gone = Vec::with_capacity(removed.len());
            for relative in &removed {
                let full = folder.join(relative.as_path());
                match std::fs::remove_file(&full) {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(source) => return Err(VersionsError::Io { path: full, source }),
                }
                gone.push(full);
            }
            written.sort();
            gone.sort();
            Ok(CaughtUp::Wrote {
                written,
                removed: gone,
                conflicted,
            })
        }
    }
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

/// The message a save is recorded with: the files saved, by name.
fn save_message(folder: &Path, saved: &[PathBuf]) -> String {
    let names: Vec<String> = saved
        .iter()
        .map(|file| {
            file.strip_prefix(folder)
                .unwrap_or(file)
                .display()
                .to_string()
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
    unsaved: &BTreeSet<RelativePath>,
) -> Result<Saved, VersionsError> {
    let mut history = History::open(folder)?;
    let before = history.main()?;
    let files = history.read_folder()?;
    let message = save_message(history.folder(), saved);
    let main = history.save(&files, actor, &message)?;
    let caught_up = catch_up_with(&mut history, unsaved)?;
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
/// [`VersionsError::History`] when the versions say no (main moved and
/// clashes, the suggestion still holds clashes).
pub fn apply(folder: &Path, id: &SuggestionId, actor: &Actor) -> Result<Applied, VersionsError> {
    let mut history = History::open(folder)?;
    let suggestion = history
        .suggestions()?
        .into_iter()
        .find(|suggestion| &suggestion.id == id)
        .ok_or_else(|| VersionsError::UnknownSuggestion { id: id.clone() })?;
    let base = history.main()?;
    let base_files = history.files_at(&base)?;
    let merged = match replay(history.folder(), &base_files, &suggestion.patches)? {
        Replayed::Clean(merged) => merged,
        Replayed::NotApplied(why) => {
            let conflicted = history.rebase_suggestion(id)?;
            if conflicted.is_empty() {
                return Err(VersionsError::Refused {
                    title: suggestion.title,
                    why,
                });
            }
            return Err(VersionsError::Clashes {
                title: suggestion.title,
                conflicted,
            });
        }
        Replayed::Errors(why) => {
            return Err(VersionsError::Refused {
                title: suggestion.title,
                why,
            });
        }
    };
    let revision = history.apply(id, actor, &base, &merged)?;
    Ok(Applied {
        title: suggestion.title,
        revision,
    })
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
    /// What was undone, as the versions describe it.
    pub what: String,
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
    unsaved: &BTreeSet<RelativePath>,
) -> Result<Undone, VersionsError> {
    let mut history = History::open(folder)?;
    let what = history.undo(actor, stave_history::UndoScope::Any)?;
    let caught_up = catch_up_with(&mut history, unsaved)?;
    Ok(Undone { what, caught_up })
}

/// How replaying a suggestion's patches went.
enum Replayed {
    /// Every patch fit and the design has no errors: the whole project
    /// after them.
    Clean(ProjectFiles),
    /// A patch no longer applies, or causes errors: why.
    NotApplied(String),
    /// Every patch fit, but the design still has errors: which.
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

/// Replays `patches` in order on `files` (the project in `folder`), on a
/// scratch copy in memory: nothing is written.
fn replay(
    folder: &Path,
    files: &ProjectFiles,
    patches: &[String],
) -> Result<Replayed, VersionsError> {
    let mut memory = MemoryFiles::new();
    for (relative, contents) in files.iter() {
        if let Ok(text) = std::str::from_utf8(contents) {
            memory.insert(folder.join(relative.as_path()), text);
        }
    }
    let mut base =
        EditBase::new(folder, &memory).map_err(|source| VersionsError::Load { source })?;
    let cancel = AtomicBool::new(false);
    let mut merged = files.clone();
    for patch in patches {
        let text = with_header(patch);
        let mut sources = SourceMap::new();
        let Ok(file) = sources.add(PATCH_NAME, text.as_str()) else {
            return Ok(Replayed::NotApplied(tr("the patch couldn't be read")));
        };
        for one in one_patch_each(file, &parse_patch_file(file, &text)) {
            let tried = match base.try_patch(PATCH_NAME, &one, &cancel) {
                Ok(tried) => tried,
                Err(crate::scratch::TryError::Load(source)) => {
                    return Err(VersionsError::Load { source });
                }
                Err(crate::scratch::TryError::Cancelled(_)) => {
                    return Ok(Replayed::NotApplied(tr("checking it was stopped")));
                }
            };
            match tried.verdict() {
                Verdict::Fits(_) => {}
                Verdict::Refused(_) | Verdict::Checking => {
                    let shown = tried.shown();
                    return Ok(Replayed::NotApplied(
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
                return Ok(Replayed::NotApplied(tr(
                    "the edit was refused, so nothing was written",
                )));
            }
        }
    }
    let opened = base.opened();
    let errors: Vec<_> = opened
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.severity == Severity::Error)
        .cloned()
        .collect();
    if errors.is_empty() {
        Ok(Replayed::Clean(merged))
    } else {
        Ok(Replayed::Errors(
            render_diagnostics(&opened.sources, &errors)
                .trim_end()
                .to_owned(),
        ))
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
