//! Checking a project in the background: one run of the pipeline as a
//! value that can cross threads ([`run`]), and a folder watcher that calls
//! back once per burst of saves ([`Watcher`]).
//!
//! A viewer runs [`run`] on a worker thread whenever the session asks for
//! a check ([`crate::session::Event::CheckStarted`]) and hands the
//! [`Outcome`] back to [`crate::session::Session::finish_check`], which
//! replaces the old result whole.

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use stave_lang::Severity;
use stave_model::solved::SolvedDesign;
use stave_scene::SceneGraph;

use crate::Opened;
use crate::view::{self, Problem};

/// How long the folder must be quiet after a change before it is checked
/// again, so an editor's save (often several writes) checks once.
pub const RECHECK_DELAY: Duration = Duration::from_millis(250);

/// What one check found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Summary {
    /// How many errors.
    pub errors: usize,
    /// How many warnings (notes are not counted).
    pub warnings: usize,
    /// How many `.stave` files the project folder holds (the standard
    /// library is not counted).
    pub files: usize,
    /// How long the check took.
    pub elapsed: Duration,
}

impl Summary {
    /// Errors and warnings together: what the status pill counts.
    pub fn problems(&self) -> usize {
        self.errors.saturating_add(self.warnings)
    }
}

/// A project folder after one check: the pipeline's result, the scene a
/// viewer draws, and the problems as rows.
#[derive(Debug)]
pub struct Checked {
    /// The project folder.
    pub folder: PathBuf,
    /// Everything the pipeline produced.
    pub opened: Opened,
    /// The scene graph of the solved design, when it solved.
    pub scene: Option<SceneGraph>,
    /// Every diagnostic as a row, in `stave check` order.
    pub problems: Vec<Problem>,
    /// The counts.
    pub summary: Summary,
}

impl Checked {
    /// The solved design, when everything placed.
    pub fn design(&self) -> Option<&SolvedDesign> {
        self.opened.design.as_ref()
    }

    /// The solved design and its scene graph, when it solved.
    pub fn drawable(&self) -> Option<(&SolvedDesign, &SceneGraph)> {
        self.design().zip(self.scene.as_ref())
    }

    /// The title the folder declares for itself: its `project "..."` or
    /// `library "..."` title, read off the resolved model (or the solved
    /// design when the model is missing). `None` when it declares none or
    /// the title is blank.
    pub fn declared_title(&self) -> Option<&str> {
        let from_model = self.opened.model.as_ref().and_then(|model| {
            let module = model.defs.modules.get(model.root.index())?;
            module
                .project
                .as_ref()
                .map(|project| project.title.as_str())
                .or_else(|| {
                    module
                        .library
                        .as_ref()
                        .map(|library| library.title.as_str())
                })
        });
        from_model
            .or_else(|| self.design().map(|design| design.project.title.as_str()))
            .map(str::trim)
            .filter(|title| !title.is_empty())
    }
}

/// What a check produced.
#[derive(Debug)]
pub enum Outcome {
    /// The pipeline ran (whatever it found).
    Checked(Box<Checked>),
    /// The folder couldn't be read at all.
    CantOpen {
        /// Why, in plain words.
        message: String,
        /// How long it took to find out.
        elapsed: Duration,
    },
}

/// Checks a project folder: runs the whole pipeline and builds the scene.
/// Blocking; call it off the main thread.
pub fn run(folder: &Path) -> Outcome {
    let started = Instant::now();
    match crate::open(folder) {
        Ok(opened) => Outcome::Checked(Box::new(checked(folder, opened, started))),
        Err(error) => Outcome::CantOpen {
            message: error.to_string(),
            elapsed: started.elapsed(),
        },
    }
}

/// Checks a project folder as the editor shows it: open buffers with
/// unsaved changes are read in place of their files (grammar, "Modules in
/// an editor"). Blocking; call it off the main thread.
pub fn run_with(folder: &Path, overlay: &stave_ide::Overlay) -> Outcome {
    if overlay.is_empty() {
        return run(folder);
    }
    let started = Instant::now();
    match crate::open_with(folder, overlay) {
        Ok(opened) => Outcome::Checked(Box::new(checked(folder, opened, started))),
        Err(error) => Outcome::CantOpen {
            message: error.to_string(),
            elapsed: started.elapsed(),
        },
    }
}

/// Wraps a pipeline result as a check (for tests and in-memory projects).
pub fn checked(folder: &Path, opened: Opened, started: Instant) -> Checked {
    let scene = opened.design.as_ref().map(view::scene);
    let problems = view::problems(&opened, folder);
    let errors = problems
        .iter()
        .filter(|p| p.severity == Severity::Error)
        .count();
    let warnings = problems
        .iter()
        .filter(|p| p.severity == Severity::Warning)
        .count();
    let summary = Summary {
        errors,
        warnings,
        files: opened.root_files,
        elapsed: started.elapsed(),
    };
    Checked {
        folder: folder.to_path_buf(),
        opened,
        scene,
        problems,
        summary,
    }
}

/// Why a folder can't be watched.
#[derive(Debug, thiserror::Error)]
#[error("can't watch '{folder}' for changes: {source}")]
pub struct WatchError {
    /// The folder.
    pub folder: PathBuf,
    /// What the file watcher said.
    #[source]
    pub source: notify::Error,
}

/// Watches a project folder and calls back once the folder has been quiet
/// for [`RECHECK_DELAY`] after a `.stave` file changed. Dropping it stops
/// watching.
pub struct Watcher {
    /// Kept alive: dropping it stops the notifications, which ends the
    /// debouncing thread.
    _watcher: notify::RecommendedWatcher,
}

impl std::fmt::Debug for Watcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Watcher").finish_non_exhaustive()
    }
}

impl Watcher {
    /// Starts watching `folder` (and everything inside it). `changed` is
    /// called on a background thread, once per burst of changes.
    pub fn start(
        folder: &Path,
        changed: impl Fn() + Send + 'static,
    ) -> Result<Watcher, WatchError> {
        use notify::Watcher as _;
        let (sender, receiver) = mpsc::channel::<()>();
        let mut watcher =
            notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
                let Ok(event) = event else { return };
                let relevant = !matches!(event.kind, notify::EventKind::Access(_))
                    && event.paths.iter().any(|path| is_source(path));
                if relevant {
                    // The debouncing thread may be gone; then nobody needs
                    // to know.
                    let _ = sender.send(());
                }
            })
            .map_err(|source| WatchError {
                folder: folder.to_path_buf(),
                source,
            })?;
        watcher
            .watch(folder, notify::RecursiveMode::Recursive)
            .map_err(|source| WatchError {
                folder: folder.to_path_buf(),
                source,
            })?;
        std::thread::Builder::new()
            .name("stave-watch".to_owned())
            .spawn(move || debounce(&receiver, RECHECK_DELAY, changed))
            .map_err(|error| WatchError {
                folder: folder.to_path_buf(),
                source: notify::Error::io(error),
            })?;
        Ok(Watcher { _watcher: watcher })
    }
}

/// Calls `changed` once after each burst of messages, when `quiet` has
/// passed without another. Returns when the sender is dropped.
pub fn debounce(receiver: &mpsc::Receiver<()>, quiet: Duration, changed: impl Fn()) {
    while receiver.recv().is_ok() {
        loop {
            match receiver.recv_timeout(quiet) {
                Ok(()) => {}
                Err(mpsc::RecvTimeoutError::Timeout) => break,
                Err(mpsc::RecvTimeoutError::Disconnected) => return,
            }
        }
        changed();
    }
}

/// A `.stave` file outside hidden and build folders.
pub fn is_source(path: &Path) -> bool {
    path.extension()
        .is_some_and(|extension| extension == "stave")
        && !path.components().any(|part| {
            let name = part.as_os_str().to_string_lossy();
            name == "target" || (name.starts_with('.') && name.len() > 1 && name != "..")
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn only_stave_files_outside_hidden_folders_count() {
        assert!(is_source(Path::new("/p/cabinet.stave")));
        assert!(is_source(Path::new("../p/doors.stave")));
        assert!(!is_source(Path::new("/p/notes.txt")));
        assert!(!is_source(Path::new("/p/.git/x.stave")));
        assert!(!is_source(Path::new("/p/target/x.stave")));
    }

    #[test]
    fn a_burst_of_changes_is_one_call() {
        let (sender, receiver) = mpsc::channel();
        let calls = Arc::new(AtomicUsize::new(0));
        let counted = calls.clone();
        let thread = std::thread::spawn(move || {
            debounce(&receiver, Duration::from_millis(40), move || {
                counted.fetch_add(1, Ordering::SeqCst);
            });
        });
        for _ in 0..5 {
            sender.send(()).unwrap();
        }
        std::thread::sleep(Duration::from_millis(150));
        sender.send(()).unwrap();
        std::thread::sleep(Duration::from_millis(150));
        drop(sender);
        thread.join().unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }
}
