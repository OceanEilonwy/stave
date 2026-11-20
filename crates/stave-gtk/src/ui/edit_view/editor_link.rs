//! The editor link (`docs/gui.md`, "The text while editing in 3D"):
//! while an edit or a change is in progress, the lines its patch would
//! write or change show as a small diff in the "Pending" section of the
//! card in the Edit panel (`@ line N`, then `−` and `+` lines); when the
//! editor is open, it goes to the first place and the old lines are
//! marked in the text itself. They go when the edit is cancelled or
//! applied. While the text doesn't read (a syntax error in one of the
//! project's files), editing in the 3D view is off, and a line at the top
//! of the view says why.
//!
//! ```text
//! ▾ Pending                        cabinet.stave
//!   @ line 88
//!   + assembly shelf : casework.Shelf {
//!   − panel_stock: oak_19
//!   + panel_stock: maple_12
//! ```

use stave_app::i18n::{fill_in, ntr, tr};
use std::path::{Path, PathBuf};

use adw::prelude::*;
use stave_app::session::{Editor, LineNumber, Session, SourceLocation};
use stave_ide::{Analysis, Document};

use super::super::Window;
use super::super::files::FilesInput;
use super::super::sections;

/// The most pending lines shown before "… and N more lines".
const SHOWN_LINES: usize = 14;

/// One place a file's text would change: from `first` (counted from 1),
/// the old lines that go and the new lines that come in their place.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::ui) struct LineChange {
    /// The first line that differs, from 1, in the old text.
    pub(in crate::ui) first: usize,
    /// The old lines replaced (empty for lines only inserted).
    pub(in crate::ui) removed: Vec<String>,
    /// The new lines (empty for lines only removed).
    pub(in crate::ui) added: Vec<String>,
}

/// Every place two texts differ, in order (Myers' line diff, from
/// `stave-lang`): a rename that rewrites three uses is three places, and
/// the lines between them are in none. Empty when they have the same
/// lines.
pub(in crate::ui) fn line_changes(before: &str, after: &str) -> Vec<LineChange> {
    let old: Vec<&str> = before.lines().collect();
    let new: Vec<&str> = after.lines().collect();
    let owned = |lines: Option<&[&str]>| {
        lines
            .unwrap_or_default()
            .iter()
            .map(|line| (*line).to_owned())
            .collect::<Vec<_>>()
    };
    stave_lang::changed_line_runs(before, after)
        .into_iter()
        .map(|(old_run, new_run)| LineChange {
            first: old_run.start.saturating_add(1),
            removed: owned(old.get(old_run)),
            added: owned(new.get(new_run)),
        })
        .collect()
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
/// share no start (it is shown as a line removed and a line added), or are
/// the same.
pub(in crate::ui) fn value_change(old: &str, new: &str) -> Option<ValueChange> {
    if old == new {
        return None;
    }
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

/// `true` when two different lines differ only in their spaces: the
/// formatter lining up a run of declarations again.
pub(in crate::ui) fn spacing_only(old: &str, new: &str) -> bool {
    old != new && old.split_whitespace().eq(new.split_whitespace())
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

/// The "Pending" section of the card in progress: the lines Apply would
/// write, as a small diff.
pub(in crate::ui) struct PendingPanel {
    /// The section, moved into the card in progress.
    pub(in crate::ui) root: gtk::Box,
    note: gtk::Label,
    lines: gtk::Box,
}

/// What would write the pending lines, for the note above them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Writer {
    /// The edit in progress, written by Apply.
    Apply,
    /// The change in progress.
    Change,
}

impl PendingPanel {
    /// The section, hidden.
    pub(in crate::ui) fn new() -> PendingPanel {
        let (root, body) = sections::section(&tr("Pending"), "", true);
        root.set_visible(false);
        let note = gtk::Label::builder()
            .xalign(0.0)
            .wrap(true)
            .css_classes(["stave-row-note"])
            .build();
        body.append(&note);
        let lines = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .css_classes(["stave-diff"])
            .build();
        body.append(
            &gtk::ScrolledWindow::builder()
                .hscrollbar_policy(gtk::PolicyType::Automatic)
                .vscrollbar_policy(gtk::PolicyType::Never)
                .child(&lines)
                .build(),
        );
        PendingPanel { root, note, lines }
    }

    fn line(&self, mark: &str, text: &str, class: &str) {
        self.parts_line(mark, &[(text, "stave-pending-text")], class);
    }

    /// A line of runs of text, each with its own class.
    fn parts_line(&self, mark: &str, parts: &[(&str, &str)], class: &str) {
        let row = gtk::Box::builder().spacing(0).css_classes([class]).build();
        row.append(
            &gtk::Label::builder()
                .label(format!("{mark} "))
                .css_classes(["stave-pending-mark"])
                .build(),
        );
        for (words, part_class) in parts {
            if words.is_empty() {
                continue;
            }
            row.append(
                &gtk::Label::builder()
                    .label(*words)
                    .xalign(0.0)
                    .selectable(false)
                    .css_classes([*part_class])
                    .build(),
            );
        }
        self.lines.append(&row);
    }

    /// A replaced value as a `−` line and a `+` line, the words that
    /// change marked in each.
    fn value_lines(&self, change: &ValueChange) {
        self.parts_line(
            "−",
            &[
                (change.head.as_str(), "stave-pending-text"),
                (change.old.as_str(), "stave-pending-old-value"),
                (change.tail.as_str(), "stave-pending-text"),
            ],
            "stave-diff-removed",
        );
        self.parts_line(
            "+",
            &[
                (change.head.as_str(), "stave-pending-text"),
                (change.new.as_str(), "stave-pending-new-value"),
                (change.tail.as_str(), "stave-pending-text"),
            ],
            "stave-diff-added",
        );
    }

    /// Shows one file's pending lines: each place it changes, in order,
    /// each after its `@ line N`.
    fn show(&self, file: &str, writer: Writer, changes: &[LineChange]) {
        while let Some(child) = self.lines.first_child() {
            self.lines.remove(&child);
        }
        if changes.is_empty() {
            self.root.set_visible(false);
            return;
        }
        let template = match (writer, changes.len()) {
            (Writer::Apply, 1) => tr("Apply would write this to {file}"),
            (Writer::Change, 1) => tr("This change would write this to {file}"),
            (Writer::Apply, _) => tr("Apply would change {file} in {places} places"),
            (Writer::Change, _) => tr("This change would change {file} in {places} places"),
        };
        self.note.set_label(&fill_in(
            &template,
            &[("file", &file), ("places", &changes.len())],
        ));
        let mut shown = 0usize;
        let mut push = |panel: &PendingPanel, f: &dyn Fn(&PendingPanel)| {
            if shown < SHOWN_LINES {
                f(panel);
            }
            shown += 1;
        };
        for change in changes {
            let place = fill_in(&tr("line {line}"), &[("line", &change.first)]);
            push(self, &|panel| panel.line("@", &place, "stave-diff-place"));
            let pairs = change.removed.len().min(change.added.len());
            for (old, new) in change.removed.iter().zip(&change.added) {
                if spacing_only(old, new) {
                    let line = fill_in(
                        &tr("{text}   (spaces only: lined up again)"),
                        &[("text", &new.trim_end())],
                    );
                    push(self, &|panel| panel.line("≈", &line, "stave-diff-place"));
                    continue;
                }
                match value_change(old, new) {
                    Some(parts) => push(self, &|panel| panel.value_lines(&parts)),
                    None => {
                        push(self, &|panel| panel.line("−", old, "stave-diff-removed"));
                        push(self, &|panel| panel.line("+", new, "stave-diff-added"));
                    }
                }
            }
            for old in change.removed.iter().skip(pairs) {
                push(self, &|panel| panel.line("−", old, "stave-diff-removed"));
            }
            for new in change.added.iter().skip(pairs) {
                push(self, &|panel| panel.line("+", new, "stave-diff-added"));
            }
        }
        if shown > SHOWN_LINES {
            self.line(
                "…",
                &fill_in(
                    &ntr(
                        "and {count} more line",
                        "and {count} more lines",
                        shown - SHOWN_LINES,
                    ),
                    &[("count", &(shown - SHOWN_LINES))],
                ),
                "stave-diff-place",
            );
        }
        self.root.set_visible(true);
    }

    /// Moves the section into `slot` (the card in progress), unless it is
    /// there already.
    fn move_into(&self, slot: &gtk::Box) {
        if self.root.parent().as_ref() == Some(slot.upcast_ref()) {
            return;
        }
        if let Some(parent) = self.root.parent().and_downcast::<gtk::Box>() {
            parent.remove(&self.root);
        }
        slot.append(&self.root);
    }

    /// The words it shows, top to bottom (for tests).
    #[cfg(test)]
    pub(in crate::ui) fn words(&self) -> Vec<String> {
        let mut words = Vec::new();
        let mut row = self.lines.first_child();
        while let Some(line) = row {
            let mut text = String::new();
            let mut part = line.first_child();
            while let Some(label) = part {
                if let Some(label) = label.downcast_ref::<gtk::Label>() {
                    text.push_str(&label.label());
                }
                part = label.next_sibling();
            }
            words.push(text);
            row = line.next_sibling();
        }
        words
    }
}

/// The lines of the old text one place strikes through, counted from 0:
/// from its first replaced line to the line after its last.
pub(in crate::ui) fn struck_lines(change: &LineChange) -> (usize, usize) {
    let first = change.first.saturating_sub(1);
    (first, first.saturating_add(change.removed.len()))
}

impl Window {
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

    /// Shows the edit's or the change's pending lines in the "Pending"
    /// section of the card in progress, or takes them away; with the
    /// editor open, its text goes to the first place and the old lines are
    /// marked.
    pub(in crate::ui) fn update_pending(&mut self) {
        let (texts, writer, editor_open) = {
            let session = &self.session;
            let writer = if session.change().is_some() {
                Writer::Change
            } else {
                Writer::Apply
            };
            (
                pending_texts(session),
                writer,
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
        let found = texts.iter().find_map(|(path, after)| {
            let before = self.text_now(path).unwrap_or_default();
            let changes = line_changes(&before, after);
            (!changes.is_empty()).then(|| (path.clone(), changes))
        });
        let Some((path, changes)) = found else {
            panel.root.set_visible(false);
            return;
        };
        let file = path
            .file_name()
            .map_or_else(String::new, |name| name.to_string_lossy().into_owned());
        // In the card that shows: the change's, else the edit's.
        if self.editing.changes.card.root.is_visible() {
            panel.move_into(&self.editing.changes.card.pending_slot);
        } else {
            panel.move_into(&self.editing.card_slots.pending_slot);
        }
        panel.show(&file, writer, &changes);
        if !editor_open {
            return;
        }
        // The text goes to the first place, and only the old lines each
        // place replaces are marked.
        if let Some(line) = changes
            .first()
            .and_then(|change| LineNumber::from_usize(change.first))
        {
            self.widgets.files.reveal(&SourceLocation {
                path: path.to_string_lossy().into_owned(),
                line,
            });
        }
        let replaced = changes.iter().any(|change| !change.removed.is_empty());
        if replaced {
            // The Code panel strikes them through itself, after the
            // reveal above has given the file its tab.
            let lines = changes
                .iter()
                .filter(|change| !change.removed.is_empty())
                .filter_map(|change| {
                    let (first, end) = struck_lines(change);
                    Some((i32::try_from(first).ok()?, i32::try_from(end).ok()?))
                })
                .collect();
            self.widgets.files.send(FilesInput::MarkPending {
                document: Document::File(path.clone()),
                lines,
            });
            self.editing.pending_marked = Some(path);
        }
    }

    /// For a rename alone (it is tried without a patch): each file it
    /// rewrites, with the name changed as a word.
    fn rename_texts(&self) -> Vec<(PathBuf, String)> {
        let session = &self.session;
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
    fn clear_pending_marks(&mut self) {
        let Some(path) = self.editing.pending_marked.take() else {
            return;
        };
        self.widgets
            .files
            .send(FilesInput::ClearPending(Document::File(path)));
    }

    // ---- while the text doesn't read ----

    /// Why editing in the 3D view is off, when it is: a file of the project
    /// that doesn't read (a syntax error), in words.
    pub(in crate::ui) fn parse_block_line(&self) -> Option<String> {
        block_line(&*self.widgets.files.analysis()?)
    }

    /// Shows why editing in 3D is off, when it is, on its own line low in
    /// the view (the floats place it just above the foot, wrapped, where
    /// it covers neither the parts list nor a card); hides it otherwise.
    pub(in crate::ui) fn show_parse_block_line(&self) {
        let line = self.parse_block_line();
        self.widgets
            .block_line
            .set_label(line.as_deref().unwrap_or_default());
        self.widgets.block_box.set_visible(line.is_some());
    }

    /// `true` while the text reads, so the 3D view's gestures may edit it.
    pub(in crate::ui) fn gestures_allowed(&self) -> bool {
        self.parse_block_line().is_none()
    }

    /// The one gate on editing in 3D while the text doesn't read: says why
    /// (a notice, beside the line low in the view) and returns `true`, so
    /// the caller does nothing; `false` while the text reads.
    ///
    /// [`Window::dispatch`] asks it before every edit command the session
    /// would act on ([`needs_readable_text`]), so a click, a script, a
    /// card, a grip, the item's tools, a hold, the finder's verbs and
    /// Delete are all refused the same way. Only what the window does
    /// before it sends any command asks it too: starting an edit (which
    /// would first drop the selection), starting a hold (which first asks
    /// for the second thing in the hint), the finder's verbs (one notice,
    /// not one per step) and Delete with nothing selected.
    pub(in crate::ui) fn refuse_while_unreadable(&self) -> bool {
        let Some(line) = self.parse_block_line() else {
            return false;
        };
        self.edit_toast(&line);
        self.show_parse_block_line();
        true
    }

    /// Whether the text reads changed, or may have (a new analysis): the
    /// line low in the view, the face button, the item's tools and the
    /// Sizes card follow it.
    pub(in crate::ui) fn readability_changed(&mut self) {
        self.update_face_button();
        self.relist_grips_soon();
    }
}

/// Whether the session acting on `command` needs the text to read: every
/// command that starts, changes or writes an edit or a change does. Only
/// dropping what is in progress, listing a selection's grips and
/// checking the open menu's entries don't.
pub(in crate::ui) fn needs_readable_text(command: &stave_app::editing::EditCommand) -> bool {
    use stave_app::editing::EditCommand;
    match command {
        EditCommand::Cancel | EditCommand::ListGrips(_) | EditCommand::CheckMenu => false,
        EditCommand::Start(_)
        | EditCommand::Choose(_)
        | EditCommand::Bind(_)
        | EditCommand::Set { .. }
        | EditCommand::PutBack
        | EditCommand::UseDefault(_)
        | EditCommand::Rename(_)
        | EditCommand::ApplyFix { .. }
        | EditCommand::Repeat(_)
        | EditCommand::RepeatItem { .. }
        | EditCommand::MoveShares { .. }
        | EditCommand::Mirror(_)
        | EditCommand::MirrorSplit { .. }
        | EditCommand::SetParam { .. }
        | EditCommand::Hold(_)
        | EditCommand::Remove(_)
        | EditCommand::IncludeDependants
        | EditCommand::RenameItem { .. }
        | EditCommand::RepeatPart { .. }
        | EditCommand::BeginDrag(_)
        | EditCommand::DragTo(_)
        | EditCommand::PickParam(_)
        | EditCommand::TypeGripValue { .. }
        | EditCommand::MakeSolved { .. }
        | EditCommand::MakePlain(_)
        | EditCommand::Add => true,
    }
}

/// Why editing in the 3D view is off for this analysis of the project, in
/// words: a file that doesn't read (a syntax error). `None` when every
/// file reads.
pub(in crate::ui) fn block_line(analysis: &Analysis) -> Option<String> {
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
            return Some(fill_in(
                &tr(
                    "Editing in 3D is off while {file} doesn't read: {problem}. Fix the text first.",
                ),
                &[("file", &name), ("problem", &diagnostic.message)],
            ));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The Pending section shows each place as `@ line N`, then the
    /// lines that go and come as `−` and `+` lines.
    #[test]
    fn the_pending_section_is_a_small_diff() {
        crate::ui::gtk_test::on_gtk_thread(|| {
            let panel = PendingPanel::new();
            let before =
                "assembly case : Case {\n  panel_stock: oak_19\n}\n\npart top : oak_19 {\n}\n";
            let after = "assembly case : Case {\n  panel_stock: maple_12\n}\n\npart top : oak_19 {\n}\npart shelf : oak_19 {\n}\n";
            panel.show("cabinet.stave", Writer::Apply, &line_changes(before, after));
            assert!(panel.root.is_visible());
            let words = panel.words();
            assert_eq!(
                words.get(..3),
                Some(
                    [
                        "@ line 2".to_owned(),
                        "−   panel_stock: oak_19".to_owned(),
                        "+   panel_stock: maple_12".to_owned(),
                    ]
                    .as_slice()
                ),
                "{words:?}"
            );
            assert!(
                words.iter().any(|line| line.starts_with("@ line "))
                    && words.contains(&"+ part shelf : oak_19 {".to_owned()),
                "{words:?}"
            );
            panel.show("cabinet.stave", Writer::Apply, &[]);
            assert!(!panel.root.is_visible());
        });
    }

    #[test]
    fn inserted_lines_are_found_between_the_common_ones() {
        let before = "a\nb\nc\n";
        let after = "a\nb\nnew one\nnew two\nc\n";
        assert_eq!(
            line_changes(before, after),
            [LineChange {
                first: 3,
                removed: Vec::new(),
                added: vec!["new one".to_owned(), "new two".to_owned()],
            }]
        );
        assert!(line_changes(before, before).is_empty());
    }

    #[test]
    fn a_replaced_value_shows_old_and_new() {
        let before = "assembly case : Case {\n  panel_stock: oak_19\n}\n";
        let after = "assembly case : Case {\n  panel_stock: maple_12\n}\n";
        let changes = line_changes(before, after);
        let [change] = changes.as_slice() else {
            panic!("one place: {changes:?}");
        };
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
        assert_eq!(value_change("same line", "same line"), None);
    }

    /// A change in two places (a repeat growing an array and its region)
    /// is two places: the lines between them, blank ones included, are not
    /// shown as changed and are not struck through.
    #[test]
    fn a_change_in_two_places_leaves_the_lines_between_alone() {
        let before = "region drawer_bay[bottom, top] = split(bay, up)\n\
                      \n\
                      stock oak_19 { a: 1 }\n\
                      \n\
                      // a note\n\
                      assembly drawer[bottom, top] : Drawer {\n\
                      }\n";
        let after = "region drawer_bay[bottom, middle, top] = split(bay, up)\n\
                     \n\
                     stock oak_19 { a: 1 }\n\
                     \n\
                     // a note\n\
                     assembly drawer[bottom, middle, top] : Drawer {\n\
                     }\n";
        let changes = line_changes(before, after);
        assert_eq!(changes.len(), 2, "{changes:?}");
        assert_eq!(changes[0].first, 1);
        assert_eq!(changes[1].first, 6);
        for change in &changes {
            assert_eq!(change.removed.len(), 1);
            assert_eq!(change.added.len(), 1);
            assert!(value_change(&change.removed[0], &change.added[0]).is_some());
        }
        assert_eq!(struck_lines(&changes[0]), (0, 1));
        assert_eq!(struck_lines(&changes[1]), (5, 6));
    }

    #[test]
    fn a_line_lined_up_again_is_spacing_only() {
        assert!(spacing_only(
            "region a     = box()",
            "region a          = box()"
        ));
        assert!(!spacing_only("region a = box()", "region a = box()"));
        assert!(!spacing_only("region a = box()", "region b = box()"));
    }

    /// A rename that rewrites every use is one place per use.
    #[test]
    fn a_rename_with_uses_is_one_place_per_use() {
        let before = "assembly drawer : Drawer {\n}\n\nx = drawer.front\n";
        let after = renamed_text(before, "drawer", "small");
        let changes = line_changes(before, &after);
        assert_eq!(
            changes
                .iter()
                .map(|change| change.first)
                .collect::<Vec<_>>(),
            [1, 4]
        );
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
        let changes = line_changes(before, after);
        let [change] = changes.as_slice() else {
            panic!("one place: {changes:?}");
        };
        assert_eq!(change.first, 2);
        assert!(change.removed.is_empty());
        assert_eq!(change.added, vec!["", "x {", "}"]);
    }
}
