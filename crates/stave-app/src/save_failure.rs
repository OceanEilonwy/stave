//! What to tell people when a file can't be saved: a heading that names
//! the file and a plain sentence on what went wrong and what to do, in
//! place of the operating system's error text. The interface shows it in
//! a dialog; work is never lost quietly.

use std::io;
use std::path::Path;

/// A failed save, put into words for people.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveFailure {
    /// "Couldn't Save doors.stave".
    pub heading: String,
    /// What went wrong and what to do, in plain words.
    pub body: String,
}

/// What a failed write means, by why the system refused it.
pub fn advice(error: &io::Error) -> &'static str {
    match error.kind() {
        io::ErrorKind::PermissionDenied | io::ErrorKind::ReadOnlyFilesystem => {
            "The folder can't be written to. Save a copy somewhere else, or check that the folder isn't read-only."
        }
        io::ErrorKind::StorageFull | io::ErrorKind::QuotaExceeded => {
            "The disk is full. Make some room and try again, or save a copy on another disk."
        }
        io::ErrorKind::NotFound => {
            "The folder the file was in isn't there any more (it may have been moved, renamed or unplugged). Save a copy somewhere else."
        }
        _ => "Something stopped the file being written. Try again, or save a copy somewhere else.",
    }
}

/// Puts a failed save of a file the editor has open into words. The
/// changes are still in the editor, and the body says so.
pub fn describe(error: &stave_ide::SaveError, folder: &Path) -> SaveFailure {
    match error {
        stave_ide::SaveError::Unwritable { path, source } => SaveFailure {
            heading: format!("Couldn't Save {}", file_name(path)),
            body: format!(
                "{} Your changes are still in the editor.\n\nFile: {}",
                advice(source),
                path.strip_prefix(folder).unwrap_or(path).display()
            ),
        },
        stave_ide::SaveError::ReadOnly(name) => SaveFailure {
            heading: format!("Couldn't Save {name}"),
            body: "This file is part of Stave's standard library and can't be changed. Save a copy in your project folder to change it there.".to_owned(),
        },
        stave_ide::SaveError::NotOpen(name) => SaveFailure {
            heading: format!("Couldn't Save {name}"),
            body: "The file isn't open in the editor any more, so there is nothing to save. Open it again to make changes.".to_owned(),
        },
    }
}

/// Puts a failed "Export View as glTF…" into words.
pub fn describe_export(error: &crate::export_view::SaveError, path: &Path) -> SaveFailure {
    let heading = format!("Couldn't Save {}", file_name(path));
    match error {
        crate::export_view::SaveError::Export(_) => SaveFailure {
            heading,
            body: "The 3D view couldn't be turned into a glTF file. Check the design for problems, then try again.".to_owned(),
        },
        crate::export_view::SaveError::Write { source, .. } => SaveFailure {
            heading,
            body: format!("{}\n\nFile: {}", advice(source), path.display()),
        },
    }
}

/// Writes a copy of a text to another file (after a failed save), leaving
/// the open document as it was.
pub fn save_copy(path: &Path, text: &str) -> Result<(), SaveFailure> {
    std::fs::write(path, text).map_err(|error| SaveFailure {
        heading: format!("Couldn't Save {}", file_name(path)),
        body: format!("{}\n\nFile: {}", advice(&error), path.display()),
    })
}

fn file_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn unwritable(kind: io::ErrorKind) -> stave_ide::SaveError {
        stave_ide::SaveError::Unwritable {
            path: PathBuf::from("/home/someone/cabinet/doors.stave"),
            source: io::Error::from(kind),
        }
    }

    #[test]
    fn names_the_file_and_says_what_to_do() {
        let failure = describe(
            &unwritable(io::ErrorKind::PermissionDenied),
            Path::new("/home/someone/cabinet"),
        );
        assert_eq!(failure.heading, "Couldn't Save doors.stave");
        assert!(failure.body.starts_with("The folder can't be written to."));
        assert!(failure.body.contains("Save a copy somewhere else"));
        assert!(failure.body.contains("File: doors.stave"));
        assert!(!failure.body.contains("os error"));
    }

    #[test]
    fn a_full_disk_and_a_missing_folder_get_their_own_words() {
        let folder = Path::new("/home/someone/cabinet");
        let full = describe(&unwritable(io::ErrorKind::StorageFull), folder);
        assert!(full.body.starts_with("The disk is full."));
        let gone = describe(&unwritable(io::ErrorKind::NotFound), folder);
        assert!(
            gone.body
                .starts_with("The folder the file was in isn't there")
        );
        let other = describe(&unwritable(io::ErrorKind::Other), folder);
        assert!(other.body.starts_with("Something stopped the file"));
    }

    #[test]
    fn a_copy_that_cannot_be_written_is_described() {
        let failure = save_copy(Path::new("/no/such/folder/doors.stave"), "text");
        let Err(failure) = failure else {
            panic!("writing into a missing folder succeeded");
        };
        assert_eq!(failure.heading, "Couldn't Save doors.stave");
        assert!(failure.body.contains("isn't there any more"));
    }
}
