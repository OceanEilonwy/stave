//! The Sizes card of the Edit panel (`docs/gui.md`, "Sizes"): the sizes
//! of the selection, one row each, a name and a number field dragged
//! sideways to change it or clicked to type a value
//! ([`ScrubField`]). Nothing is drawn on the model.
//!
//! ```text
//! ▾ Sizes                                   divider
//!   divider.height                [ 500mm ] ⋮
//!   divider.inset                 [  20mm ]
//!     follows overall_width
//! ▸ Part sizes                                    6
//! ```
//!
//! The selection's own sizes come first: an assembly's params, the
//! envelope's three sizes, a split's boundaries, and the sizes of the one
//! part selected. The other parts' lengths and widths, which follow those
//! params, are folded away under "Part sizes". The card decides nothing:
//! each row reports what its field and options menu do as the window's
//! messages, by the grip's index in the session's listing.

use adw::prelude::*;
use stave_app::gripping::ListedGrip;
use stave_app::grips::GripSize;
use stave_app::i18n::tr;
use stave_model::ids::PartId;

use super::super::sections;
use super::grip_panel::{SwitchMenu, switch_menu};
use super::grip_text::{
    GripLook, amount_words, grip_look, grip_value, label_words, row_note, solver_switches,
};
use super::grips::switch_asked;
use super::scrub_field::{ScrubEvent, ScrubField};
use crate::ui::WindowInput;
use crate::ui::update::editing::EditingInput;
use stave_app::editing::EditCommand;

/// How many part sizes (other than the selected part's) are listed with
/// the rest before they are folded away under "Part sizes".
const FOLD_PARTS_OVER: usize = 4;

/// One size's row.
struct SizeRow {
    /// The grip, in the session's listing.
    index: usize,
    /// Its number field.
    field: ScrubField,
    /// Why a typed value was refused.
    error: gtk::Label,
    /// Its options (⋮), when the solver can move it.
    menu: Option<gtk::MenuButton>,
}

/// The card: hidden while there are no sizes.
pub(super) struct SizesCard {
    /// What the Edit panel holds.
    pub(super) root: gtk::Box,
    /// The selection's own sizes.
    body: gtk::Box,
    /// What the sizes are of, in the section's head.
    note: gtk::Label,
    /// The folded section of the other parts' sizes.
    parts: gtk::Box,
    parts_body: gtk::Box,
    parts_note: gtk::Label,
    rows: Vec<SizeRow>,
    input: relm4::Sender<WindowInput>,
}

impl SizesCard {
    /// The card, hidden, its rows' messages going to `input`.
    pub(super) fn new(input: &relm4::Sender<WindowInput>) -> SizesCard {
        let root = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .css_classes(["stave-sizes"])
            .visible(false)
            .build();
        let (section, body, note) = sections::section_noted(&tr("Sizes"), true);
        root.append(&section);
        let (parts, parts_body, parts_note) = sections::section_noted(&tr("Part sizes"), false);
        root.append(&parts);
        root.append(&hint(&tr(
            "Drag a number left or right to change it, or click it to type a value.",
        )));
        SizesCard {
            root,
            body,
            note,
            parts,
            parts_body,
            parts_note,
            rows: Vec::new(),
            input: input.clone(),
        }
    }

    /// Takes every row away and hides the card.
    pub(super) fn clear(&mut self) {
        self.rows.clear();
        clear(&self.body);
        clear(&self.parts_body);
        self.root.set_visible(false);
    }

    /// Shows a row for each of `grips` (each with its index in the
    /// listing), headed `title`; `part` is the part selected, whose sizes
    /// are the selection's own.
    pub(super) fn show(
        &mut self,
        title: &str,
        grips: &[(usize, ListedGrip)],
        part: Option<PartId>,
    ) {
        self.clear();
        let others = |listed: &ListedGrip| match &listed.grip.size {
            GripSize::PartSize { part: of, .. } => Some(*of) != part,
            GripSize::Param { .. } | GripSize::RegionSize { .. } | GripSize::Boundary { .. } => {
                false
            }
        };
        // A few part sizes (a one-part assembly's length and width) stay
        // in the list; many (a drawer's sides, front, back and bottom) fold.
        let fold = grips.iter().filter(|(_, listed)| others(listed)).count() > FOLD_PARTS_OVER;
        let mut own = 0;
        let mut folded = 0;
        for (index, listed) in grips {
            let row = self.row(*index, listed);
            let line = row_widget(&row, listed, title);
            if fold && others(listed) {
                self.parts_body.append(&line);
                folded += 1;
            } else {
                self.body.append(&line);
                own += 1;
            }
            self.rows.push(row);
        }
        self.note.set_label(title);
        self.note.set_visible(!title.is_empty());
        self.parts_note.set_label(&folded.to_string());
        self.parts_note.set_visible(folded > 0);
        self.parts.set_visible(folded > 0);
        if own == 0 && folded > 0 {
            self.body.append(&sections::words(
                &tr("Its sizes follow its parts' sizes: open Part sizes."),
                &["stave-row-note"],
            ));
        }
        self.root.set_visible(!grips.is_empty());
    }

    /// One row's field and options.
    fn row(&self, index: usize, listed: &ListedGrip) -> SizeRow {
        let look = grip_look(listed);
        let input = self.input.clone();
        let field = ScrubField::new(
            &listed.grip.label,
            &amount_words(grip_value(listed)),
            look != GripLook::Locked,
            move |event| input.emit(heard(index, event).into()),
        );
        let error = gtk::Label::builder()
            .css_classes(["stave-sizes-error"])
            .xalign(0.0)
            .wrap(true)
            .visible(false)
            .build();
        let switches = solver_switches(listed);
        let menu = (!switches.is_empty() && look != GripLook::Locked).then(|| {
            let input = self.input.clone();
            let menu = switch_menu(&switches, move |ask, menu| {
                input.emit(switch_asked(index, ask, SwitchMenu::Row(menu)).into());
            });
            menu.set_valign(gtk::Align::Center);
            menu
        });
        SizeRow {
            index,
            field,
            error,
            menu,
        }
    }

    /// Shows `value` in grip `index`'s field.
    pub(super) fn set_value(&self, index: usize, value: &str) {
        if let Some(row) = self.find(index) {
            row.field.set_value(value);
        }
    }

    /// Marks grip `index`'s field as scrubbed, and no other.
    pub(super) fn set_scrubbing(&self, index: Option<usize>) {
        for row in &self.rows {
            row.field.set_scrubbing(Some(row.index) == index);
        }
    }

    /// Turns grip `index`'s field into its entry.
    pub(super) fn open_entry(&self, index: usize) {
        for row in &self.rows {
            if row.index != index && row.field.typing() {
                close_row(row);
            }
        }
        if let Some(row) = self.find(index) {
            let value = row.field.value();
            row.field.open_entry(&value, false);
        }
    }

    /// Grip `index`'s field back to its number.
    pub(super) fn close_entry(&self, index: usize) {
        if let Some(row) = self.find(index) {
            close_row(row);
        }
    }

    /// Every field back to its number.
    pub(super) fn close_entries(&self) {
        for row in &self.rows {
            close_row(row);
        }
    }

    /// Grip `index`'s entry stays open holding `text`, marked, with why
    /// it was refused under it.
    pub(super) fn show_refusal(&self, index: usize, text: &str, why: &str) {
        if let Some(row) = self.find(index) {
            row.field.open_entry(text, true);
            row.error.set_label(why);
            row.error.set_visible(true);
        }
    }

    /// Types `text` into grip `index`'s entry, for a script.
    pub(super) fn type_text(&self, index: usize, text: &str) {
        if let Some(row) = self.find(index) {
            row.field.type_text(text);
        }
    }

    /// Opens grip `index`'s options (⋮); `false` when it has none.
    pub(super) fn open_menu(&self, index: usize) -> bool {
        let Some(menu) = self.find(index).and_then(|row| row.menu.as_ref()) else {
            return false;
        };
        menu.popup();
        true
    }

    /// The indices of the grips with a row, in order, for a test.
    #[cfg(test)]
    pub(super) fn indices(&self) -> Vec<usize> {
        self.rows.iter().map(|row| row.index).collect()
    }

    /// What grip `index`'s field shows, for a test.
    #[cfg(test)]
    pub(super) fn value_of(&self, index: usize) -> Option<String> {
        self.find(index).map(|row| row.field.value())
    }

    fn find(&self, index: usize) -> Option<&SizeRow> {
        self.rows.iter().find(|row| row.index == index)
    }
}

/// What a field's event asks of the window, for grip `index`.
fn heard(index: usize, event: ScrubEvent) -> EditingInput {
    match event {
        ScrubEvent::Began => EditingInput::GripPressed(index),
        ScrubEvent::Moved { dx, fine, round } => EditingInput::GripMoved { dx, fine, round },
        ScrubEvent::Ended => EditingInput::GripReleased,
        ScrubEvent::Clicked => EditingInput::OpenGripEntry(index),
        ScrubEvent::Typed(text) => EditingInput::GripTyped { index, text },
        ScrubEvent::Escaped => EditingInput::CloseGripTyping(index),
        ScrubEvent::Stepped { steps, round } => EditingInput::GripStepped {
            index,
            steps,
            round,
        },
        ScrubEvent::Enter => EditingInput::SendEditLater(EditCommand::Add),
    }
}

/// A row as laid out: the name (and what it follows, under it), the
/// field and its options; the reason a typed value was refused below.
fn row_widget(row: &SizeRow, listed: &ListedGrip, title: &str) -> gtk::Box {
    let names = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .hexpand(true)
        .valign(gtk::Align::Center)
        .build();
    // The card's head names what the sizes are of: `apron_front.setback`
    // is listed as `setback`.
    let label = &listed.grip.label;
    let short = label
        .strip_prefix(title)
        .and_then(|rest| rest.strip_prefix('.'))
        .filter(|rest| !rest.is_empty())
        .unwrap_or(label);
    names.append(
        &gtk::Label::builder()
            .label(short)
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::Start)
            .tooltip_text(format!("{}: {}", listed.grip.label, label_words(listed)))
            .css_classes(["stave-sizes-name"])
            .build(),
    );
    if let Some(words) = row_note(listed) {
        names.append(
            &gtk::Label::builder()
                .label(&words)
                .xalign(0.0)
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .tooltip_text(&words)
                .css_classes(["stave-row-note"])
                .build(),
        );
    }
    let line = gtk::Box::builder()
        .spacing(6)
        .css_classes(["stave-sizes-row"])
        .build();
    line.append(&names);
    // As wide on every row, whatever the name's length.
    let field = gtk::Box::builder()
        .width_request(112)
        .hexpand(false)
        .valign(gtk::Align::Center)
        .build();
    field.append(&row.field.root);
    line.append(&field);
    let options = gtk::Box::builder()
        .width_request(24)
        .valign(gtk::Align::Center)
        .build();
    if let Some(menu) = &row.menu {
        options.append(menu);
    }
    line.append(&options);
    let holder = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .build();
    holder.append(&line);
    holder.append(&row.error);
    holder
}

/// The line under the sizes saying how to change one: wrapped, its text
/// at the top of whatever height it is given.
fn hint(text: &str) -> gtk::Label {
    gtk::Label::builder()
        .label(text)
        .xalign(0.0)
        .yalign(0.0)
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .natural_wrap_mode(gtk::NaturalWrapMode::Word)
        .vexpand(false)
        .css_classes(["stave-row-note", "stave-sizes-hint"])
        .build()
}

fn close_row(row: &SizeRow) {
    row.field.close_entry();
    row.error.set_visible(false);
}

fn clear(container: &gtk::Box) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
}

/// The card's look.
pub(super) const STYLE: &str = r#"
.stave-sizes-row { min-height: 26px; padding: 1px 0; }
.stave-sizes-name { font-family: var(--monospace-font-family, monospace); font-size: 0.818rem; color: #99c1f1; }
.stave-sizes-error { color: @error_color; font-size: 0.784rem; font-weight: 600; padding: 0 0 4px 0; }
.stave-sizes-hint { padding: 4px 8px 8px 8px; }
"#;
