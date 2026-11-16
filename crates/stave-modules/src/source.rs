//! Where module files come from: the disk, or memory for tests and editors.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

/// Supplies folders and files to the loader, so tests and editors can load
/// modules from memory and the command line can load them from disk.
pub trait FileSource {
    /// Every `.stave` file directly inside `folder` (not in subfolders), in
    /// any order. An error if the folder can't be listed.
    fn list_stave_files(&self, folder: &Path) -> io::Result<Vec<PathBuf>>;

    /// The text of the file at `path`.
    fn read(&self, path: &Path) -> io::Result<String>;

    /// The identity of `folder`: two paths naming the same folder give the
    /// same result. An error if the folder doesn't exist.
    fn canonical(&self, folder: &Path) -> io::Result<PathBuf>;
}

/// Files on disk. Folder identity is [`fs::canonicalize`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DiskFiles;

impl FileSource for DiskFiles {
    fn list_stave_files(&self, folder: &Path) -> io::Result<Vec<PathBuf>> {
        let mut files = Vec::new();
        for entry in fs::read_dir(folder)? {
            let path = entry?.path();
            if is_stave_path(&path) && fs::metadata(&path)?.is_file() {
                files.push(path);
            }
        }
        Ok(files)
    }

    fn read(&self, path: &Path) -> io::Result<String> {
        fs::read_to_string(path)
    }

    fn canonical(&self, folder: &Path) -> io::Result<PathBuf> {
        fs::canonicalize(folder)
    }
}

/// Files held in memory, keyed by path. Paths are normalised lexically
/// ([`normalize`]) when added and when looked up, and a folder exists when
/// some file lies somewhere below it.
///
/// ```
/// use std::path::Path;
/// use stave_modules::{MemoryFiles, ModuleSet};
///
/// let mut files = MemoryFiles::new();
/// files.insert("shop/project.stave", "stave 1\n\nproject \"Shop\" {\n  units: mm\n}\n");
/// let set = ModuleSet::load_with(Path::new("shop"), &files).unwrap();
/// assert_eq!(set.modules.len(), 4); // the project and the three standard modules
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MemoryFiles {
    files: BTreeMap<PathBuf, String>,
}

impl MemoryFiles {
    /// No files.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds or replaces the file at `path`.
    pub fn insert(&mut self, path: impl AsRef<Path>, text: impl Into<String>) {
        self.files.insert(normalize(path.as_ref()), text.into());
    }

    /// Adds or replaces the file at `path`, returning `self` for chaining.
    #[must_use]
    pub fn with(mut self, path: impl AsRef<Path>, text: impl Into<String>) -> Self {
        self.insert(path, text);
        self
    }

    /// `true` if some file lies below `folder`.
    fn folder_exists(&self, folder: &Path) -> bool {
        self.files
            .keys()
            .any(|path| path != folder && path.starts_with(folder))
    }
}

impl FromIterator<(PathBuf, String)> for MemoryFiles {
    fn from_iter<I: IntoIterator<Item = (PathBuf, String)>>(iter: I) -> Self {
        let mut files = MemoryFiles::new();
        for (path, text) in iter {
            files.insert(path, text);
        }
        files
    }
}

impl FileSource for MemoryFiles {
    fn list_stave_files(&self, folder: &Path) -> io::Result<Vec<PathBuf>> {
        let folder = normalize(folder);
        if self.files.contains_key(&folder) {
            return Err(io::Error::from(io::ErrorKind::NotADirectory));
        }
        if !self.folder_exists(&folder) {
            return Err(io::Error::from(io::ErrorKind::NotFound));
        }
        Ok(self
            .files
            .keys()
            .filter(|path| path.parent() == Some(folder.as_path()) && is_stave_path(path))
            .cloned()
            .collect())
    }

    fn read(&self, path: &Path) -> io::Result<String> {
        self.files
            .get(&normalize(path))
            .cloned()
            .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))
    }

    fn canonical(&self, folder: &Path) -> io::Result<PathBuf> {
        let folder = normalize(folder);
        if self.files.contains_key(&folder) || self.folder_exists(&folder) {
            Ok(folder)
        } else {
            Err(io::Error::from(io::ErrorKind::NotFound))
        }
    }
}

/// `true` if the path ends in `.stave`.
pub(crate) fn is_stave_path(path: &Path) -> bool {
    path.extension()
        .is_some_and(|extension| extension == "stave")
}

/// Removes `.` segments and resolves `..` against the segment before it,
/// without touching the file system. A leading `..` that has nothing to
/// cancel is kept, so `../x` stays `../x`; `/..` stays `/`.
///
/// ```
/// use std::path::Path;
/// use stave_modules::normalize;
///
/// assert_eq!(
///     normalize(Path::new("examples/joint_samples/../japanese_joinery")),
///     Path::new("examples/japanese_joinery"),
/// );
/// assert_eq!(normalize(Path::new("./casework/")), Path::new("casework"));
/// ```
pub fn normalize(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => match normalized.components().next_back() {
                Some(Component::Normal(_)) => {
                    normalized.pop();
                }
                Some(Component::RootDir | Component::Prefix(_)) => {}
                Some(Component::ParentDir | Component::CurDir) | None => {
                    normalized.push(Component::ParentDir);
                }
            },
            Component::Normal(_) | Component::RootDir | Component::Prefix(_) => {
                normalized.push(component);
            }
        }
    }
    normalized
}
