//! The project folder: whether versions are on, and which files count.

use std::path::{Path, PathBuf};

use crate::error::HistoryError;
use crate::paths::{self, ProjectFiles, RelativePath};

/// Where the repository lives, relative to the project folder.
pub(crate) const HISTORY_DIRECTORY: &str = ".stave/history";

/// Whether versions are on for a project folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VersionsStatus {
    /// Versions are off; they can be turned on.
    Off,
    /// Versions are on.
    On,
    /// The folder sits inside a bigger repository, so versions can't be
    /// turned on (the project can be copied to a folder of its own).
    Nested {
        /// The root of the repository around the folder.
        repository_root: PathBuf,
    },
}

/// The folder, made absolute and free of symbolic links.
pub(crate) fn canonical(folder: &Path) -> Result<PathBuf, HistoryError> {
    std::fs::canonicalize(folder).map_err(|error| HistoryError::io(folder, error))
}

/// Whether versions are on for `folder` (already canonical).
pub(crate) fn status(folder: &Path) -> VersionsStatus {
    if folder.join(HISTORY_DIRECTORY).join("repo").is_dir() {
        return VersionsStatus::On;
    }
    match folder
        .ancestors()
        .skip(1)
        .find(|ancestor| ancestor.join(".git").exists())
    {
        Some(root) => VersionsStatus::Nested {
            repository_root: root.to_owned(),
        },
        None => VersionsStatus::Off,
    }
}

/// Every file in `folder` that counts: all but `.stave/history`, any `.git` and
/// what `.gitignore` excludes. Symbolic links are skipped, so nothing
/// outside the folder is read.
pub(crate) fn read(folder: &Path) -> Result<ProjectFiles, HistoryError> {
    let root = folder.to_owned();
    let walk = ignore::WalkBuilder::new(folder)
        .hidden(false)
        .parents(false)
        .ignore(false)
        .git_global(false)
        .git_exclude(false)
        .git_ignore(true)
        .require_git(false)
        .follow_links(false)
        // `.git` folders (at any depth, in any case) and `.stave/history`
        // hold repositories, not project files.
        .filter_entry(move |entry| {
            entry.path().strip_prefix(&root).is_ok_and(|relative| {
                !paths::is_reserved(relative.iter().filter_map(|part| part.to_str()))
            })
        })
        .build();
    let mut files = ProjectFiles::new();
    for entry in walk {
        let entry = entry
            .map_err(|error| HistoryError::io(folder, std::io::Error::other(error.to_string())))?;
        let is_file = entry
            .file_type()
            .is_some_and(|file_type| file_type.is_file());
        if !is_file {
            continue;
        }
        let Ok(relative) = entry.path().strip_prefix(folder) else {
            continue;
        };
        let relative = RelativePath::new(relative)?;
        let contents =
            std::fs::read(entry.path()).map_err(|error| HistoryError::io(entry.path(), error))?;
        files.insert(relative, contents);
    }
    Ok(files)
}
