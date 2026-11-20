//! Open editor buffers: the text people are typing, which is the source
//! for checking while they type (grammar, "Modules in an editor"), and
//! saving it back to disk.
//!
//! [`Buffers`] holds every open document's text and the text last read
//! from or written to disk, so it knows which ones have unsaved changes.
//! [`Buffers::overlay`] hands the unsaved texts to the loader as an
//! [`Overlay`]: a [`FileSource`] that reads open buffers in place of their
//! files and the disk for everything else. It is a plain value that can
//! cross threads, so a check can run in the background on a snapshot.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use stave_modules::{DiskFiles, FileSource, normalize};

use crate::document::Document;

/// Why a document can't be opened.
#[derive(Debug, thiserror::Error)]
pub enum OpenError {
    /// The file can't be read.
    #[error("can't read '{}': {source}", path.display())]
    Unreadable {
        /// The file.
        path: PathBuf,
        /// What went wrong.
        source: io::Error,
    },
}

/// Why a document can't be changed or saved.
#[derive(Debug, thiserror::Error)]
pub enum SaveError {
    /// A standard library file: it is part of Stave and can't be changed.
    #[error("'{0}' is part of the standard library and can't be changed")]
    ReadOnly(String),
    /// The document isn't open.
    #[error("'{0}' isn't open")]
    NotOpen(String),
    /// Writing the file failed; the buffer keeps its text.
    #[error("can't save '{}': {source}", path.display())]
    Unwritable {
        /// The file.
        path: PathBuf,
        /// What went wrong.
        source: io::Error,
    },
}

/// One open document.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Buffer {
    /// What the editor shows.
    text: String,
    /// What is on disk, as last read or written.
    saved: String,
    /// The text on disk the unsaved changes were made from, kept when the
    /// file changed on disk underneath them: saving merges the two changes
    /// from it rather than writing over the one on disk.
    before: Option<String>,
}

/// Merges two changes to one text for a save: `(before, yours, on_disk)`,
/// where `yours` and `on_disk` were both changed from `before`. It gives
/// the merged text and whether it holds conflict markers (both changed the
/// same lines).
pub type MergeText<'a> = &'a dyn Fn(&str, &str, &str) -> (String, bool);

/// How a save went.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Saved {
    /// The buffer's text was written as it was.
    AsTyped,
    /// The file had changed on disk under unsaved changes: the two were
    /// merged, and the buffer now holds what was written.
    Merged {
        /// Both changed the same lines: both are kept between conflict
        /// markers.
        conflicted: bool,
    },
}

/// What happened to an open buffer when its file changed on disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reload {
    /// The buffer had no unsaved changes and now shows the file's new text.
    Reloaded,
    /// The buffer has unsaved changes, which are kept; saving merges them
    /// with the change on disk.
    KeptChanges,
    /// Nothing changed.
    Unchanged,
}

/// Every open document's text.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Buffers {
    open: BTreeMap<Document, Buffer>,
}

impl Buffers {
    /// No open documents.
    pub fn new() -> Buffers {
        Buffers::default()
    }

    /// Opens a document (reading it from disk, or from Stave for a
    /// standard file) and gives its text. Opening an open document gives
    /// its buffer, unsaved changes included.
    pub fn open(&mut self, document: &Document) -> Result<&str, OpenError> {
        if !self.open.contains_key(document) {
            let text = read(document)?;
            self.open.insert(
                document.clone(),
                Buffer {
                    saved: text.clone(),
                    text,
                    before: None,
                },
            );
        }
        Ok(self.open.get(document).map_or("", |b| b.text.as_str()))
    }

    /// `true` if the document is open.
    pub fn is_open(&self, document: &Document) -> bool {
        self.open.contains_key(document)
    }

    /// The open documents, sorted.
    pub fn documents(&self) -> impl Iterator<Item = &Document> {
        self.open.keys()
    }

    /// An open document's text.
    pub fn text(&self, document: &Document) -> Option<&str> {
        self.open.get(document).map(|b| b.text.as_str())
    }

    /// Replaces an open document's text with what the editor now shows.
    pub fn edit(&mut self, document: &Document, text: &str) -> Result<(), SaveError> {
        if document.is_read_only() {
            return Err(SaveError::ReadOnly(document.source_path()));
        }
        let buffer = self
            .open
            .get_mut(document)
            .ok_or_else(|| SaveError::NotOpen(document.source_path()))?;
        if buffer.text != text {
            text.clone_into(&mut buffer.text);
        }
        Ok(())
    }

    /// `true` if the document has changes that aren't saved.
    pub fn is_dirty(&self, document: &Document) -> bool {
        self.open.get(document).is_some_and(|b| b.text != b.saved)
    }

    /// Every document with unsaved changes, sorted.
    pub fn dirty(&self) -> impl Iterator<Item = &Document> {
        self.open
            .iter()
            .filter(|(_, b)| b.text != b.saved)
            .map(|(document, _)| document)
    }

    /// Writes a document's text to its file. The file is written whole to
    /// a temporary file next to it first and then moved over it, so a
    /// failed save never leaves half a file.
    ///
    /// When the file changed on disk since the unsaved changes were made
    /// from it (someone else wrote it), the two changes are merged with
    /// `merge` and the merged text is written and shown: a change on disk
    /// is never just written over.
    pub fn save(&mut self, document: &Document, merge: MergeText<'_>) -> Result<Saved, SaveError> {
        let Document::File(path) = document else {
            return Err(SaveError::ReadOnly(document.source_path()));
        };
        let buffer = self
            .open
            .get_mut(document)
            .ok_or_else(|| SaveError::NotOpen(document.source_path()))?;
        // The file as it is now; a change not yet taken in by `reload`
        // counts as much as one that was.
        let on_disk = fs::read_to_string(path).ok();
        let before = match (&buffer.before, &on_disk) {
            (Some(before), _) => Some(before.clone()),
            (None, Some(now)) if *now != buffer.saved && buffer.text != buffer.saved => {
                Some(buffer.saved.clone())
            }
            (None, Some(_) | None) => None,
        };
        let (text, saved) = match (&before, &on_disk) {
            (Some(before), Some(now)) if now != before => {
                let (merged, conflicted) = merge(before, &buffer.text, now);
                (merged, Saved::Merged { conflicted })
            }
            (Some(_) | None, Some(_) | None) => (buffer.text.clone(), Saved::AsTyped),
        };
        write_whole(path, &text).map_err(|source| SaveError::Unwritable {
            path: path.clone(),
            source,
        })?;
        buffer.saved.clone_from(&text);
        buffer.text = text;
        buffer.before = None;
        Ok(saved)
    }

    /// Saves every document with unsaved changes; the first failure stops.
    /// Returns the documents whose text a merge changed (see
    /// [`Buffers::save`]), with whether each holds conflict markers.
    pub fn save_all(&mut self, merge: MergeText<'_>) -> Result<Vec<(Document, bool)>, SaveError> {
        let dirty: Vec<Document> = self.dirty().cloned().collect();
        let mut merged = Vec::new();
        for document in dirty {
            match self.save(&document, merge)? {
                Saved::AsTyped => {}
                Saved::Merged { conflicted } => merged.push((document, conflicted)),
            }
        }
        Ok(merged)
    }

    /// Throws away a document's unsaved changes, reading its file again.
    pub fn revert(&mut self, document: &Document) -> Result<(), OpenError> {
        let text = read(document)?;
        if let Some(buffer) = self.open.get_mut(document) {
            buffer.saved.clone_from(&text);
            buffer.text = text;
            buffer.before = None;
        }
        Ok(())
    }

    /// Takes a change made to a file on disk by something else: a buffer
    /// without unsaved changes shows the new text; one with unsaved
    /// changes keeps them, and its next save merges the two.
    pub fn reload(&mut self, document: &Document) -> Result<Reload, OpenError> {
        let Some(buffer) = self.open.get(document) else {
            return Ok(Reload::Unchanged);
        };
        let text = read(document)?;
        if text == buffer.saved {
            return Ok(Reload::Unchanged);
        }
        let dirty = buffer.text != buffer.saved;
        if let Some(buffer) = self.open.get_mut(document) {
            if dirty {
                // Remember what the unsaved changes were made from, so the
                // save merges rather than writes over the change on disk.
                if buffer.before.is_none() {
                    buffer.before = Some(buffer.saved.clone());
                }
            } else {
                buffer.text.clone_from(&text);
            }
            buffer.saved = text;
        }
        Ok(if dirty {
            Reload::KeptChanges
        } else {
            Reload::Reloaded
        })
    }

    /// Closes a document, unsaved changes and all; the caller asks first.
    pub fn close(&mut self, document: &Document) {
        self.open.remove(document);
    }

    /// The unsaved texts, for checking what the editor shows.
    pub fn overlay(&self) -> Overlay {
        Overlay {
            texts: self
                .open
                .iter()
                .filter(|(_, b)| b.text != b.saved)
                .filter_map(|(document, b)| match document {
                    Document::File(path) => Some((normalize(path), b.text.clone())),
                    Document::Std { .. } => None,
                })
                .collect(),
        }
    }
}

/// Reads a document's text from disk or from Stave.
fn read(document: &Document) -> Result<String, OpenError> {
    match document {
        Document::File(path) => fs::read_to_string(path).map_err(|source| OpenError::Unreadable {
            path: path.clone(),
            source,
        }),
        Document::Std { .. } => Ok(document.std_text().unwrap_or_default().to_owned()),
    }
}

/// Writes `text` to a temporary file beside `path` and moves it over
/// `path`.
fn write_whole(path: &Path, text: &str) -> io::Result<()> {
    let name = path
        .file_name()
        .ok_or_else(|| io::Error::from(io::ErrorKind::InvalidInput))?;
    let mut temporary_name = std::ffi::OsString::from(".");
    temporary_name.push(name);
    temporary_name.push(".saving");
    let temporary = path.with_file_name(temporary_name);
    fs::write(&temporary, text)?;
    fs::rename(&temporary, path).inspect_err(|_| {
        // Best effort: don't leave the temporary file behind.
        let _ = fs::remove_file(&temporary);
    })
}

/// Unsaved editor texts over the disk: what the loader reads while people
/// type. Paths are normalised ([`normalize`]) when looked up.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Overlay {
    texts: BTreeMap<PathBuf, String>,
}

impl Overlay {
    /// Nothing unsaved: reads the disk.
    pub fn new() -> Overlay {
        Overlay::default()
    }

    /// Adds or replaces an unsaved text.
    pub fn insert(&mut self, path: impl AsRef<Path>, text: impl Into<String>) {
        self.texts.insert(normalize(path.as_ref()), text.into());
    }

    /// `true` if nothing is unsaved.
    pub fn is_empty(&self) -> bool {
        self.texts.is_empty()
    }
}

impl FileSource for Overlay {
    fn list_stave_files(&self, folder: &Path) -> io::Result<Vec<PathBuf>> {
        let mut files = DiskFiles.list_stave_files(folder)?;
        let normal = normalize(folder);
        for path in self.texts.keys() {
            let new = path.parent() == Some(normal.as_path())
                && !files.iter().any(|f| normalize(f) == *path);
            if new {
                files.push(path.clone());
            }
        }
        Ok(files)
    }

    fn read(&self, path: &Path) -> io::Result<String> {
        match self.texts.get(&normalize(path)) {
            Some(text) => Ok(text.clone()),
            None => DiskFiles.read(path),
        }
    }

    fn canonical(&self, folder: &Path) -> io::Result<PathBuf> {
        DiskFiles.canonical(folder)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let folder =
            std::env::temp_dir().join(format!("stave-ide-buffers-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&folder);
        fs::create_dir_all(&folder).unwrap();
        folder
    }

    /// A merge that keeps the buffer's text, for saves with nothing on
    /// disk to merge.
    fn keep_yours(_before: &str, yours: &str, _on_disk: &str) -> (String, bool) {
        (yours.to_owned(), false)
    }

    /// A merge that says what it was given, to see that a save merges.
    fn tell(before: &str, yours: &str, on_disk: &str) -> (String, bool) {
        (format!("{before}|{yours}|{on_disk}"), true)
    }

    #[test]
    fn a_save_merges_unsaved_changes_with_a_change_on_disk() {
        let folder = scratch("merge");
        let (a, b) = (folder.join("a.stave"), folder.join("b.stave"));
        fs::write(&a, "old").unwrap();
        fs::write(&b, "old").unwrap();
        let (a, b) = (Document::File(a.clone()), Document::File(b.clone()));
        let mut buffers = Buffers::new();
        buffers.open(&a).unwrap();
        buffers.open(&b).unwrap();
        buffers.edit(&a, "mine").unwrap();
        buffers.edit(&b, "mine").unwrap();
        for document in [&a, &b] {
            let Document::File(path) = document else {
                unreachable!()
            };
            fs::write(path, "theirs").unwrap();
        }
        // Taken in by a reload, or not yet: either way the save merges.
        assert_eq!(buffers.reload(&a).unwrap(), Reload::KeptChanges);
        assert_eq!(
            buffers.save(&a, &tell).unwrap(),
            Saved::Merged { conflicted: true }
        );
        assert_eq!(buffers.text(&a), Some("old|mine|theirs"));
        assert!(!buffers.is_dirty(&a));
        let Document::File(path) = &b else {
            unreachable!()
        };
        assert_eq!(buffers.save_all(&tell).unwrap(), vec![(b.clone(), true)]);
        assert_eq!(fs::read_to_string(path).unwrap(), "old|mine|theirs");
        // With nothing changed on disk, a save writes what was typed.
        buffers.edit(&b, "again").unwrap();
        assert_eq!(buffers.save(&b, &tell).unwrap(), Saved::AsTyped);
        assert_eq!(fs::read_to_string(path).unwrap(), "again");
        fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn edits_are_unsaved_until_saved() {
        let folder = scratch("save");
        let path = folder.join("a.stave");
        fs::write(&path, "stave 1\n").unwrap();
        let document = Document::File(path.clone());
        let mut buffers = Buffers::new();
        assert_eq!(buffers.open(&document).unwrap(), "stave 1\n");
        assert!(!buffers.is_dirty(&document));
        buffers.edit(&document, "stave 1\n\nlet a = 1\n").unwrap();
        assert!(buffers.is_dirty(&document));
        assert_eq!(buffers.dirty().count(), 1);
        // The disk still has the old text; the overlay has the new one.
        assert_eq!(fs::read_to_string(&path).unwrap(), "stave 1\n");
        let overlay = buffers.overlay();
        assert_eq!(overlay.read(&path).unwrap(), "stave 1\n\nlet a = 1\n");
        // Opening again keeps the unsaved text.
        assert_eq!(buffers.open(&document).unwrap(), "stave 1\n\nlet a = 1\n");
        buffers.save(&document, &keep_yours).unwrap();
        assert!(!buffers.is_dirty(&document));
        assert_eq!(fs::read_to_string(&path).unwrap(), "stave 1\n\nlet a = 1\n");
        assert!(buffers.overlay().is_empty());
        // No temporary file is left behind.
        assert_eq!(fs::read_dir(&folder).unwrap().count(), 1);
        fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn editing_back_to_the_saved_text_is_clean() {
        let folder = scratch("clean");
        let path = folder.join("a.stave");
        fs::write(&path, "stave 1\n").unwrap();
        let document = Document::File(path);
        let mut buffers = Buffers::new();
        buffers.open(&document).unwrap();
        buffers.edit(&document, "stave 1\n// x\n").unwrap();
        buffers.edit(&document, "stave 1\n").unwrap();
        assert!(!buffers.is_dirty(&document));
        fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn a_change_on_disk_reloads_a_clean_buffer_and_keeps_unsaved_changes() {
        let folder = scratch("reload");
        let (a, b) = (folder.join("a.stave"), folder.join("b.stave"));
        fs::write(&a, "stave 1\n").unwrap();
        fs::write(&b, "stave 1\n").unwrap();
        let (a, b) = (Document::File(a), Document::File(b));
        let mut buffers = Buffers::new();
        buffers.open(&a).unwrap();
        buffers.open(&b).unwrap();
        buffers.edit(&b, "stave 1\n// mine\n").unwrap();
        for document in [&a, &b] {
            let Document::File(path) = document else {
                unreachable!()
            };
            fs::write(path, "stave 1\n// theirs\n").unwrap();
        }
        assert_eq!(buffers.reload(&a).unwrap(), Reload::Reloaded);
        assert_eq!(buffers.text(&a), Some("stave 1\n// theirs\n"));
        assert_eq!(buffers.reload(&b).unwrap(), Reload::KeptChanges);
        assert_eq!(buffers.text(&b), Some("stave 1\n// mine\n"));
        assert!(buffers.is_dirty(&b));
        assert_eq!(buffers.reload(&a).unwrap(), Reload::Unchanged);
        buffers.revert(&b).unwrap();
        assert_eq!(buffers.text(&b), Some("stave 1\n// theirs\n"));
        fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn the_standard_library_is_read_only() {
        let dado = Document::std("joinery", "dado.stave").unwrap();
        let mut buffers = Buffers::new();
        assert!(buffers.open(&dado).unwrap().contains("kind dado"));
        assert!(matches!(
            buffers.edit(&dado, "x"),
            Err(SaveError::ReadOnly(_))
        ));
        assert!(matches!(
            buffers.save(&dado, &keep_yours),
            Err(SaveError::ReadOnly(_))
        ));
        assert!(!buffers.is_dirty(&dado));
    }

    #[test]
    fn a_failed_save_keeps_the_text() {
        let folder = scratch("fail");
        let path = folder.join("gone").join("a.stave");
        let document = Document::File(path);
        let mut buffers = Buffers::new();
        // Not open yet: can't be edited.
        assert!(matches!(
            buffers.edit(&document, "x"),
            Err(SaveError::NotOpen(_))
        ));
        assert!(buffers.open(&document).is_err());
        fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn the_overlay_reads_the_disk_for_files_not_changed() {
        let folder = scratch("overlay");
        fs::write(folder.join("a.stave"), "disk a").unwrap();
        fs::write(folder.join("b.stave"), "disk b").unwrap();
        let mut overlay = Overlay::new();
        overlay.insert(folder.join("./b.stave"), "typed b");
        overlay.insert(folder.join("c.stave"), "new c");
        assert_eq!(overlay.read(&folder.join("a.stave")).unwrap(), "disk a");
        assert_eq!(overlay.read(&folder.join("b.stave")).unwrap(), "typed b");
        let mut listed = overlay.list_stave_files(&folder).unwrap();
        listed.sort();
        assert_eq!(listed.len(), 3);
        fs::remove_dir_all(&folder).unwrap();
    }
}
