//! What an editor opens: a project file on disk, or a read-only file of
//! the standard library served from inside Stave (grammar, "Tooling and
//! editors": `stave-std:///joinery/dado.stave`).

use std::path::{Path, PathBuf};

/// The URI scheme of the standard library's read-only documents.
pub const STD_SCHEME: &str = "stave-std";

/// The folder the source map shows standard files under: `std/joinery/dado.stave`.
const STD_FOLDER: &str = "std/";

/// One document an editor can show.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Document {
    /// A `.stave` file on disk, by its absolute path.
    File(PathBuf),
    /// A file of the standard library, embedded in Stave. Read only.
    Std {
        /// One of `stave_modules::STD_MODULES`: `"joinery"`, `"hardware"`, `"tools"`.
        module: &'static str,
        /// The file name: `"dado.stave"`.
        file: &'static str,
    },
}

impl Document {
    /// The standard file `module/file`, when Stave has one by that name.
    pub fn std(module: &str, file: &str) -> Option<Document> {
        stave_modules::std_files()
            .find(|(m, f, _)| *m == module && *f == file)
            .map(|(module, file, _)| Document::Std { module, file })
    }

    /// Every file of the standard library, modules in prelude order, files
    /// by name.
    pub fn std_documents() -> impl Iterator<Item = Document> {
        stave_modules::std_files().map(|(module, file, _)| Document::Std { module, file })
    }

    /// The document a source map path names: `std/joinery/dado.stave` is
    /// the standard file, anything else a file on disk.
    pub fn from_source_path(path: &str) -> Document {
        path.strip_prefix(STD_FOLDER)
            .and_then(|rest| rest.split_once('/'))
            .and_then(|(module, file)| Document::std(module, file))
            .unwrap_or_else(|| Document::File(PathBuf::from(path)))
    }

    /// The path the source map gives it: the file's own path, or
    /// `std/joinery/dado.stave`.
    pub fn source_path(&self) -> String {
        match self {
            Document::File(path) => path.display().to_string(),
            Document::Std { module, file } => format!("{STD_FOLDER}{module}/{file}"),
        }
    }

    /// Its URI: `file:///…` or `stave-std:///joinery/dado.stave`.
    pub fn uri(&self) -> String {
        match self {
            Document::File(path) => format!("file://{}", path.display()),
            Document::Std { module, file } => format!("{STD_SCHEME}:///{module}/{file}"),
        }
    }

    /// The document a URI names.
    pub fn from_uri(uri: &str) -> Option<Document> {
        if let Some(path) = uri.strip_prefix("file://") {
            return Some(Document::File(PathBuf::from(path)));
        }
        let rest = uri.strip_prefix(STD_SCHEME)?.strip_prefix(":///")?;
        let (module, file) = rest.split_once('/')?;
        Document::std(module, file)
    }

    /// `true` for the standard library's files, which can't be edited.
    pub fn is_read_only(&self) -> bool {
        match self {
            Document::File(_) => false,
            Document::Std { .. } => true,
        }
    }

    /// The file name: `doors.stave`.
    pub fn file_name(&self) -> String {
        match self {
            Document::File(path) => path.file_name().map_or_else(
                || path.display().to_string(),
                |n| n.to_string_lossy().into_owned(),
            ),
            Document::Std { file, .. } => (*file).to_owned(),
        }
    }

    /// How a person reads its name in a project: `doors.stave`,
    /// `casework/frame_panel_door.stave` (relative to `folder` when it is
    /// inside it), or `std/joinery/dado.stave`.
    pub fn display(&self, folder: &Path) -> String {
        match self {
            Document::File(path) => path
                .strip_prefix(folder)
                .unwrap_or(path)
                .display()
                .to_string(),
            Document::Std { .. } => self.source_path(),
        }
    }

    /// The embedded text of a standard file.
    pub(crate) fn std_text(&self) -> Option<&'static str> {
        match self {
            Document::File(_) => None,
            Document::Std { module, file } => stave_modules::std_files()
                .find(|(m, f, _)| m == module && f == file)
                .map(|(_, _, text)| text),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_files_have_read_only_uris() {
        let dado = Document::std("joinery", "dado.stave").unwrap();
        assert_eq!(dado.uri(), "stave-std:///joinery/dado.stave");
        assert_eq!(Document::from_uri(&dado.uri()), Some(dado.clone()));
        assert_eq!(dado.source_path(), "std/joinery/dado.stave");
        assert_eq!(Document::from_source_path("std/joinery/dado.stave"), dado);
        assert!(dado.is_read_only());
        assert!(dado.std_text().is_some_and(|t| t.contains("kind dado")));
        assert_eq!(Document::std("joinery", "nothing.stave"), None);
    }

    #[test]
    fn project_files_are_paths() {
        let doors = Document::File(PathBuf::from("/p/cabinet/doors.stave"));
        assert_eq!(doors.uri(), "file:///p/cabinet/doors.stave");
        assert_eq!(Document::from_uri(&doors.uri()), Some(doors.clone()));
        assert_eq!(doors.display(Path::new("/p/cabinet")), "doors.stave");
        assert_eq!(doors.file_name(), "doors.stave");
        assert!(!doors.is_read_only());
        assert_eq!(Document::from_source_path("/p/cabinet/doors.stave"), doors);
    }
}
