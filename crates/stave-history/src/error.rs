//! What can go wrong with a project's versions.

use std::path::PathBuf;

use thiserror::Error;

use crate::paths::{PathError, RelativePath};
use crate::{RevisionId, SuggestionId};

/// An error from the revision system, worded for the person using Stave.
#[derive(Debug, Error)]
pub enum HistoryError {
    /// The project folder sits inside a bigger repository.
    #[error(
        "this project is inside another project's history (at {}); versions can't be turned on here, but a copy of the project in a folder of its own can have them",
        repository_root.display()
    )]
    Nested {
        /// The root of the repository around the folder.
        repository_root: PathBuf,
    },
    /// Versions are not turned on for this project.
    #[error("versions are off for this project; turn them on first")]
    Off,
    /// A file or folder could not be read or written.
    #[error("couldn't use {}: {source}", path.display())]
    Io {
        /// The file or folder.
        path: PathBuf,
        /// What the system said.
        #[source]
        source: std::io::Error,
    },
    /// A path is not a path inside the project.
    #[error(transparent)]
    Path(#[from] PathError),
    /// The stored versions could not be read or written.
    #[error("the project's versions couldn't be read or written: {0}")]
    Store(#[source] Box<dyn std::error::Error + Send + Sync>),
    /// No revision has this id.
    #[error("there is no version {0}")]
    UnknownRevision(RevisionId),
    /// No suggestion has this id (it may have been applied or discarded).
    #[error("there is no suggestion {0}; it may have been applied or discarded already")]
    UnknownSuggestion(SuggestionId),
    /// A suggestion still holds clashing changes.
    #[error(
        "suggestion {id} has clashing changes in {}; resolve them before applying it",
        list(files)
    )]
    SuggestionConflicted {
        /// The suggestion.
        id: SuggestionId,
        /// The files with clashes.
        files: Vec<RelativePath>,
    },
    /// Main moved while the files were being prepared, and the two clash.
    #[error(
        "the project changed while this was being prepared, and the changes clash in {}; try again on the project as it is now",
        list(files)
    )]
    MainMoved {
        /// The files where the changes clash.
        files: Vec<RelativePath>,
    },
    /// A suggestion would change the project's settings in `.stave/`, which
    /// only the person using Stave may change.
    #[error(
        "a suggestion can't change {path}: the project's settings are changed only by the person using Stave"
    )]
    SettingsProtected {
        /// The settings file the suggestion changed.
        path: RelativePath,
    },
    /// The folder's git branch moved while Stave was moving main.
    #[error("the project's git branch {branch} changed while this was being recorded; try again")]
    BranchMoved {
        /// The branch, as `main`.
        branch: String,
    },
    /// There is nothing to undo.
    #[error("there is nothing to undo")]
    NothingToUndo,
    /// Undo would throw away someone else's later change.
    #[error("can't undo \"{undone}\": {later} came after it and builds on it")]
    UndoBlocked {
        /// What would have been undone.
        undone: String,
        /// The later change in the way, described.
        later: String,
    },
}

fn list(files: &[RelativePath]) -> String {
    files
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

impl HistoryError {
    /// Wraps an error from the store.
    pub(crate) fn store(error: impl std::error::Error + Send + Sync + 'static) -> Self {
        Self::Store(Box::new(error))
    }

    /// Wraps an I/O error with the path it was about.
    pub(crate) fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }
}
