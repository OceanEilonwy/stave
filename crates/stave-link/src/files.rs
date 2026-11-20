//! Reading and writing an app's settings file: a missing file reads as
//! none, and a write goes to a temporary file beside it that is then
//! renamed over it, so the app never reads half a file.

use std::fs;
use std::io::ErrorKind;
use std::path::Path;

use crate::LinkError;

/// The file's text, or `None` when there is no such file.
pub(crate) fn read(path: &Path) -> Result<Option<String>, LinkError> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(source) => Err(LinkError::Read {
            path: path.to_path_buf(),
            source,
        }),
    }
}

/// Writes the file whole, making its folder when needed, keeping the
/// permissions of the file it replaces.
pub(crate) fn write(path: &Path, text: &str) -> Result<(), LinkError> {
    let failed = |source| LinkError::Write {
        path: path.to_path_buf(),
        source,
    };
    if let Some(folder) = path.parent() {
        fs::create_dir_all(folder).map_err(failed)?;
    }
    let mut temporary = path.as_os_str().to_owned();
    temporary.push(".stave-new");
    let temporary = Path::new(&temporary);
    fs::write(temporary, text).map_err(failed)?;
    if let Ok(metadata) = fs::metadata(path) {
        fs::set_permissions(temporary, metadata.permissions()).map_err(failed)?;
    }
    fs::rename(temporary, path).map_err(failed)
}
