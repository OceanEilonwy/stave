//! Where Add writes in the window (`docs/editing.md`, "The edit in
//! progress"): through the editor's open buffers when the file has a tab
//! (or the editor is open, which opens one), so the text shows the change
//! unsaved and one undo takes it back; otherwise to disk, as the command
//! line does. The session decides what to write; this only writes it.

use std::cell::Cell;
use std::io;
use std::rc::{Rc, Weak};

use gtk::prelude::*;
use stave_app::editing::{FileEdit, WriteEdits};
use stave_app::scratch::DiskWriter;
use stave_ide::Document;

use super::files::FilesPage;

/// Writes an edit's files through the open buffers, else to disk.
pub(super) struct BufferWriter {
    files: Weak<FilesPage>,
    /// The editor is open: a file the commit writes is opened in a tab, so
    /// the text shows the change.
    editor_open: Rc<Cell<bool>>,
}

impl BufferWriter {
    /// A writer through these buffers; `editor_open` says, when Add is
    /// pressed, whether the editor shows.
    pub(super) fn new(files: &Rc<FilesPage>, editor_open: Rc<Cell<bool>>) -> BufferWriter {
        BufferWriter {
            files: Rc::downgrade(files),
            editor_open,
        }
    }

    /// The buffer a file is written through, if any. A file the commit
    /// creates always goes to disk.
    fn buffer(&self, file: &FileEdit) -> Option<gtk::TextBuffer> {
        if file.created {
            return None;
        }
        self.files
            .upgrade()?
            .buffer_for(&file.path, self.editor_open.get())
    }
}

impl WriteEdits for BufferWriter {
    fn write_edits(&mut self, file: &FileEdit) -> io::Result<()> {
        match self.buffer(file) {
            Some(buffer) => {
                apply_to_buffer(&buffer, file)?;
                // The editor shows the file the change went into.
                if self.editor_open.get()
                    && let Some(files) = self.files.upgrade()
                {
                    files.open_document(&Document::File(file.path.clone()));
                }
                Ok(())
            }
            None => DiskWriter.write_edits(file),
        }
    }

    fn can_write(&mut self, file: &FileEdit) -> io::Result<()> {
        match self.buffer(file) {
            Some(buffer) => {
                if buffer_text(&buffer) == file.before {
                    Ok(())
                } else {
                    Err(changed(file))
                }
            }
            None => DiskWriter.can_write(file),
        }
    }
}

/// The whole text of a buffer.
fn buffer_text(buffer: &gtk::TextBuffer) -> String {
    buffer
        .text(&buffer.start_iter(), &buffer.end_iter(), true)
        .to_string()
}

fn changed(file: &FileEdit) -> io::Error {
    io::Error::other(format!(
        "'{}' changed since the edit began; start the edit again",
        file.path.display()
    ))
}

/// The character offset of a byte offset in `text`; `None` when it isn't
/// on a character boundary.
fn char_offset(text: &str, byte: usize) -> Option<i32> {
    let before = text.get(..byte)?;
    i32::try_from(before.chars().count()).ok()
}

/// Makes a file's edits in its buffer as one user action, so one undo
/// takes the whole commit back. The buffer must hold the text the edits
/// were computed from.
pub(super) fn apply_to_buffer(buffer: &gtk::TextBuffer, file: &FileEdit) -> io::Result<()> {
    if buffer_text(buffer) != file.before {
        return Err(changed(file));
    }
    // Every offset is worked out from the text before, which the edits
    // (last in the file first) leave alone ahead of each one.
    let mut ranges = Vec::with_capacity(file.edits.len());
    for edit in &file.edits {
        let start = char_offset(&file.before, edit.range.start);
        let end = char_offset(&file.before, edit.range.end);
        match (start, end) {
            (Some(start), Some(end)) if start <= end => ranges.push((start, end, &edit.text)),
            _ => return Err(io::Error::other("an edit falls inside a character")),
        }
    }
    buffer.begin_user_action();
    for (start, end, text) in ranges {
        let mut from = buffer.iter_at_offset(start);
        let mut to = buffer.iter_at_offset(end);
        buffer.delete(&mut from, &mut to);
        buffer.insert(&mut from, text);
    }
    buffer.end_user_action();
    if buffer_text(buffer) == file.after {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "'{}' didn't come out as the edit says",
            file.path.display()
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offsets_count_characters_not_bytes() {
        assert_eq!(char_offset("a·b", 0), Some(0));
        assert_eq!(char_offset("a·b", 1), Some(1));
        assert_eq!(char_offset("a·b", 3), Some(2));
        assert_eq!(char_offset("a·b", 2), None);
    }
}
