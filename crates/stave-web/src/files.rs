//! A project's files as the page sends them: a folder, and every file's
//! text keyed by its path.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use stave_modules::{MemoryFiles, normalize};

/// Why the files the page sent can't be read.
#[derive(Debug, thiserror::Error)]
pub enum FilesError {
    /// The files aren't a JSON object of paths to texts.
    #[error("the project's files must be a JSON object of paths to texts: {0}")]
    Json(#[from] serde_json::Error),
    /// The project folder isn't an absolute path.
    #[error("the project folder must be an absolute path, not '{0}'")]
    FolderNotAbsolute(String),
    /// A file has no path.
    #[error("a file has an empty path")]
    EmptyPath,
}

/// A project's files held in memory: the project folder, and every file
/// with its text. A file's path relative to the folder is taken inside it
/// (`box.stave` is `/p/box.stave`); an absolute path is kept as it is, so
/// a library folder beside the project can come along too. Nothing is
/// ever read from or written to a disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Files {
    folder: PathBuf,
    files: BTreeMap<PathBuf, String>,
}

impl Files {
    /// The files `files` (path to text) of the project in `folder`, an
    /// absolute path.
    pub fn new(
        folder: impl AsRef<Path>,
        files: BTreeMap<String, String>,
    ) -> Result<Files, FilesError> {
        let folder = folder.as_ref();
        if !folder.has_root() {
            return Err(FilesError::FolderNotAbsolute(folder.display().to_string()));
        }
        let folder = normalize(folder);
        let mut kept = BTreeMap::new();
        for (path, text) in files {
            if path.is_empty() {
                return Err(FilesError::EmptyPath);
            }
            kept.insert(normalize(&folder.join(path)), text);
        }
        Ok(Files {
            folder,
            files: kept,
        })
    }

    /// [`Files::new`] from the JSON object the page sends:
    /// `{"box.stave": "stave 1\n…"}`.
    pub fn from_json(folder: impl AsRef<Path>, json: &str) -> Result<Files, FilesError> {
        Files::new(folder, serde_json::from_str(json)?)
    }

    /// The project folder.
    pub fn folder(&self) -> &Path {
        &self.folder
    }

    /// How many files there are.
    pub fn count(&self) -> usize {
        self.files.len()
    }

    /// These files with `changes`' files put in their place: an agent's
    /// suggestion over the project as it is. A file only in `changes` is
    /// added. `changes`' folder is not used: its paths were already made
    /// whole against its own folder.
    #[must_use]
    pub fn with_changes(&self, changes: &Files) -> Files {
        let mut files = self.files.clone();
        for (path, text) in &changes.files {
            files.insert(path.clone(), text.clone());
        }
        Files {
            folder: self.folder.clone(),
            files,
        }
    }

    /// The files as the loader reads them.
    pub fn memory(&self) -> MemoryFiles {
        self.files
            .iter()
            .map(|(path, text)| (path.clone(), text.clone()))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_paths_go_inside_the_folder() {
        let files = Files::from_json(
            "/p",
            r#"{"box.stave": "a", "/lib/shelf.stave": "b", "./parts/../lid.stave": "c"}"#,
        )
        .unwrap();
        let paths: Vec<_> = files.files.keys().cloned().collect();
        assert_eq!(
            paths,
            [
                PathBuf::from("/lib/shelf.stave"),
                PathBuf::from("/p/box.stave"),
                PathBuf::from("/p/lid.stave"),
            ]
        );
    }

    #[test]
    fn a_relative_folder_is_refused() {
        assert!(matches!(
            Files::from_json("p", "{}"),
            Err(FilesError::FolderNotAbsolute(_))
        ));
    }

    #[test]
    fn json_that_is_not_paths_to_texts_is_refused() {
        assert!(matches!(
            Files::from_json("/p", r#"{"box.stave": 3}"#),
            Err(FilesError::Json(_))
        ));
        assert!(matches!(
            Files::from_json("/p", r#"{"": "x"}"#),
            Err(FilesError::EmptyPath)
        ));
    }

    #[test]
    fn changes_replace_and_add_files() {
        let base = Files::from_json("/p", r#"{"a.stave": "old", "b.stave": "kept"}"#).unwrap();
        let changes = Files::from_json("/p", r#"{"a.stave": "new", "c.stave": "added"}"#).unwrap();
        let merged = base.with_changes(&changes);
        assert_eq!(merged.count(), 3);
        assert_eq!(
            merged
                .files
                .get(Path::new("/p/a.stave"))
                .map(String::as_str),
            Some("new")
        );
    }
}
