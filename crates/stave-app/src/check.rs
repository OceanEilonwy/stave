//! Checking a project in the background: one run of the pipeline as a
//! value that can cross threads ([`run`]), and a folder watcher that calls
//! back once per burst of saves ([`Watcher`]).
//!
//! A viewer runs [`run`] on a worker thread whenever the session asks for
//! a check ([`crate::session::Event::CheckStarted`]) and hands the
//! [`Outcome`] back to [`crate::session::Session::finish_check`], which
//! replaces the old result whole.
//!
//! The session keeps a [`MeshCache`] ([`crate::session::Session::meshes`])
//! that a viewer hands to [`run_cached`]: each check then makes afresh only
//! the part meshes whose cuts changed since the last one, and the rest
//! come from the cache.
//!
//! With the environment variable `STAVE_PROFILE` set (to anything but `0`
//! or nothing), each check prints to standard error how long reading and
//! solving, building the scene (with how many part meshes it made and
//! reused) and listing the problems took.

use std::path::{Path, PathBuf};
use std::sync::{OnceLock, mpsc};
use std::time::{Duration, Instant};

use stave_lang::Severity;
use stave_model::solved::SolvedDesign;
use stave_scene::{MeshCache, SceneGraph};

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
    run_cached(folder, &stave_ide::Overlay::new(), &MeshCache::new())
}

/// Checks a project folder as the editor shows it ([`run_with`]), taking
/// the part meshes whose cuts haven't changed from `meshes` and leaving
/// in it the meshes of this check. Blocking; call it off the main thread.
pub fn run_cached(folder: &Path, overlay: &stave_ide::Overlay, meshes: &MeshCache) -> Outcome {
    let started = Instant::now();
    let opened = if overlay.is_empty() {
        crate::open(folder)
    } else {
        crate::open_with(folder, overlay)
    };
    match opened {
        Ok(opened) => Outcome::Checked(Box::new(checked_with(folder, opened, started, meshes))),
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
    run_cached(folder, overlay, &MeshCache::new())
}

/// Wraps a pipeline result as a check (for tests and in-memory projects).
pub fn checked(folder: &Path, opened: Opened, started: Instant) -> Checked {
    checked_with(folder, opened, started, &MeshCache::new())
}

/// Whether `STAVE_PROFILE` asks for timings. Read once.
fn profiling() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| {
        std::env::var_os("STAVE_PROFILE").is_some_and(|value| !value.is_empty() && value != "0")
    })
}

/// Milliseconds, for a log line.
fn milliseconds(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

/// [`checked`], taking unchanged part meshes from `meshes` and leaving
/// this check's in it.
pub fn checked_with(
    folder: &Path,
    opened: Opened,
    started: Instant,
    meshes: &MeshCache,
) -> Checked {
    let solved = started.elapsed();
    let scene_started = Instant::now();
    let scene = opened
        .design
        .as_ref()
        .map(|design| view::scene_with(design, meshes));
    let scene_time = scene_started.elapsed();
    let problems_started = Instant::now();
    let problems = view::problems(&opened, folder);
    if profiling() {
        let usage = meshes.last_use();
        eprintln!(
            "stave-profile: check: open and solve {:.2} ms, scene {:.2} ms \
             ({} part meshes made, {} reused, {} held), problems {:.2} ms",
            milliseconds(solved),
            milliseconds(scene_time),
            usage.made,
            usage.reused,
            usage.held,
            milliseconds(problems_started.elapsed()),
        );
    }
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
