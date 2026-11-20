//! The agent's line: the files an agent's changes are tried on and
//! recorded against, read from the project's versions rather than from
//! the folder the person has open.
//!
//! ```text
//! History ──main, or a suggestion──▶ ProjectFiles ──LineFiles──▶ EditBase ──replay──▶ Replay
//! ```
//!
//! An agent never works in the project folder: with versions on, a patch
//! is tried on main's latest files (or on one of the suggestions'), held
//! in memory as [`LineFiles`], so the person's unsaved work and the
//! agent's attempts never meet. [`replay`] tries a list of patches in
//! order on such files, the way `apply_suggestion` re-tries a suggestion
//! on main as it is now.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use stave_app::scratch::{EditBase, one_patch_each};
use stave_history::{
    Actor, ActorKind, History, HistoryError, ProjectFiles, RelativePath, RevisionId, SuggestionId,
    VersionsStatus,
};
use stave_lang::{SourceMap, parse_patch_file, render_diagnostics};
use stave_modules::{DiskFiles, FileSource, MemoryFiles};

use crate::tools::changing::{PATCH_NAME, title, with_header};
use crate::tools::projects::load_error;
use crate::{Agent, McpError};

/// The person, as Stave records the folder's files when versions are
/// turned on for a project that already has them.
pub(crate) fn person() -> Actor {
    Actor {
        kind: ActorKind::User,
        name: "You".to_owned(),
        id: String::new(),
    }
}

/// The agent as the project's versions record it.
pub(crate) fn actor(agent: &Agent) -> Actor {
    Actor {
        kind: ActorKind::Agent,
        name: agent.name.clone(),
        id: agent.id.clone(),
    }
}

/// The versions of the project in `folder`.
///
/// # Errors
///
/// [`McpError::VersionsOff`] when they are off, with how to turn them on.
pub(crate) fn history(folder: &Path) -> Result<History, McpError> {
    match History::open(folder) {
        Ok(history) => Ok(history),
        Err(HistoryError::Off) => Err(McpError::VersionsOff),
        Err(error) => Err(error.into()),
    }
}

/// Whether versions are on for `folder`.
///
/// # Errors
///
/// [`McpError::Nested`] when the folder sits inside a bigger repository.
pub(crate) fn versions_on(folder: &Path) -> Result<bool, McpError> {
    match History::status(folder)? {
        VersionsStatus::On => Ok(true),
        VersionsStatus::Off => Ok(false),
        VersionsStatus::Nested { repository_root } => Err(McpError::Nested {
            folder: folder.to_path_buf(),
            repository_root,
        }),
    }
}

/// Turns versions on for `folder` and, when main holds nothing yet,
/// records the folder's files as its first revision by `actor`.
pub(crate) fn start(folder: &Path, actor: &Actor, message: &str) -> Result<History, McpError> {
    let mut history = match History::start(folder) {
        Ok(history) => history,
        Err(HistoryError::Nested { repository_root }) => {
            return Err(McpError::Nested {
                folder: folder.to_path_buf(),
                repository_root,
            });
        }
        Err(error) => return Err(error.into()),
    };
    if history.versions(1)?.is_empty() {
        let files = history.read_folder()?;
        history.save(&files, actor, message)?;
    }
    Ok(history)
}

/// The files of the agent's line, with the revision they are read from:
/// suggestion `onto`'s latest, or main's latest.
pub(crate) fn line_files(
    history: &History,
    onto: Option<&SuggestionId>,
) -> Result<(RevisionId, ProjectFiles), McpError> {
    let revision = match onto {
        Some(id) => {
            history
                .suggestions()?
                .into_iter()
                .find(|suggestion| &suggestion.id == id)
                .ok_or_else(|| HistoryError::UnknownSuggestion(id.clone()))?
                .tip
        }
        None => history.main()?,
    };
    let files = history.files_at(&revision)?;
    Ok((revision, files))
}

/// A project's files held in memory, as the loader reads them: a path
/// inside the project folder is read from the files, and only from them
/// (a file they don't hold isn't there); a path outside it, such as a
/// library the project imports from a folder beside it, from disk.
#[derive(Debug)]
pub(crate) struct LineFiles {
    folder: PathBuf,
    files: MemoryFiles,
}

impl LineFiles {
    /// The files of `files` as the project in `folder` (a full path with
    /// no symbolic links). Files that aren't text can't be `.stave` files
    /// and are left out.
    pub(crate) fn new(folder: &Path, files: &ProjectFiles) -> Self {
        let mut memory = MemoryFiles::new();
        for (path, contents) in files.iter() {
            if let Ok(text) = std::str::from_utf8(contents) {
                memory.insert(folder.join(path.as_path()), text);
            }
        }
        Self {
            folder: folder.to_path_buf(),
            files: memory,
        }
    }

    fn inside(&self, path: &Path) -> bool {
        stave_modules::normalize(path).starts_with(&self.folder)
    }
}

impl FileSource for LineFiles {
    fn list_stave_files(&self, folder: &Path) -> io::Result<Vec<PathBuf>> {
        if self.inside(folder) {
            self.files.list_stave_files(folder)
        } else {
            DiskFiles.list_stave_files(folder)
        }
    }

    fn read(&self, path: &Path) -> io::Result<String> {
        if self.inside(path) {
            self.files.read(path)
        } else {
            DiskFiles.read(path)
        }
    }

    fn canonical(&self, folder: &Path) -> io::Result<PathBuf> {
        if self.inside(folder) {
            self.files.canonical(folder)
        } else {
            DiskFiles.canonical(folder)
        }
    }
}

/// What came of trying a list of patches in order.
#[derive(Debug)]
pub(crate) enum Replay {
    /// Every patch fit: the project after the last one.
    Fits {
        /// Every file of the project afterwards.
        files: ProjectFiles,
        /// How many errors the design has afterwards (ones it had before
        /// included).
        errors: usize,
        /// Those errors and the warnings, as `stave check` prints them.
        problems: String,
    },
    /// A patch was refused.
    Refused {
        /// Which, as `patch "Add a shelf"`.
        title: String,
        /// Why, as `stave check` prints it.
        problems: String,
    },
}

/// Tries `patches` in order on `files`, the project in `folder`, each on
/// top of the ones before it.
///
/// # Errors
///
/// When the files can't be loaded as a project, or a patch would write a
/// file outside the project folder.
pub(crate) fn replay(
    folder: &Path,
    files: &ProjectFiles,
    patches: &[String],
) -> Result<Replay, McpError> {
    let source = LineFiles::new(folder, files);
    let mut base = EditBase::new(folder, &source).map_err(|error| load_error(folder, error))?;
    let mut result = files.clone();
    let cancel = AtomicBool::new(false);
    for text in patches {
        let text = with_header(text);
        let mut sources = SourceMap::new();
        let file = sources.add(PATCH_NAME, text.as_str())?;
        for patch in one_patch_each(file, &parse_patch_file(file, &text)) {
            let tried = base.try_patch(PATCH_NAME, &patch, &cancel)?;
            if !tried.fits() {
                return Ok(Replay::Refused {
                    title: title(&patch),
                    problems: render_diagnostics(tried.sources(), &tried.shown()),
                });
            }
            for changed in tried.changed() {
                let path = project_file(folder, &changed.path)?;
                result.insert(path, changed.text.clone().into_bytes());
            }
            base.advance(tried)?;
        }
    }
    let opened = base.opened();
    Ok(Replay::Fits {
        files: result,
        errors: opened.error_count(),
        problems: render_diagnostics(&opened.sources, &opened.diagnostics),
    })
}

/// `path`, a file a patch writes, as a path inside the project.
fn project_file(folder: &Path, path: &Path) -> Result<RelativePath, McpError> {
    let outside = || McpError::OutsideProject(path.display().to_string());
    let inside = stave_modules::normalize(path)
        .strip_prefix(folder)
        .map_err(|_| outside())?
        .to_path_buf();
    RelativePath::new(&inside).map_err(|_| outside())
}
