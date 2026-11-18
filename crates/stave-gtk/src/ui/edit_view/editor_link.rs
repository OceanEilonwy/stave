//! The editor link (`docs/gui.md`, "The text while editing in 3D"):
//! while an edit or a change is in progress and the editor is open, the
//! lines its patch would write or change show in the editor as pending,
//! above the text, with the old lines marked in the text itself; they go
//! when the edit is cancelled or added. While the text doesn't read (a
//! syntax error in one of the project's files), editing in the 3D view is
//! off, and a line at the top of the view says why.
//!
//! ```text
//! ┌ cabinet.stave ──────────────────────────────┐
//! │ PENDING · Add writes this, line 88           │
//! │ + assembly shelf : casework.Shelf {          │
//! │ − panel_stock: oak_19                        │
//! │ + panel_stock: maple_12                      │
//! │  85  ...the text, the old lines tinted...    │
//! ```

use std::path::{Path, PathBuf};
use std::rc::Rc;

use adw::prelude::*;
use stave_app::session::{Editor, LineNumber, Session, SourceLocation};
use stave_ide::Document;

use super::super::Window;

/// The most pending lines shown before "… and N more lines".
const SHOWN_LINES: usize = 14;

/// The lines one file's text would change: from `first` (counted from 1),
/// the old lines that go and the new lines that come in their place.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::ui) struct LineChange {
    /// The first line that differs, from 1.
    pub(in crate::ui) first: usize,
    /// The old lines replaced (empty for lines only inserted).
    pub(in crate::ui) removed: Vec<String>,
    /// The new lines (empty for lines only removed).
    pub(in crate::ui) added: Vec<String>,
}

/// The lines that differ between two texts: the run between their common
/// first lines and their common last lines. `None` when they are the same.
pub(in crate::ui) fn line_change(before: &str, after: &str) -> Option<LineChange> {
    if before == after {
        return None;
    }
    let old: Vec<&str> = before.lines().collect();
    let new: Vec<&str> = after.lines().collect();
    let prefix = old
        .iter()
        .zip(new.iter())
        .take_while(|(a, b)| a == b)
        .count();
    let room = old.len().min(new.len()).saturating_sub(prefix);
    let suffix = old
        .iter()
        .rev()
        .zip(new.iter().rev())
        .take(room)
        .take_while(|(a, b)| a == b)
        .count();
    let removed = old
        .get(prefix..old.len().saturating_sub(suffix))
        .unwrap_or_default();
    let added = new
        .get(prefix..new.len().saturating_sub(suffix))
        .unwrap_or_default();
    if removed.is_empty() && added.is_empty() {
        // Only a line ending changed.
        return None;
    }
    Some(LineChange {
        first: prefix.saturating_add(1),
        removed: removed.iter().map(|line| (*line).to_owned()).collect(),
        added: added.iter().map(|line| (*line).to_owned()).collect(),
    })
}

/// What changed in a replaced line, where it is a word or a value
/// (`panel_stock: oak_19` → `maple_12`): the part before it both lines
/// share, the old words, the new words, and the part after it both share.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::ui) struct ValueChange {
    /// The shared start, up to the start of the word that differs.
    pub(in crate::ui) head: String,
    /// The old words.
    pub(in crate::ui) old: String,
    /// The new words.
    pub(in crate::ui) new: String,
    /// The shared end.
    pub(in crate::ui) tail: String,
}

/// The word or value a replaced line changes; `None` when the two lines
/// share no start (it is shown as a line removed and a line added).
pub(in crate::ui) fn value_change(old: &str, new: &str) -> Option<ValueChange> {
    let word = |c: char| c.is_alphanumeric() || c == '_' || c == '.';
    let shared = old
        .char_indices()
        .zip(new.chars())
        .take_while(|((_, a), b)| a == b)
        .last()
        .map_or(0, |((index, c), _)| index + c.len_utf8());
    // Back to the start of the word the two differ in.
    let cut = old
        .get(..shared)?
        .rfind(|c: char| !word(c))
        .map_or(0, |index| index + 1);
    if cut == 0 {
        return None;
    }
    let (old_rest, new_rest) = (old.get(cut..)?, new.get(cut..)?);
    // The shared end, from the end of the word that differs.
    let mut tail = old_rest
        .chars()
        .rev()
        .zip(new_rest.chars().rev())
        .take_while(|(a, b)| a == b)
        .count()
        .min(old_rest.chars().count())
        .min(new_rest.chars().count());
    let char_start = |text: &str, from_end: usize| {
        text.char_indices()
            .nth(text.chars().count() - from_end)
            .map_or(text.len(), |(index, _)| index)
    };
    // Forward to the end of the word the two differ in.
    while tail > 0 {
        let at = char_start(old_rest, tail);
        if old_rest.get(at..)?.chars().next().is_some_and(word) {
            tail -= 1;
        } else {
            break;
        }
    }
    let old_at = char_start(old_rest, tail);
    let new_at = char_start(new_rest, tail);
    Some(ValueChange {
        head: old.get(..cut)?.to_owned(),
        old: old_rest.get(..old_at)?.to_owned(),
        new: new_rest.get(..new_at)?.to_owned(),
        tail: old_rest.get(old_at..)?.to_owned(),
    })
}

/// A text with every whole-word use of `from` written as `to`: how a
/// rename's pending lines are shown (what is written is the session's
/// rename, which knows every use).
pub(in crate::ui) fn renamed_text(text: &str, from: &str, to: &str) -> String {
    let word = |c: char| c.is_alphanumeric() || c == '_';
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find(from) {
        let before = rest.get(..at).unwrap_or_default();
        let after = rest.get(at + from.len()..).unwrap_or_default();
        let whole = !before.chars().next_back().is_some_and(word)
            && !after.chars().next().is_some_and(word);
        out.push_str(before);
        out.push_str(if whole { to } else { from });
        rest = after;
    }
    out.push_str(rest);
    out
}

/// The files the edit or change in progress would write, with their new
/// text, as last tried.
pub(in crate::ui) fn pending_texts(session: &Session) -> Vec<(PathBuf, String)> {
    let tried = session
        .edit()
        .and_then(|edit| edit.chosen())
        .and_then(|chosen| chosen.tried())
        .or_else(|| session.change().and_then(|change| change.tried()));
    tried
        .map(|tried| {
            tried
                .changed()
                .iter()
                .map(|file| (file.path.clone(), file.text.clone()))
                .collect()
        })
        .unwrap_or_default()
}

/// The pending lines' panel, above the editor's text.
pub(in crate::ui) struct PendingPanel {
    /// The panel.
    pub(in crate::ui) root: gtk::Box,
    head: gtk::Label,
    lines: gtk::Box,
}

impl PendingPanel {
    /// The panel, hidden.
    pub(in crate::ui) fn new() -> PendingPanel {
        let head = gtk::Label::builder()
            .xalign(0.0)
            .wrap(true)
            .css_classes(["stave-pending-head"])
            .build();
        let lines = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .css_classes(["stave-pending-lines"])
            .build();
        let root = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(4)
            .css_classes(["stave-pending"])
            .visible(false)
            .build();
        root.append(&head);
        root.append(
            &gtk::ScrolledWindow::builder()
                .hscrollbar_policy(gtk::PolicyType::Automatic)
                .vscrollbar_policy(gtk::PolicyType::Never)
                .child(&lines)
                .build(),
        );
        PendingPanel { root, head, lines }
    }

    fn line(&self, mark: &str, text: &str, class: &str) {
        let row = gtk::Box::builder().spacing(6).css_classes([class]).build();
        row.append(
            &gtk::Label::builder()
                .label(mark)
                .css_classes(["stave-pending-mark"])
                .build(),
        );
        row.append(
            &gtk::Label::builder()
                .label(text)
                .xalign(0.0)
                .selectable(false)
                .css_classes(["stave-pending-text"])
                .build(),
        );
        self.lines.append(&row);
    }

    fn value_line(&self, change: &ValueChange) {
        let row = gtk::Box::builder()
            .spacing(0)
            .css_classes(["stave-pending-changed"])
            .build();
        row.append(
            &gtk::Label::builder()
                .label("~ ")
                .css_classes(["stave-pending-mark"])
                .build(),
        );
        for (words, class) in [
            (change.head.as_str(), "stave-pending-text"),
            (change.old.as_str(), "stave-pending-old-value"),
            (" → ", "stave-pending-text"),
            (change.new.as_str(), "stave-pending-new-value"),
            (change.tail.as_str(), "stave-pending-text"),
        ] {
            row.append(
                &gtk::Label::builder()
                    .label(words)
                    .css_classes([class])
                    .build(),
            );
        }
        self.lines.append(&row);
    }

    /// Shows one file's pending lines.
    fn show(&self, file: &str, verb: &str, change: &LineChange) {
        while let Some(child) = self.lines.first_child() {
            self.lines.remove(&child);
        }
        self.head.set_label(&format!(
            "PENDING · {verb} writes this to {file}, line {}",
            change.first
        ));
        let mut shown = 0usize;
        let pairs = change.removed.len().min(change.added.len());
        let mut push = |panel: &PendingPanel, f: &dyn Fn(&PendingPanel)| {
            if shown < SHOWN_LINES {
                f(panel);
            }
            shown += 1;
        };
        for index in 0..pairs {
            let (Some(old), Some(new)) = (change.removed.get(index), change.added.get(index))
            else {
                continue;
            };
            match value_change(old, new) {
                Some(parts) => push(self, &|panel| panel.value_line(&parts)),
                None => {
                    push(self, &|panel| panel.line("−", old, "stave-pending-removed"));
                    push(self, &|panel| panel.line("+", new, "stave-pending-added"));
                }
            }
        }
        for old in change.removed.iter().skip(pairs) {
            push(self, &|panel| panel.line("−", old, "stave-pending-removed"));
        }
        for new in change.added.iter().skip(pairs) {
            push(self, &|panel| panel.line("+", new, "stave-pending-added"));
        }
        if shown > SHOWN_LINES {
            self.line(
                "…",
                &format!("and {} more lines", shown - SHOWN_LINES),
                "stave-pending-more",
            );
        }
        self.root.set_visible(true);
    }
}

/// The tag that marks old lines in the text while they are pending.
const PENDING_TAG: &str = "stave-pending-old";

impl Window {
    /// Puts the pending lines' panel into the editor's panel, above the
    /// text.
    pub(in crate::ui) fn connect_editor_link(&self) {
        let panel = &self.editing.pending.root;
        let column = &self.widgets.files.root;
        // After the head and the breadcrumbs.
        let after = column.first_child().and_then(|head| head.next_sibling());
        column.insert_child_after(panel, after.as_ref());
    }

    /// The text of a file of the project as the editor has it (unsaved
    /// changes included), else as on disk.
    fn text_now(&self, path: &Path) -> Option<String> {
        use stave_modules::FileSource;
        self.widgets
            .files
            .edit_files()
            .read(path)
            .ok()
            .or_else(|| std::fs::read_to_string(path).ok())
    }

    /// Shows the edit's or the change's pending lines in the editor, or
    /// takes them away.
    pub(in crate::ui) fn update_pending(self: &Rc<Self>) {
        let (texts, verb, editor_open) = {
            let session = self.session.borrow();
            let verb = if session.change().is_some() {
                "this change"
            } else {
                "Add"
            };
            (
                pending_texts(&session),
                verb,
                session.editor() == Editor::Open,
            )
        };
        self.clear_pending_marks();
        let panel = &self.editing.pending;
        let texts = if texts.is_empty() {
            self.rename_texts()
        } else {
            texts
        };
        let first = texts.iter().find_map(|(path, after)| {
            let before = self.text_now(path).unwrap_or_default();
            line_change(&before, after).map(|change| (path.clone(), change))
        });
        let Some((path, change)) = first.filter(|_| editor_open) else {
            panel.root.set_visible(false);
            return;
        };
        let file = path
            .file_name()
            .map_or_else(String::new, |name| name.to_string_lossy().into_owned());
        let verb = if verb == "Add" { "Add" } else { "This change" };
        panel.show(&file, verb, &change);
        // The text goes to the place, and its old lines are marked.
        if let Some(line) = LineNumber::from_usize(change.first) {
            self.widgets.files.reveal(&SourceLocation {
                path: path.to_string_lossy().into_owned(),
                line,
            });
        }
        if let Some(buffer) = self.widgets.files.buffer_for(&path, false)
            && !change.removed.is_empty()
        {
            let tag = buffer.tag_table().lookup(PENDING_TAG).unwrap_or_else(|| {
                let tag = gtk::TextTag::builder()
                    .name(PENDING_TAG)
                    .strikethrough(true)
                    .background_rgba(&gtk::gdk::RGBA::new(0.85, 0.2, 0.15, 0.16))
                    .build();
                buffer.tag_table().add(&tag);
                tag
            });
            let first = i32::try_from(change.first.saturating_sub(1)).unwrap_or(0);
            let last = i32::try_from(change.first.saturating_sub(1) + change.removed.len())
                .unwrap_or(first);
            if let (Some(start), Some(end)) =
                (buffer.iter_at_line(first), buffer.iter_at_line(last))
            {
                buffer.apply_tag(&tag, &start, &end);
            }
            self.editing.pending_marked.replace(Some(path));
        }
    }

    /// For a rename alone (it is tried without a patch): each file it
    /// rewrites, with the name changed as a word.
    fn rename_texts(&self) -> Vec<(PathBuf, String)> {
        let session = self.session.borrow();
        let Some(change) = session.change() else {
            return Vec::new();
        };
        let stave_app::change::ChangeDetail::Rename { from, to } = change.detail() else {
            return Vec::new();
        };
        change
            .renamed_files()
            .filter_map(|path| {
                let text = self.text_now(path)?;
                Some((path.to_path_buf(), renamed_text(&text, from, to.as_str())))
            })
            .collect()
    }

    /// Takes the pending marks off the text.
    fn clear_pending_marks(&self) {
        let Some(path) = self.editing.pending_marked.take() else {
            return;
        };
        if let Some(buffer) = self.widgets.files.buffer_for(&path, false)
            && let Some(tag) = buffer.tag_table().lookup(PENDING_TAG)
        {
            let (start, end) = buffer.bounds();
            buffer.remove_tag(&tag, &start, &end);
        }
    }

    // ---- while the text doesn't read ----

    /// Why editing in the 3D view is off, when it is: a file of the project
    /// that doesn't read (a syntax error), in words.
    pub(in crate::ui) fn parse_block_line(&self) -> Option<String> {
        let analysis = self.widgets.files.analysis()?;
        for (file, document) in analysis.project_files() {
            let broken = analysis
                .diagnostics(file)
                .find(|diagnostic| diagnostic.is_error() && diagnostic.code.layer() == 1);
            if let Some(diagnostic) = broken {
                let name = match &document {
                    Document::File(path) => path
                        .file_name()
                        .map_or_else(String::new, |name| name.to_string_lossy().into_owned()),
                    Document::Std { file, .. } => (*file).to_owned(),
                };
                return Some(format!(
                    "Editing in 3D is off while {name} doesn't read: {}. Fix the text first.",
                    diagnostic.message
                ));
            }
        }
        None
    }

    /// `true` while the text reads, so the 3D view's gestures may edit it.
    pub(in crate::ui) fn gestures_allowed(&self) -> bool {
        self.parse_block_line().is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inserted_lines_are_found_between_the_common_ones() {
        let before = "a\nb\nc\n";
        let after = "a\nb\nnew one\nnew two\nc\n";
        assert_eq!(
            line_change(before, after),
            Some(LineChange {
                first: 3,
                removed: Vec::new(),
                added: vec!["new one".to_owned(), "new two".to_owned()],
            })
        );
        assert_eq!(line_change(before, before), None);
    }

    #[test]
    fn a_replaced_value_shows_old_and_new() {
        let before = "assembly case : Case {\n  panel_stock: oak_19\n}\n";
        let after = "assembly case : Case {\n  panel_stock: maple_12\n}\n";
        let change = line_change(before, after).unwrap();
        assert_eq!(change.first, 2);
        assert_eq!(change.removed, vec!["  panel_stock: oak_19".to_owned()]);
        assert_eq!(change.added, vec!["  panel_stock: maple_12".to_owned()]);
        assert_eq!(
            value_change(&change.removed[0], &change.added[0]),
            Some(ValueChange {
                head: "  panel_stock: ".to_owned(),
                old: "oak_19".to_owned(),
                new: "maple_12".to_owned(),
                tail: String::new(),
            })
        );
        assert_eq!(
            value_change(
                "assembly drawer[bottom, top] : Drawer {",
                "assembly drawer_small[bottom, top] : Drawer {"
            ),
            Some(ValueChange {
                head: "assembly ".to_owned(),
                old: "drawer".to_owned(),
                new: "drawer_small".to_owned(),
                tail: "[bottom, top] : Drawer {".to_owned(),
            })
        );
        assert_eq!(value_change("abc", "xyz"), None);
    }

    #[test]
    fn a_rename_changes_whole_words() {
        assert_eq!(
            renamed_text("drawer[top] drawer_bay drawer.front", "drawer", "box"),
            "box[top] drawer_bay box.front"
        );
    }

    #[test]
    fn a_repeated_line_at_the_end_is_not_counted_twice() {
        let before = "}\n";
        let after = "}\n\nx {\n}\n";
        let change = line_change(before, after).unwrap();
        assert_eq!(change.first, 2);
        assert!(change.removed.is_empty());
        assert_eq!(change.added, vec!["", "x {", "}"]);
    }
}
