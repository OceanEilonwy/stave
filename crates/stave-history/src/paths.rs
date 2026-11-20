//! Paths inside a project and the files of a project at one moment.

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Component, Path, PathBuf};

use thiserror::Error;

/// Why a path is not a path inside the project.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PathError {
    /// The path is empty or names only the project folder itself.
    #[error("the path is empty; name a file inside the project")]
    Empty,
    /// The path starts at the root of the disk.
    #[error("\"{0}\" starts at the root of the disk; use a path inside the project folder")]
    Absolute(PathBuf),
    /// The path climbs out of its folder with `..`.
    #[error("\"{0}\" climbs out of the project folder with \"..\"; use a path inside it")]
    Parent(PathBuf),
    /// The path holds letters that are not text.
    #[error("\"{0}\" holds letters Stave cannot read; rename the file")]
    NotText(PathBuf),
}

/// A path to a file inside a project folder, relative to the folder.
///
/// It is never empty, never absolute and never climbs out with `..`, so a
/// `RelativePath` joined to the project folder always stays inside it (a
/// symbolic link aside: those are never read or written, see
/// [`History::read_folder`](crate::History::read_folder)).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RelativePath(PathBuf);

impl RelativePath {
    /// Checks `path` and makes it a project path. `.` components are dropped.
    ///
    /// # Errors
    ///
    /// [`PathError`] when the path is empty, absolute, climbs out with `..`
    /// or is not valid UTF-8.
    pub fn new(path: &Path) -> Result<Self, PathError> {
        let mut clean = PathBuf::new();
        for component in path.components() {
            match component {
                Component::Normal(part) => {
                    if part.to_str().is_none() {
                        return Err(PathError::NotText(path.to_owned()));
                    }
                    clean.push(part);
                }
                Component::CurDir => {}
                Component::ParentDir => return Err(PathError::Parent(path.to_owned())),
                Component::RootDir | Component::Prefix(_) => {
                    return Err(PathError::Absolute(path.to_owned()));
                }
            }
        }
        if clean.as_os_str().is_empty() {
            return Err(PathError::Empty);
        }
        Ok(Self(clean))
    }

    /// The path, relative to the project folder.
    pub fn as_path(&self) -> &Path {
        &self.0
    }

    /// The path with `/` between its parts, as the repository stores it.
    pub(crate) fn to_slash_string(&self) -> String {
        let mut text = String::new();
        for part in self.0.iter() {
            if !text.is_empty() {
                text.push('/');
            }
            // Checked in `new`: every part is UTF-8.
            text.push_str(&part.to_string_lossy());
        }
        text
    }
}

impl fmt::Display for RelativePath {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.0.display())
    }
}

/// Every file of a project at one moment: path to contents, sorted by path.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProjectFiles(BTreeMap<RelativePath, Vec<u8>>);

impl ProjectFiles {
    /// No files.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the contents of `path`, returning what it held before.
    pub fn insert(&mut self, path: RelativePath, contents: Vec<u8>) -> Option<Vec<u8>> {
        self.0.insert(path, contents)
    }

    /// Removes `path`, returning what it held.
    pub fn remove(&mut self, path: &RelativePath) -> Option<Vec<u8>> {
        self.0.remove(path)
    }

    /// The contents of `path`, if the project has it.
    pub fn get(&self, path: &RelativePath) -> Option<&[u8]> {
        self.0.get(path).map(Vec::as_slice)
    }

    /// Whether the project has `path`.
    pub fn contains(&self, path: &RelativePath) -> bool {
        self.0.contains_key(path)
    }

    /// The paths, sorted.
    pub fn paths(&self) -> impl Iterator<Item = &RelativePath> {
        self.0.keys()
    }

    /// Each path with its contents, sorted by path.
    pub fn iter(&self) -> impl Iterator<Item = (&RelativePath, &[u8])> {
        self.0
            .iter()
            .map(|(path, contents)| (path, contents.as_slice()))
    }

    /// How many files there are.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether there are no files.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl FromIterator<(RelativePath, Vec<u8>)> for ProjectFiles {
    fn from_iter<I: IntoIterator<Item = (RelativePath, Vec<u8>)>>(iter: I) -> Self {
        Self(iter.into_iter().collect())
    }
}

impl IntoIterator for ProjectFiles {
    type Item = (RelativePath, Vec<u8>);
    type IntoIter = std::collections::btree_map::IntoIter<RelativePath, Vec<u8>>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

impl<'a> IntoIterator for &'a ProjectFiles {
    type Item = (&'a RelativePath, &'a Vec<u8>);
    type IntoIter = std::collections::btree_map::Iter<'a, RelativePath, Vec<u8>>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_directory_parts_are_dropped() {
        let path = RelativePath::new(Path::new("./drawers/./drawer.stave")).expect("valid");
        assert_eq!(path.as_path(), Path::new("drawers/drawer.stave"));
        assert_eq!(path.to_slash_string(), "drawers/drawer.stave");
    }

    #[test]
    fn empty_and_dot_are_refused() {
        assert_eq!(RelativePath::new(Path::new("")), Err(PathError::Empty));
        assert_eq!(RelativePath::new(Path::new(".")), Err(PathError::Empty));
    }
}
