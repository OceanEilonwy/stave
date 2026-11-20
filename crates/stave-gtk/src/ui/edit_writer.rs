//! Where Add writes in the window (`docs/editing.md`, "The edit in
//! progress"): through the editor's open buffers when the file has a tab
//! (or the editor is open, which opens one), so the text shows the change
//! unsaved and one undo takes it back; otherwise to disk, as the command
//! line does. The session decides what to write; this only writes it.
//!
//! The window builds a writer for each Add ([`super::files::FilesPage::writer`])
//! from its own copies of the open buffers (GObjects the Files component
//! published) and whether the editor shows. It shares no Rust state, but
//! it writes into the Code panel's live buffers inside `session.apply`,
//! because `stave_app`'s [`WriteEdits`] must answer the commit at once:
//! an accepted exception in docs/relm4-migration.md ("Accepted
//! exceptions"), sound because the component's handlers on a buffer only
//! queue messages.
//! A file it writes while the editor is open but that has no tab gets a
//! buffer made here, which the writer reports through a channel the window
//! drains straight after the commit ([`super::files::FilesPage::take_written`]):
//! the window reads its unsaved text at once and the component adopts it
//! as the file's tab.

use stave_app::i18n::{fill_in, tr};
use std::collections::BTreeMap;
use std::io;
use std::sync::mpsc;

use gtk::prelude::*;
use stave_app::editing::{FileEdit, WriteEdits};
use stave_app::scratch::DiskWriter;
use stave_ide::Document;

use super::files::{Made, Written};

/// Writes an edit's files through the open buffers, else to disk.
pub(super) struct BufferWriter {
    /// The open documents' buffers, and any made for this commit.
    buffers: BTreeMap<Document, gtk::TextBuffer>,
    /// The editor is open: a file the commit writes is opened in a tab, so
    /// the text shows the change.
    editor_open: bool,
    /// The tag table a buffer made here shares with the editor's.
    table: gtk::TextTagTable,
    /// Where it says what it wrote through a buffer.
    written: mpsc::Sender<Written>,
}

impl BufferWriter {
    /// A writer through these buffers; `editor_open` says, when Add is
    /// pressed, whether the editor shows.
    pub(super) fn through(
        buffers: BTreeMap<Document, gtk::TextBuffer>,
        editor_open: bool,
        table: gtk::TextTagTable,
        written: mpsc::Sender<Written>,
    ) -> BufferWriter {
        BufferWriter {
            buffers,
            editor_open,
            table,
            written,
        }
    }

    /// The buffer a file is written through, if it has a tab. A file the
    /// commit creates always goes to disk.
    fn buffer(&self, file: &FileEdit) -> Option<gtk::TextBuffer> {
        if file.created {
            return None;
        }
        self.buffers
            .get(&Document::File(file.path.clone()))
            .cloned()
    }

    /// With the editor open, a file with no tab is written through a new
    /// buffer holding its text on disk, so one undo takes the change back.
    fn wants_buffer(&self, file: &FileEdit) -> bool {
        self.editor_open && !file.created
    }
}

impl WriteEdits for BufferWriter {
    fn write_edits(&mut self, file: &FileEdit) -> io::Result<()> {
        let document = Document::File(file.path.clone());
        // The text a buffer made here starts from: the file's on disk.
        let (buffer, base) = match self.buffer(file) {
            Some(buffer) => (buffer, None),
            None if self.wants_buffer(file) => {
                let text = std::fs::read_to_string(&file.path)?;
                (super::editor::new_buffer(&self.table, &text), Some(text))
            }
            None => return DiskWriter.write_edits(file),
        };
        apply_to_buffer(&buffer, file)?;
        if base.is_some() {
            self.buffers.insert(document.clone(), buffer.clone());
        }
        if base.is_some() || self.editor_open {
            // The window may have gone; then nobody needs to know.
            let _ = self.written.send(Written {
                document,
                created: base.map(|base| Made { buffer, base }),
                // The editor shows the file the change went into.
                show: self.editor_open,
            });
        }
        Ok(())
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
            // A buffer made for it would hold the file's text on disk.
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
    io::Error::other(fill_in(
        &tr("'{file}' changed since the edit began; start the edit again"),
        &[("file", &file.path.display())],
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
            _ => return Err(io::Error::other(tr("an edit falls inside a character"))),
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
        Err(io::Error::other(fill_in(
            &tr("'{file}' didn't come out as the edit says"),
            &[("file", &file.path.display())],
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
