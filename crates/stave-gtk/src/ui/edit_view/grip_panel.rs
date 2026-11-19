//! The size a grip moves, as a section of the change's card in the Edit
//! panel (`docs/gui.md`, "Grips"): no card floats beside the handle. It
//! says what is moved and to what (a field where a value can be typed),
//! what the size follows, what else changes, the sizes to pick from as a
//! radio list (one row each: the radio, the name, the value), the range
//! and the rule a stop names, and holds the grip's options (⋮: let the
//! solver move it, with its range). The verdict and Apply / Cancel are
//! the change card's own. It is drawn from a [`PanelModel`] and reports
//! what is clicked or typed; it decides nothing.
//!
//! ```text
//! ▾ Size
//!   Grip         case.base.length
//!   Value        [1000mm    ] was 900mm  ⋮
//!   Follows      overall_width
//!   Which size changes?
//!   (•) overall_width             940 → 1040 mm
//!   ( ) case.overhang              20 → −30 mm
//!   Stops        stops at 1200mm, because …
//! ```

use stave_app::i18n::{fill_in, tr};
use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::gdk;
use stave_model::ids::ParamId;

use super::super::sections;
use super::grip_text::PanelModel;

/// What the panel reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum PanelAction {
    /// One of the sizes to pick from.
    Pick(ParamId),
    /// A value typed in its field, Enter pressed.
    Type(String),
}

type ActionCallback = Rc<dyn Fn(PanelAction)>;

/// The section.
pub(super) struct GripPanel {
    /// The section, put into the change card's grip slot.
    pub(super) root: gtk::Box,
    note: gtk::Label,
    value: gtk::Entry,
    /// "was 900mm", after the field.
    was: gtk::Label,
    /// Where the window puts the grip's options (⋮).
    pub(super) options: gtk::Box,
    about: gtk::Label,
    about_label: gtk::Label,
    about_row: gtk::Box,
    changes: gtk::Box,
    choices: gtk::Box,
    stop: gtk::Label,
    stop_row: gtk::Box,
    waiting: gtk::Label,
    callback: Rc<RefCell<Option<ActionCallback>>>,
    /// The value last shown, so a redraw doesn't undo typing.
    shown_value: RefCell<String>,
}

impl GripPanel {
    /// The section, hidden.
    pub(super) fn new() -> GripPanel {
        let (root, body) = sections::section(&tr("Size"), "", true);
        root.set_visible(false);
        // What is moved: the grip.
        let note = gtk::Label::builder()
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::Start)
            .css_classes(["stave-field-value", "stave-grip-title"])
            .build();
        body.append(&sections::row(&tr("Grip"), &note));
        let value = gtk::Entry::builder()
            .hexpand(true)
            .width_chars(6)
            .css_classes(["stave-grip-value"])
            .tooltip_text(tr("Type a value and press Enter"))
            .build();
        value.update_property(&[gtk::accessible::Property::Label(&tr("Value"))]);
        let options = gtk::Box::builder().valign(gtk::Align::Center).build();
        let was = gtk::Label::builder()
            .css_classes(["stave-row-note"])
            .build();
        let value_line = gtk::Box::builder().spacing(4).hexpand(true).build();
        value_line.append(&value);
        value_line.append(&was);
        value_line.append(&options);
        body.append(&sections::row(&tr("Value"), &value_line));
        let about = sections::value("");
        let about_label = sections::row_label(&tr("Follows"));
        let about_row = gtk::Box::builder()
            .spacing(6)
            .css_classes(["stave-field-row"])
            .build();
        about_row.append(&about_label);
        about.set_hexpand(true);
        about_row.append(&about);
        body.append(&about_row);
        let changes = gtk::Box::new(gtk::Orientation::Vertical, 0);
        body.append(&changes);
        let choices = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .accessible_role(gtk::AccessibleRole::RadioGroup)
            .build();
        choices.update_property(&[gtk::accessible::Property::Label(&tr("Which size changes?"))]);
        body.append(&choices);
        let stop = sections::value("");
        stop.add_css_class("stave-grip-stop");
        let stop_row = sections::row(&tr("Stops"), &stop);
        body.append(&stop_row);
        let waiting = sections::words("", &["stave-row-note"]);
        body.append(&waiting);
        let panel = GripPanel {
            root,
            note,
            value,
            was,
            options,
            about,
            about_label,
            about_row,
            changes,
            choices,
            stop,
            stop_row,
            waiting,
            callback: Rc::new(RefCell::new(None)),
            shown_value: RefCell::new(String::new()),
        };
        let callback = panel.callback.clone();
        panel.value.connect_activate(move |entry| {
            let text = entry.text().trim().to_owned();
            if !text.is_empty() {
                report(&callback, PanelAction::Type(text));
            }
        });
        panel
    }

    /// Calls `callback` with what is clicked or typed.
    pub(super) fn connect_action(&self, callback: impl Fn(PanelAction) + 'static) {
        *self.callback.borrow_mut() = Some(Rc::new(callback));
    }

    /// Draws `model`.
    pub(super) fn show(&self, model: &PanelModel) {
        self.note.set_label(&model.title);
        // The value in the field (so it can be typed over), "was 900mm"
        // after it.
        let value = model.value.clone().unwrap_or_default();
        let was = model.was.as_ref().map_or_else(String::new, |was| {
            fill_in(&tr("was {value}"), &[("value", was)])
        });
        // The field keeps what is being typed in it.
        if *self.shown_value.borrow() != value && !self.value.has_focus() {
            self.value.set_text(&value);
        }
        self.shown_value.replace(value);
        self.was.set_label(&was);
        self.was.set_visible(!was.is_empty());
        // "overall_width" under "Follows"; anything else ("solved",
        // "stuck: …") under "Note".
        match &model.about {
            Some(about) => {
                self.about_label.set_label(&about.label);
                self.about.set_label(&about.words);
                self.about_row.set_visible(true);
            }
            None => self.about_row.set_visible(false),
        }
        clear(&self.changes);
        for line in &model.changes {
            self.changes
                .append(&sections::words(line, &["stave-grip-change"]));
        }
        self.changes.set_visible(!model.changes.is_empty());
        clear(&self.choices);
        if !model.choices.is_empty() {
            self.choices.append(&sections::words(
                &tr("Which size changes?"),
                &["stave-grip-ask"],
            ));
            let mut group: Option<gtk::CheckButton> = None;
            for choice in &model.choices {
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
                        .ellipsize(gtk::pango::EllipsizeMode::Start)
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
                let callback = self.callback.clone();
                let param = choice.param;
                radio.connect_toggled(move |radio| {
                    if radio.is_active() {
                        report(&callback, PanelAction::Pick(param));
                    }
                });
                self.choices.append(&radio);
            }
        }
        self.choices.set_visible(!model.choices.is_empty());
        self.stop
            .set_label(model.stop.as_deref().unwrap_or_default());
        self.stop_row.set_visible(model.stop.is_some());
        // The verdict is the card's; what to do next is said here.
        let waiting = (model.verdict.class == "waiting").then_some(&model.verdict.words);
        self.waiting.set_label(waiting.map_or("", String::as_str));
        self.waiting.set_visible(waiting.is_some());
    }

    /// What the section says, top to bottom, for a test.
    #[cfg(test)]
    pub(super) fn words(&self) -> Vec<String> {
        vec![
            self.note.label().to_string(),
            self.value.text().to_string(),
            self.was.label().to_string(),
            self.about.label().to_string(),
            self.stop.label().to_string(),
        ]
    }

    /// The radio rows' names and whether each is picked, for a test.
    #[cfg(test)]
    pub(super) fn radios(&self) -> Vec<(String, bool)> {
        let mut radios = Vec::new();
        let mut child = self.choices.first_child();
        while let Some(widget) = child {
            if let Some(radio) = widget.downcast_ref::<gtk::CheckButton>() {
                let name = radio
                    .child()
                    .and_then(|row| row.first_child())
                    .and_downcast::<gtk::Label>()
                    .map(|label| label.label().to_string())
                    .unwrap_or_default();
                radios.push((name, radio.is_active()));
            }
            child = widget.next_sibling();
        }
        radios
    }
}

/// The one-line readout at the top of the 3D view while a grip is moved:
/// `overall_width 940 → 1040 mm`, the name in light blue.
pub(super) struct Readout {
    /// The overlay.
    pub(super) root: gtk::Box,
    name: gtk::Label,
    value: gtk::Label,
}

impl Readout {
    /// The readout, hidden.
    pub(super) fn new() -> Readout {
        let name = gtk::Label::builder()
            .css_classes(["stave-readout-name"])
            .build();
        let value = gtk::Label::builder()
            .css_classes(["stave-readout-value"])
            .build();
        let root = gtk::Box::builder()
            .spacing(8)
            .halign(gtk::Align::Center)
            .valign(gtk::Align::Start)
            .margin_top(10)
            .can_target(false)
            .visible(false)
            .css_classes(["stave-grip-readout"])
            .build();
        root.append(&name);
        root.append(&value);
        Readout { root, name, value }
    }

    /// Shows `name` and `value`, or hides it for `None`.
    pub(super) fn show(&self, line: Option<&(String, String)>) {
        match line {
            Some((name, value)) => {
                self.name.set_label(name);
                self.value.set_label(value);
                self.root.set_visible(true);
            }
            None => self.root.set_visible(false),
        }
    }

    /// What it says, for a test.
    #[cfg(test)]
    pub(super) fn words(&self) -> Option<String> {
        self.root
            .is_visible()
            .then(|| format!("{} {}", self.name.label(), self.value.label()))
    }
}

fn report(callback: &Rc<RefCell<Option<ActionCallback>>>, action: PanelAction) {
    let callback = callback.borrow().clone();
    if let Some(callback) = callback {
        callback(action);
    }
}

fn clear(container: &gtk::Box) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
}

/// The grips' own few colours (libadwaita's named colours), installed
/// once per display. The handles are small round dots (white, a blue
/// ring), the same for every grip: what a size follows is said in words.
const STYLE: &str = r#"
.stave-grip-title { font-family: monospace; }
entry.stave-grip-value { font-family: monospace; }
.stave-grip-change { font-size: 12px; font-family: monospace; color: #99c1f1; }
.stave-grip-ask { font-size: 12px; font-weight: 700; margin-top: 4px; }
.stave-grip-stop { color: @error_color; }
.stave-grip-handle { min-width: 8px; min-height: 8px; border-radius: 999px; background: white;
  border: 2px solid @accent_color; box-shadow: 0 1px 2px alpha(black, 0.35); }
.stave-grip-handle:focus-visible { outline: 2px solid alpha(@accent_color, 0.5); outline-offset: 2px; }
.stave-grip-handle.dragging { background: @accent_color; }
.stave-grip-lock { min-width: 18px; min-height: 18px; border-radius: 999px; background: @window_bg_color;
  color: @window_fg_color; box-shadow: 0 1px 3px alpha(black, 0.35); -gtk-icon-size: 12px; }
.stave-grip-lines { color: @accent_color; }
.stave-grip-label { padding: 0; border-radius: 6px; background: alpha(@window_bg_color, 0.92);
  color: @window_fg_color; box-shadow: 0 1px 3px alpha(black, 0.25); }
.stave-grip-label button.stave-grip-number { color: @window_fg_color; padding: 0 6px; min-height: 20px;
  font-weight: 600; font-size: 11.5px; font-family: monospace; }
.stave-grip-label.locked { padding: 2px 6px; font-size: 11.5px; opacity: 0.85; }
.stave-grip-label entry { min-height: 22px; font-size: 12px; }
.stave-grip-label .stave-grip-error { color: @error_color; font-size: 12px; font-weight: 600;
  padding: 2px 6px 4px 6px; }
.stave-grip-label menubutton > button { min-height: 20px; min-width: 20px; padding: 0 2px; }
.stave-grip-switches { padding: 8px; }
.stave-grip-switches .stave-grip-note { font-size: 12px; opacity: 0.75; }
"#;

/// Puts the grips' stylesheet on the display, once.
pub(super) fn install_style(display: &gdk::Display) {
    thread_local! {
        static INSTALLED: RefCell<Vec<gdk::Display>> = const { RefCell::new(Vec::new()) };
    }
    INSTALLED.with(|installed| {
        let mut installed = installed.borrow_mut();
        if installed.contains(display) {
            return;
        }
        let provider = gtk::CssProvider::new();
        provider.load_from_string(STYLE);
        gtk::style_context_add_provider_for_display(
            display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 1,
        );
        installed.push(display.clone());
    });
}
