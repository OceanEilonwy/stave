//! The Sizes card of the Edit panel (`docs/gui.md`, "Sizes"): the sizes
//! of the selection, one compact row each, and, for the one row clicked,
//! the size editor in its place: the value in a large field with how far
//! it moved, a slider with the size's stops, buttons that step it, what
//! else changes, the verdict, and Cancel / Apply. Nothing is written
//! until Apply. Nothing is drawn on the model.
//!
//! ```text
//! ▾ Sizes                                   divider
//!   height                        500mm  ›          a row: click to edit
//!   ┌ height                      was 500mm ⋮ ┐
//!   │ [ 540mm ]  +40mm                          │    the editor
//!   │ ////|━━━━━━━|━━━━━●──────────|////        │
//!   │ 180mm                            900mm    │
//!   │ [−10] [−1] [+1] [+10] [back to 500mm]     │
//!   │ What changes   divider.height 500 → 540mm │
//!   │ It fits                                   │
//!   │                      [Cancel] [Apply]     │
//!   └───────────────────────────────────────────┘
//!   depth                     🔒  9mm             locked: can't be opened
//! ▸ Part sizes                                    6
//! ```
//!
//! The card decides nothing: rows and the editor report what is clicked,
//! dragged or typed as the window's messages, by the grip's index in the
//! session's listing, and show what the window hands them
//! ([`EditorView`]).

use adw::prelude::*;
use gtk::{gdk, glib};
use stave_app::gripping::ListedGrip;
use stave_app::grips::GripSize;
use stave_app::i18n::{fill_in, tr};
use stave_model::ids::{ParamId, PartId};

use super::super::sections;
use super::grip_panel::{PanelAction, SwitchMenu, switch_menu};
use super::grip_text::{
    GripLook, PanelChoice, SolverSwitch, amount_words, grip_look, grip_value, label_words, row_note,
};
use super::grips::switch_asked;
use super::size_slider::{SizeSlider, SliderEvent, SliderLook, Tone};
use crate::ui::WindowInput;
use crate::ui::update::editing::EditingInput;

/// How many part sizes (other than the selected part's) are listed with
/// the rest before they are folded away under "Part sizes".
const FOLD_PARTS_OVER: usize = 4;

/// What the editor shows, worked out by the window from the session.
#[derive(Debug, Clone, Default)]
pub(super) struct EditorView {
    /// The grip, in the listing.
    pub(super) index: usize,
    /// The size's name, shortened as its row shows it.
    pub(super) name: String,
    /// What it was: "500mm".
    pub(super) was: String,
    /// The value now, for the field (left alone while it is typed in).
    pub(super) value: String,
    /// How far it moved: "+40mm"; empty when it hasn't.
    pub(super) delta: String,
    /// The slider, when the size has a range to show.
    pub(super) slider: Option<SliderLook>,
    /// The slider's low end, in words.
    pub(super) low: String,
    /// The slider's high end, in words.
    pub(super) high: String,
    /// What else changes: "divider.height 500mm → 540mm".
    pub(super) changes: Vec<String>,
    /// The sizes to pick from, when the size follows several.
    pub(super) choices: Vec<PanelChoice>,
    /// The verdict in words.
    pub(super) verdict: String,
    /// Where the change stands.
    pub(super) tone: Tone,
    /// Apply would write it.
    pub(super) can_apply: bool,
    /// Why the value typed was refused, under the field.
    pub(super) refusal: Option<String>,
    /// The ⋮ menu's switches ("Let the solver move this").
    pub(super) switches: Vec<SolverSwitch>,
}

/// One size's row.
struct SizeRow {
    /// The grip, in the session's listing.
    index: usize,
    /// The row and, when it is open, the editor in its place.
    holder: gtk::Box,
    /// The row's button (hidden while the editor is open on it).
    button: gtk::Button,
    /// Its value.
    value: gtk::Label,
}

/// The size editor's widgets, made once and put in the open row.
struct SizeEditor {
    root: gtk::Box,
    name: gtk::Label,
    was: gtk::Label,
    options: gtk::Box,
    options_for: Option<usize>,
    entry: gtk::Entry,
    delta: gtk::Label,
    refusal: gtk::Label,
    slider: SizeSlider,
    ends: gtk::Box,
    low: gtk::Label,
    high: gtk::Label,
    back: gtk::Button,
    changes: gtk::Box,
    /// The lines the box shows now.
    shown_changes: Vec<String>,
    choices: gtk::Box,
    shown_choices: Vec<PanelChoice>,
    verdict: gtk::Box,
    spinner: adw::Spinner,
    verdict_words: gtk::Label,
    apply: gtk::Button,
    /// The value last put in the field, so a redraw doesn't undo typing.
    shown_value: String,
}

/// The card: hidden while there are no sizes.
pub(super) struct SizesCard {
    /// What the Edit panel holds.
    pub(super) root: gtk::Box,
    body: gtk::Box,
    note: gtk::Label,
    parts: gtk::Box,
    parts_body: gtk::Box,
    parts_note: gtk::Label,
    rows: Vec<SizeRow>,
    editor: SizeEditor,
    /// The row the editor is open on.
    open: Option<usize>,
    input: relm4::Sender<WindowInput>,
}

impl SizesCard {
    /// The card, hidden, its rows' and editor's messages going to `input`.
    pub(super) fn new(input: &relm4::Sender<WindowInput>) -> SizesCard {
        let root = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .css_classes(["stave-sizes"])
            .visible(false)
            .build();
        let (section, body, note) = sections::section_noted(&tr("Sizes"), true);
        body.add_css_class("stave-sizes-body");
        root.append(&section);
        let (parts, parts_body, parts_note) = sections::section_noted(&tr("Part sizes"), false);
        parts_body.add_css_class("stave-sizes-body");
        root.append(&parts);
        root.append(&hint(&tr(
            "Click a size to change it. Nothing is written until Apply.",
        )));
        SizesCard {
            root,
            body,
            note,
            parts,
            parts_body,
            parts_note,
            rows: Vec::new(),
            editor: SizeEditor::new(input),
            open: None,
            input: input.clone(),
        }
    }

    /// Takes every row away and hides the card.
    pub(super) fn clear(&mut self) {
        self.close_editor();
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
            let row = self.row(*index, listed, title);
            if fold && others(listed) {
                self.parts_body.append(&row.holder);
                folded += 1;
            } else {
                self.body.append(&row.holder);
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

    /// One row: the name (and what it follows, under it), the value, and
    /// an arrow; a size nothing can move shows a padlock and can't be
    /// opened.
    fn row(&self, index: usize, listed: &ListedGrip, title: &str) -> SizeRow {
        let locked = grip_look(listed) == GripLook::Locked;
        let label = &listed.grip.label;
        let names = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .hexpand(true)
            .valign(gtk::Align::Center)
            .build();
        names.append(
            &gtk::Label::builder()
                .label(short_name(label, title))
                .xalign(0.0)
                .ellipsize(gtk::pango::EllipsizeMode::Start)
                .css_classes(["stave-sizes-name"])
                .build(),
        );
        if let Some(words) = row_note(listed) {
            names.append(
                &gtk::Label::builder()
                    .label(&words)
                    .xalign(0.0)
                    .ellipsize(gtk::pango::EllipsizeMode::End)
                    .css_classes(["stave-row-note"])
                    .build(),
            );
        }
        let line = gtk::Box::builder().spacing(10).build();
        line.append(&names);
        let value = gtk::Label::builder()
            .label(amount_words(grip_value(listed)))
            .valign(gtk::Align::Center)
            .css_classes(["stave-sizes-value"])
            .build();
        if locked {
            line.append(
                &gtk::Image::builder()
                    .icon_name("changes-prevent-symbolic")
                    .pixel_size(12)
                    .valign(gtk::Align::Center)
                    .css_classes(["stave-sizes-lock"])
                    .build(),
            );
        }
        line.append(&value);
        line.append(
            &gtk::Image::builder()
                .icon_name("go-next-symbolic")
                .pixel_size(12)
                .opacity(if locked { 0.0 } else { 0.55 })
                .valign(gtk::Align::Center)
                .build(),
        );
        let button = gtk::Button::builder()
            .child(&line)
            .css_classes(["flat", "stave-sizes-row"])
            .tooltip_text(format!("{label}: {}", label_words(listed)))
            .sensitive(!locked)
            .build();
        button.update_property(&[gtk::accessible::Property::Label(&fill_in(
            &tr("Change {size}"),
            &[("size", label)],
        ))]);
        let input = self.input.clone();
        button.connect_clicked(move |_| input.emit(EditingInput::OpenSize(index).into()));
        let holder = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .build();
        holder.append(&button);
        SizeRow {
            index,
            holder,
            button,
            value,
        }
    }

    /// Shows `value` in grip `index`'s row.
    pub(super) fn set_row_value(&self, index: usize, value: &str) {
        if let Some(row) = self.find(index)
            && row.value.label() != value
        {
            row.value.set_label(value);
        }
    }

    /// Marks grip `index`'s row as just written, for a moment.
    pub(super) fn flash(&self, index: usize) {
        if let Some(row) = self.find(index) {
            row.button.add_css_class("written");
            let button = row.button.downgrade();
            glib::timeout_add_local_once(std::time::Duration::from_millis(1600), move || {
                if let Some(button) = button.upgrade() {
                    button.remove_css_class("written");
                }
            });
        }
    }

    /// The row the editor is open on.
    #[cfg(test)]
    pub(super) fn open(&self) -> Option<usize> {
        self.open
    }

    /// Opens the editor on grip `index`'s row, showing `view`, the
    /// keyboard in its field.
    pub(super) fn open_editor(&mut self, view: &EditorView) {
        self.close_editor();
        let Some(row) = self.rows.iter().find(|row| row.index == view.index) else {
            return;
        };
        row.button.set_visible(false);
        row.holder.append(&self.editor.root);
        self.open = Some(view.index);
        self.editor.shown_value.clear();
        self.editor.show(view, &self.input, true);
        self.editor.entry.grab_focus();
        self.editor.entry.select_region(0, -1);
    }

    /// Shows `view` in the open editor.
    pub(super) fn update_editor(&mut self, view: &EditorView) {
        if self.open == Some(view.index) {
            self.editor.show(view, &self.input, false);
        }
    }

    /// Closes the editor; its row comes back.
    pub(super) fn close_editor(&mut self) {
        if let Some(index) = self.open.take()
            && let Some(row) = self.find(index)
        {
            row.button.set_visible(true);
        }
        if let Some(parent) = self.editor.root.parent().and_downcast::<gtk::Box>() {
            parent.remove(&self.editor.root);
        }
    }

    /// Types `text` into the open editor's field, for a script.
    pub(super) fn type_text(&self, text: &str) {
        self.editor.entry.set_text(text);
    }

    /// Opens the open editor's options (⋮); `false` when it has none.
    pub(super) fn open_menu(&self) -> bool {
        let Some(menu) = self
            .editor
            .options
            .first_child()
            .and_downcast::<gtk::MenuButton>()
        else {
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

    /// What grip `index`'s row shows, for a test.
    #[cfg(test)]
    pub(super) fn value_of(&self, index: usize) -> Option<String> {
        self.find(index).map(|row| row.value.label().to_string())
    }

    /// What the open editor says: its field, its change, its verdict, and
    /// whether Apply is on, for a test.
    #[cfg(test)]
    pub(super) fn editor_words(&self) -> (String, String, String, bool) {
        (
            self.editor.entry.text().to_string(),
            self.editor.delta.label().to_string(),
            self.editor.verdict_words.label().to_string(),
            self.editor.apply.is_sensitive(),
        )
    }

    fn find(&self, index: usize) -> Option<&SizeRow> {
        self.rows.iter().find(|row| row.index == index)
    }
}

impl SizeEditor {
    /// The editor's widgets, wired to `input`.
    fn new(input: &relm4::Sender<WindowInput>) -> SizeEditor {
        let root = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(10)
            .css_classes(["stave-size-editor"])
            .build();
        // The head: the name, what it was, the ⋮ menu.
        let head = gtk::Box::builder().spacing(8).build();
        let name = gtk::Label::builder()
            .xalign(0.0)
            .hexpand(true)
            .ellipsize(gtk::pango::EllipsizeMode::Start)
            .css_classes(["stave-sizes-name"])
            .build();
        let was = gtk::Label::builder()
            .css_classes(["stave-row-note"])
            .build();
        let options = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        head.append(&name);
        head.append(&was);
        head.append(&options);
        root.append(&head);
        // The value, large, and how far it moved.
        let value_line = gtk::Box::builder().spacing(10).build();
        let entry = gtk::Entry::builder()
            .width_chars(8)
            .css_classes(["stave-size-entry"])
            .tooltip_text(tr("Type a value and press Enter"))
            .build();
        entry.update_property(&[gtk::accessible::Property::Label(&tr("Value"))]);
        // A fixed width: "+6mm" and "+1234mm" must not push the field.
        let delta = gtk::Label::builder()
            .valign(gtk::Align::Center)
            .xalign(0.0)
            .width_chars(10)
            .css_classes(["stave-size-delta"])
            .build();
        value_line.append(&entry);
        value_line.append(&delta);
        root.append(&value_line);
        let refusal = gtk::Label::builder()
            .xalign(0.0)
            .wrap(true)
            .visible(false)
            .css_classes(["stave-sizes-error"])
            .build();
        root.append(&refusal);
        // The slider and its ends.
        let slider = SizeSlider::new(&tr("Value"), input, slider_said);
        let slider_box = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .build();
        slider_box.append(&slider.area);
        let ends = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        let low = gtk::Label::builder()
            .xalign(0.0)
            .hexpand(true)
            .css_classes(["stave-size-end"])
            .build();
        let high = gtk::Label::builder()
            .xalign(1.0)
            .css_classes(["stave-size-end"])
            .build();
        ends.append(&low);
        ends.append(&high);
        slider_box.append(&ends);
        root.append(&slider_box);
        // The steps.
        let steps = gtk::Box::builder().spacing(6).build();
        for units in [-10_i32, -1, 1, 10] {
            let words = if units > 0 {
                format!("+{units}")
            } else {
                format!("−{}", units.unsigned_abs())
            };
            let button = gtk::Button::builder()
                .label(&words)
                .css_classes(["stave-size-step"])
                .build();
            let input = input.clone();
            button.connect_clicked(move |_| input.emit(EditingInput::SizeNudged(units).into()));
            steps.append(&button);
        }
        let back = gtk::Button::builder()
            .css_classes(["stave-size-step"])
            .build();
        {
            let input = input.clone();
            back.connect_clicked(move |_| input.emit(EditingInput::SizeReset.into()));
        }
        steps.append(&back);
        root.append(&steps);
        // What changes, and the sizes to pick from.
        let changes = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .visible(false)
            .build();
        root.append(&changes);
        let choices = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .accessible_role(gtk::AccessibleRole::RadioGroup)
            .visible(false)
            .build();
        choices.update_property(&[gtk::accessible::Property::Label(&tr("Which size changes?"))]);
        root.append(&choices);
        // The verdict.
        let verdict = gtk::Box::builder()
            .spacing(8)
            .css_classes(["stave-size-verdict"])
            .build();
        let spinner = adw::Spinner::builder().visible(false).build();
        let verdict_words = gtk::Label::builder()
            .xalign(0.0)
            .wrap(true)
            .hexpand(true)
            .build();
        verdict.append(&spinner);
        verdict.append(&verdict_words);
        root.append(&verdict);
        // Cancel and Apply.
        let footer = gtk::Box::builder()
            .spacing(8)
            .halign(gtk::Align::End)
            .build();
        let cancel = gtk::Button::builder()
            .label(tr("Cancel"))
            .tooltip_text(tr("Esc"))
            .build();
        let apply = gtk::Button::builder()
            .label(tr("Apply"))
            .tooltip_text(tr("Enter"))
            .css_classes(["suggested-action"])
            .sensitive(false)
            .build();
        {
            let input = input.clone();
            cancel.connect_clicked(move |_| input.emit(EditingInput::CloseSize.into()));
        }
        {
            let input = input.clone();
            apply.connect_clicked(move |_| input.emit(EditingInput::SizeApply.into()));
        }
        footer.append(&cancel);
        footer.append(&apply);
        root.append(&footer);
        // Enter in the field tries what is typed (or applies it, when it
        // is what was tried); Escape anywhere in the editor closes it.
        {
            let input = input.clone();
            entry.connect_activate(move |entry| {
                input.emit(EditingInput::SizeTyped(entry.text().trim().to_owned()).into());
            });
        }
        let keys = gtk::EventControllerKey::new();
        {
            let input = input.clone();
            keys.connect_key_pressed(move |_, key, _, _| {
                if key == gdk::Key::Escape {
                    input.emit(EditingInput::CloseSize.into());
                    return glib::Propagation::Stop;
                }
                glib::Propagation::Proceed
            });
        }
        root.add_controller(keys);
        SizeEditor {
            root,
            name,
            was,
            options,
            options_for: None,
            entry,
            delta,
            refusal,
            slider,
            ends,
            low,
            high,
            back,
            changes,
            shown_changes: Vec::new(),
            choices,
            shown_choices: Vec::new(),
            verdict,
            spinner,
            verdict_words,
            apply,
            shown_value: String::new(),
        }
    }

    /// Shows `view`; `opened` when the editor was just opened on it.
    fn show(&mut self, view: &EditorView, input: &relm4::Sender<WindowInput>, opened: bool) {
        self.name.set_label(&view.name);
        self.was
            .set_label(&fill_in(&tr("was {value}"), &[("value", &view.was)]));
        if opened || self.options_for != Some(view.index) {
            clear(&self.options);
            self.options_for = Some(view.index);
            if !view.switches.is_empty() {
                let input = input.clone();
                let index = view.index;
                let menu = switch_menu(&view.switches, move |ask, menu| {
                    input.emit(switch_asked(index, ask, SwitchMenu::Row(menu)).into());
                });
                menu.set_valign(gtk::Align::Center);
                self.options.append(&menu);
            }
        }
        // The field keeps what is being typed in it.
        if self.shown_value != view.value && (opened || !self.entry.has_focus()) {
            self.entry.set_text(&view.value);
        }
        self.shown_value.clone_from(&view.value);
        self.delta.set_label(&view.delta);
        for tone in [Tone::Waiting, Tone::Checking, Tone::Fits, Tone::Refused] {
            self.delta.remove_css_class(tone.class());
            self.verdict.remove_css_class(tone.class());
            self.entry.remove_css_class(tone.class());
        }
        self.delta.add_css_class(view.tone.class());
        self.verdict.add_css_class(view.tone.class());
        if !view.delta.is_empty() {
            self.entry.add_css_class(view.tone.class());
        }
        match &view.refusal {
            Some(why) => {
                self.refusal.set_label(why);
                self.refusal.set_visible(true);
                self.entry.add_css_class("error");
            }
            None => {
                self.refusal.set_visible(false);
                self.entry.remove_css_class("error");
            }
        }
        match view.slider {
            Some(look) => {
                self.slider.show(look);
                self.slider.area.set_visible(true);
                self.ends.set_visible(true);
            }
            None => {
                self.slider.area.set_visible(false);
                self.ends.set_visible(false);
            }
        }
        self.low.set_label(&view.low);
        self.high.set_label(&view.high);
        self.back
            .set_label(&fill_in(&tr("back to {value}"), &[("value", &view.was)]));
        self.back.set_sensitive(!view.delta.is_empty());
        // While a check runs the lines stay as they were, and are only
        // rebuilt when they change: the card keeps its height through
        // a drag instead of shrinking and growing with each check.
        let settling =
            view.tone == Tone::Checking && !view.delta.is_empty() && !self.shown_changes.is_empty();
        if !settling && self.shown_changes != view.changes {
            clear(&self.changes);
            if !view.changes.is_empty() {
                self.changes.append(&sections::words(
                    &tr("What changes"),
                    &["stave-size-heading"],
                ));
                for line in &view.changes {
                    self.changes
                        .append(&sections::words(line, &["stave-grip-change"]));
                }
            }
            self.changes.set_visible(!view.changes.is_empty());
            self.shown_changes.clone_from(&view.changes);
        }
        if self.shown_choices != view.choices {
            self.show_choices(&view.choices, input);
            self.shown_choices.clone_from(&view.choices);
        }
        self.spinner.set_visible(view.tone == Tone::Checking);
        self.verdict_words.set_label(&view.verdict);
        self.apply.set_sensitive(view.can_apply);
    }

    /// The sizes to pick from, as radios: the name, and the value it would
    /// take on the right.
    fn show_choices(&self, choices: &[PanelChoice], input: &relm4::Sender<WindowInput>) {
        clear(&self.choices);
        if choices.is_empty() {
            self.choices.set_visible(false);
            return;
        }
        self.choices.append(&sections::words(
            &tr("Which size changes?"),
            &["stave-size-heading"],
        ));
        let mut group: Option<gtk::CheckButton> = None;
        for choice in choices {
            let row = gtk::Box::builder()
                .spacing(6)
                .css_classes(["stave-radio-row"])
                .build();
            row.append(
                &gtk::Label::builder()
                    .label(&choice.name)
                    .xalign(0.0)
                    .hexpand(true)
                    .ellipsize(gtk::pango::EllipsizeMode::End)
                    .css_classes(["stave-radio-name"])
                    .build(),
            );
            row.append(
                &gtk::Label::builder()
                    .label(&choice.value)
                    .xalign(1.0)
                    .css_classes(["stave-radio-value"])
                    .build(),
            );
            let radio = gtk::CheckButton::builder()
                .child(&row)
                .active(choice.picked)
                .sensitive(choice.possible)
                .tooltip_text(&choice.words)
                .build();
            radio.update_property(&[gtk::accessible::Property::Label(&choice.words)]);
            if let Some(first) = &group {
                radio.set_group(Some(first));
            } else {
                group = Some(radio.clone());
            }
            let input = input.clone();
            let param: ParamId = choice.param;
            radio.connect_toggled(move |radio| {
                if radio.is_active() {
                    input.emit(EditingInput::GripPanel(PanelAction::Pick(param)).into());
                }
            });
            self.choices.append(&radio);
        }
        self.choices.set_visible(true);
    }
}

/// What people did on the size editor's slider, as the window's message.
fn slider_said(event: SliderEvent) -> WindowInput {
    match event {
        SliderEvent::Moved(position) => EditingInput::SizeSlid {
            position,
            released: false,
        },
        SliderEvent::Released(position) => EditingInput::SizeSlid {
            position,
            released: true,
        },
        SliderEvent::Nudged(units) => EditingInput::SizeNudged(units),
        SliderEvent::Enter => EditingInput::SizeApply,
        SliderEvent::Escape => EditingInput::CloseSize,
    }
    .into()
}

/// `label` without the selection's name in front: `apron_front.setback`
/// under `apron_front` is `setback`.
pub(super) fn short_name<'a>(label: &'a str, title: &str) -> &'a str {
    label
        .strip_prefix(title)
        .and_then(|rest| rest.strip_prefix('.'))
        .filter(|rest| !rest.is_empty())
        .unwrap_or(label)
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

fn clear(container: &gtk::Box) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
}

/// The card's look.
pub(super) const STYLE: &str = r#"
.stave-sizes-body { padding: 4px 4px 6px 4px; }
button.stave-sizes-row { padding: 4px 8px; min-height: 30px; border-radius: 8px; font-weight: normal; }
button.stave-sizes-row:hover { background: alpha(currentColor, 0.07); }
button.stave-sizes-row.written { background: alpha(@success_color, 0.18); }
button.stave-sizes-row.written .stave-sizes-value { color: @success_color; }
button.stave-sizes-row:disabled { opacity: 0.75; }
.stave-sizes-name { font-family: var(--monospace-font-family, monospace); font-size: 0.818rem; color: #99c1f1; }
.stave-sizes-value { font-family: var(--monospace-font-family, monospace); font-size: 0.818rem; font-weight: 600;
  padding: 2px 8px; border-radius: 6px; background: alpha(black, 0.22); }
.stave-sizes-lock { opacity: 0.6; }
.stave-sizes-error { color: @error_color; font-size: 0.784rem; font-weight: 600; }
.stave-sizes-hint { padding: 4px 8px 8px 8px; }
.stave-size-editor { margin: 4px 0 8px 0; padding: 12px; border-radius: 10px; background: alpha(currentColor, 0.05);
  border: 1px solid alpha(currentColor, 0.12); }
entry.stave-size-entry { min-height: 40px; font-family: var(--monospace-font-family, monospace); font-size: 1.4rem;
  font-weight: 600; border: 2px solid alpha(@accent_color, 0.8); }
entry.stave-size-entry.checking { border-color: #99c1f1; }
entry.stave-size-entry.fits { border-color: @success_color; }
entry.stave-size-entry.refused, entry.stave-size-entry.error { border-color: @error_color; }
.stave-size-delta { font-family: var(--monospace-font-family, monospace); font-weight: 600; }
.stave-size-delta.checking { color: #99c1f1; }
.stave-size-delta.fits { color: @success_color; }
.stave-size-delta.refused { color: @error_color; }
.stave-size-end { font-family: var(--monospace-font-family, monospace); font-size: 0.7rem; opacity: 0.6; }
button.stave-size-step { font-family: var(--monospace-font-family, monospace); font-size: 0.784rem; padding: 2px 9px;
  min-height: 24px; }
.stave-size-heading { font-size: 0.784rem; font-weight: 700; opacity: 0.7; }
.stave-size-verdict { padding: 8px 10px; min-height: 24px; border-radius: 8px; font-size: 0.85rem; font-weight: 600;
  background: alpha(currentColor, 0.06); }
.stave-size-verdict.checking { color: #99c1f1; background: alpha(#99c1f1, 0.1); }
.stave-size-verdict.fits { color: @success_color; background: alpha(@success_color, 0.14); }
.stave-size-verdict.refused { color: @error_color; background: alpha(@error_color, 0.14); }
"#;
