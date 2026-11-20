//! Reading and writing an app's settings file: a missing file reads as
//! none, and a write goes to a temporary file beside it that is then
//! renamed over it, so the app never reads half a file.
//!
//! A settings file that is a symlink (a dotfiles setup) is written
//! through: the file it points at is replaced, and the link stays. The
//! temporary file is made with the old file's permissions (or the
//! owner's alone for a new file) before any text goes in, so settings
//! holding keys are never readable by others, and its name is unique to
//! the process and the write, so two installs at once don't share one.

use std::fs::{self, OpenOptions};
use std::io::{ErrorKind, Write as _};
use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::LinkError;

/// The permissions of a settings file Stave makes: the owner's alone.
const NEW_FILE_MODE: u32 = 0o600;

/// The permission bits of a mode (no file type).
const PERMISSION_BITS: u32 = 0o7777;

/// Counts writes, for the temporary files' names.
static WRITES: AtomicU64 = AtomicU64::new(0);

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
/// permissions of the file it replaces, and writing through a symlink.
pub(crate) fn write(path: &Path, text: &str) -> Result<(), LinkError> {
    let failed = |source| LinkError::Write {
        path: path.to_path_buf(),
        source,
    };
    let target = resolved(path).map_err(failed)?;
    if let Some(folder) = target.parent() {
        fs::create_dir_all(folder).map_err(failed)?;
    }
    let mode = match fs::metadata(&target) {
        Ok(metadata) => metadata.permissions().mode() & PERMISSION_BITS,
        Err(error) if error.kind() == ErrorKind::NotFound => NEW_FILE_MODE,
        Err(error) => return Err(failed(error)),
    };
    let mut temporary = target.as_os_str().to_owned();
    temporary.push(format!(
        ".stave-new-{}-{}",
        std::process::id(),
        WRITES.fetch_add(1, Ordering::Relaxed)
    ));
    let temporary = PathBuf::from(temporary);
    let written = write_new(&temporary, mode, text).and_then(|()| fs::rename(&temporary, &target));
    if written.is_err() {
        // Best effort: the temporary file is Stave's own, and the error
        // that matters is the write's.
        let _ = fs::remove_file(&temporary);
    }
    written.map_err(failed)
}

/// The file a write lands in: `path` itself, or the file its symlinks
/// lead to (a dangling link's target, which the write then makes).
fn resolved(path: &Path) -> std::io::Result<PathBuf> {
    match fs::canonicalize(path) {
        Ok(target) => Ok(target),
        Err(error) if error.kind() == ErrorKind::NotFound => match fs::symlink_metadata(path) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                let link = fs::read_link(path)?;
                Ok(match path.parent() {
                    Some(folder) if link.is_relative() => folder.join(link),
                    Some(_) | None => link,
                })
            }
            Ok(_) | Err(_) => Ok(path.to_path_buf()),
        },
        Err(error) => Err(error),
    }
}

/// Makes a new file with `mode` and writes `text` into it, flushed to
/// disk.
fn write_new(path: &Path, mode: u32, text: &str) -> std::io::Result<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(mode)
        .open(path)?;
    // The mode given at creation is narrowed by the umask; set it exactly.
    file.set_permissions(fs::Permissions::from_mode(mode))?;
    file.write_all(text.as_bytes())?;
    file.sync_all()
}
