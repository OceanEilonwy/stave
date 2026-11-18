//! The card for a chosen candidate (`docs/editing.md`, "Verdicts and
//! strict commit"): the name it would be written under, what is still to
//! point at, its settings and where each value comes from, and the
//! verdict with the checker's problems and fixes. It draws the contract's
//! values (`stave_app::edit`) and reports what the user did as a
//! [`CardAction`]; it decides nothing about the design.
//!
//! ```text
//! ┌ IN THIS SPACE · casework ────────────────┐
//! │ Drawer                                   │
//! │ Name  [drawer_2          ]               │
//! │ POINT AT                                 │
//! │  hinge_side  Click the part on the model │
//! │ SETTINGS                                 │
//! │  front_stock [oak_19]  copied from …     │
//! │  pull_kind   [      ]  needs a choice    │
//! │ ✕ It doesn't fit yet                     │
//! │  error[overlap]: drawer_2 and door_l …   │
//! │  help: …                                 │
//! │  [Remove door_l and door_r]              │
//! │                        [Cancel]  [Add]   │
//! └──────────────────────────────────────────┘
//! ```
//!
//! Keys: Enter adds when Add is enabled; Escape cancels.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, glib};
use stave_app::edit::{
    Candidate, Heading, MenuGroup, PlaceParam, PlaceSlot, Problem, SettingParam, SettingValue,
    Verdict,
};
use stave_lang::Diagnostic;
use stave_lang::ast::{Expr, Ident};
use stave_model::ids::AssemblyId;

type ActionCallback = Rc<dyn Fn(CardAction)>;

/// What the user did on the card.
#[derive(Debug, Clone, PartialEq)]
pub enum CardAction {
    /// Retyped the generated name (a `rename`).
    Rename(String),
    /// Typed a value for a setting and pressed Enter: the text as typed,
    /// for the app to read as an expression.
    Choose {
        /// The setting param.
        param: Ident,
        /// What was typed.
        text: String,
    },
    /// Pressed a fix's button: the problem's index in the refusal, in the
    /// checker's order, and the fix's index among its fixes.
    Fix {
        /// Which problem.
        problem: usize,
        /// Which of its fixes.
        fix: usize,
    },
    /// Pressed Add (or Enter while Add is enabled).
    Add,
    /// Pressed Cancel (or Escape).
    Cancel,
}

/// The words for values the contract holds as ids and expressions, which
/// only the caller (the app, with the project's text) can say.
pub trait Words {
    /// An expression as written: `oak_19`, `overlay(left: 18mm, ...)`.
    fn expression(&self, expression: &Expr) -> String;
    /// An assembly's name: `drawer[top]`.
    fn assembly(&self, assembly: AssemblyId) -> String;
    /// What a bound place param holds, as written: `door_bay[left]`,
    /// `side_l`; `None` while unbound.
    fn place(&self, slot: &PlaceSlot) -> Option<String>;
}

/// What the card shows.
#[derive(Debug, Clone, Copy)]
pub struct CardModel<'a> {
    /// The kind of selection it was chosen for.
    pub heading: Heading,
    /// The candidate.
    pub candidate: &'a Candidate,
    /// The name the pick writes it under: `drawer_2`.
    pub name: &'a str,
    /// The verdict; `None` while the candidate isn't complete, so isn't
    /// tried.
    pub verdict: Option<&'a Verdict>,
}

/// `true` when Add may be pressed: the verdict is Fits and nothing is
/// still to point at or to choose.
pub fn can_add(candidate: &Candidate, verdict: Option<&Verdict>) -> bool {
    candidate.is_complete() && matches!(verdict, Some(Verdict::Fits(_)))
}

/// Why Add is disabled, said to the user; `None` when it is enabled.
pub fn add_blocked_reason(
    candidate: &Candidate,
    verdict: Option<&Verdict>,
) -> Option<&'static str> {
    if candidate.unbound_place().next().is_some() {
        Some("Point at what is still missing on the model first")
    } else if candidate.unset_settings().next().is_some() {
        Some("Choose the settings marked as needing a choice first")
    } else {
        match verdict {
            Some(Verdict::Fits(_)) => None,
            Some(Verdict::Checking) | None => Some("Wait until the check has finished"),
            Some(Verdict::Refused(_)) => Some("It doesn't fit yet: use a fix or change a setting"),
        }
    }
}

/// The prompt for a place param still to point at: "Click a space on the
/// model for opening".
pub fn place_prompt(param: &PlaceParam) -> String {
    let what = match param.slot {
        PlaceSlot::Region(_) => "a space",
        PlaceSlot::Anchor(_) => "a face of a part",
        PlaceSlot::Part(_) => "a part",
        PlaceSlot::Parts(_) => "the parts",
    };
    format!(
        "Click {what} on the model for {}",
        spaced(param.name.as_str())
    )
}

/// A name with its underscores as spaces, for a sentence: `hinge side`.
fn spaced(name: &str) -> String {
    name.replace('_', " ")
}

/// Where a setting's value comes from, in words.
pub fn source_words(value: &SettingValue, words: &dyn Words) -> String {
    match value {
        SettingValue::Default => "the default".to_owned(),
        SettingValue::Copied { from, .. } => format!("copied from {}", words.assembly(*from)),
        SettingValue::Chosen(_) => "your choice".to_owned(),
        SettingValue::Unset => "needs a choice".to_owned(),
    }
}

/// The value written for a setting, if there is one to show.
fn value_text(value: &SettingValue, words: &dyn Words) -> Option<String> {
    match value {
        SettingValue::Copied { value, .. } | SettingValue::Chosen(value) => {
            Some(words.expression(value))
        }
        SettingValue::Default | SettingValue::Unset => None,
    }
}

/// A diagnostic's first line as `stave check` words it:
/// `error[overlap]: drawer_2 and door_l overlap`.
pub fn diagnostic_head(diagnostic: &Diagnostic) -> String {
    format!(
        "{}[{}]: {}",
        diagnostic.severity.word(),
        diagnostic.code.as_str(),
        diagnostic.message
    )
}

/// The card's eyebrow: "IN THIS SPACE · CASEWORK".
fn eyebrow(heading: Heading, group: &MenuGroup) -> String {
    let from = super::edit_menu::group_title(group);
    format!("{} · {}", heading.text(), from).to_uppercase()
}

/// The card for a chosen candidate.
pub struct EditCard {
    /// The panel, ready to float over the 3D view.
    pub root: gtk::Box,
    body: gtk::Box,
    add: gtk::Button,
    name: gtk::EditableLabel,
    /// The name as last shown, so only a retyped one is reported.
    shown_name: Rc<RefCell<String>>,
    add_enabled: Rc<Cell<bool>>,
    /// Each setting's entry, by param (for tests).
    entries: RefCell<Vec<(Ident, gtk::Entry)>>,
    /// Each fix's button, with its problem and fix index (for tests).
    fixes: RefCell<Vec<((usize, usize), gtk::Button)>>,
    on_action: Rc<RefCell<Option<ActionCallback>>>,
}

impl EditCard {
    /// An empty card.
    pub fn new() -> EditCard {
        if let Some(display) = gdk::Display::default() {
            install_style(&display);
        }
        let on_action: Rc<RefCell<Option<ActionCallback>>> = Rc::new(RefCell::new(None));
        let body = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(16)
            .css_classes(["stave-inspector"])
            .build();
        let scrolled = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .propagate_natural_height(true)
            .max_content_height(640)
            .vexpand(true)
            .child(&body)
            .build();
        let name = gtk::EditableLabel::builder()
            .hexpand(true)
            .css_classes(["stave-edit-name"])
            .tooltip_text("The name it is written under. Click to retype it.")
            .build();
        name.update_property(&[gtk::accessible::Property::Label("Name")]);
        let cancel = gtk::Button::builder()
            .label("Cancel")
            .tooltip_text("Leave the design as it is (Escape)")
            .build();
        let add = gtk::Button::builder()
            .label("Add")
            .css_classes(["suggested-action"])
            .build();
        let buttons = gtk::Box::builder()
            .spacing(8)
            .halign(gtk::Align::End)
            .css_classes(["stave-edit-buttons"])
            .build();
        buttons.append(&cancel);
        buttons.append(&add);
        let root = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .css_classes(["stave-card-panel", "stave-edit-card"])
            .width_request(340)
            .accessible_role(gtk::AccessibleRole::Group)
            .overflow(gtk::Overflow::Hidden)
            .build();
        root.append(&scrolled);
        root.append(&buttons);

        let add_enabled = Rc::new(Cell::new(false));
        let shown_name = Rc::new(RefCell::new(String::new()));
        let send = {
            let on_action = on_action.clone();
            move |action: CardAction| {
                let callback = on_action.borrow().clone();
                if let Some(callback) = callback {
                    callback(action);
                }
            }
        };
        {
            let send = send.clone();
            add.connect_clicked(move |_| send(CardAction::Add));
        }
        {
            let send = send.clone();
            cancel.connect_clicked(move |_| send(CardAction::Cancel));
        }
        {
            let send = send.clone();
            let shown = shown_name.clone();
            name.connect_editing_notify(move |label| {
                if label.is_editing() {
                    return;
                }
                let text = label.text().trim().to_owned();
                if text.is_empty() {
                    label.set_text(&shown.borrow());
                    return;
                }
                if text != *shown.borrow() {
                    shown.replace(text.clone());
                    send(CardAction::Rename(text));
                }
            });
        }
        let keys = gtk::EventControllerKey::new();
        {
            let send = send.clone();
            let enabled = add_enabled.clone();
            keys.connect_key_pressed(move |_, key, _, _| key_action(key, enabled.get(), &send));
        }
        root.add_controller(keys);

        EditCard {
            root,
            body,
            add,
            name,
            shown_name,
            add_enabled,
            entries: RefCell::new(Vec::new()),
            fixes: RefCell::new(Vec::new()),
            on_action,
        }
    }

    /// Calls `callback` with each thing the user does on the card.
    pub fn connect_action(&self, callback: impl Fn(CardAction) + 'static) {
        self.on_action.replace(Some(Rc::new(callback)));
    }

    /// Shows a candidate, its name and its verdict.
    pub fn show(&self, model: &CardModel<'_>, words: &dyn Words) {
        while let Some(child) = self.body.first_child() {
            self.body.remove(&child);
        }
        let candidate = model.candidate;
        let label = candidate.entry.label.as_str();
        self.root
            .update_property(&[gtk::accessible::Property::Label(&format!("Add {label}"))]);

        let head = gtk::Box::new(gtk::Orientation::Vertical, 2);
        head.append(&text(
            &eyebrow(model.heading, &candidate.entry.group),
            &["stave-eyebrow"],
        ));
        head.append(&text(label, &["stave-inspector-title"]));
        if let Some(hint) = &candidate.entry.hint {
            head.append(&text(hint, &["stave-description"]));
        }
        self.body.append(&head);

        // The generated name, retyped in place.
        let name_row = gtk::Box::builder()
            .spacing(10)
            .css_classes(["stave-card-row"])
            .build();
        name_row.append(&key_label("Name"));
        self.shown_name.replace(model.name.to_owned());
        self.name.set_text(model.name);
        if let Some(parent) = self.name.parent()
            && let Ok(parent) = parent.downcast::<gtk::Box>()
        {
            parent.remove(&self.name);
        }
        name_row.append(&self.name);
        let name_card = card();
        name_card.append(&name_row);
        self.body.append(&name_card);

        self.body.append(&self.place_section(candidate, words));
        let mut fixes = Vec::new();
        self.body
            .append(&self.verdict_section(model.verdict, candidate, &mut fixes));
        self.fixes.replace(fixes);
        let mut entries = Vec::new();
        if !candidate.settings.is_empty() {
            self.body
                .append(&self.settings_section(&candidate.settings, words, &mut entries));
        }
        self.entries.replace(entries);

        let enabled = can_add(candidate, model.verdict);
        self.add_enabled.set(enabled);
        self.add.set_sensitive(enabled);
        self.add.set_tooltip_text(Some(
            add_blocked_reason(candidate, model.verdict)
                .unwrap_or("Write it into the design (Enter)"),
        ));
    }

    /// The place params: each bound one with what holds it, each unbound
    /// one with a prompt to point at it.
    fn place_section(&self, candidate: &Candidate, words: &dyn Words) -> gtk::Box {
        let section = gtk::Box::new(gtk::Orientation::Vertical, 8);
        let waiting = candidate.unbound_place().next().is_some();
        section.append(&text(
            if waiting { "POINT AT" } else { "PLACED IN" },
            &["stave-section"],
        ));
        let rows = card();
        for param in &candidate.place {
            let row = gtk::Box::builder()
                .spacing(10)
                .css_classes(["stave-card-row"])
                .build();
            row.append(&key_label(param.name.as_str()));
            match words.place(&param.slot) {
                Some(value) => {
                    row.append(&value_label(&value));
                }
                None => {
                    let prompt = gtk::Box::builder()
                        .spacing(6)
                        .hexpand(true)
                        .halign(gtk::Align::End)
                        .css_classes(["stave-edit-prompt"])
                        .build();
                    prompt.append(&gtk::Image::from_icon_name("find-location-symbolic"));
                    let words = place_prompt(param);
                    let label = text(&words, &[]);
                    label.set_xalign(1.0);
                    label.set_justify(gtk::Justification::Right);
                    prompt.append(&label);
                    row.append(&prompt);
                }
            }
            rows.append(&row);
        }
        section.append(&rows);
        section
    }

    /// The settings: each with its value, typed in place, and where the
    /// value comes from. Those that need a choice come first; those left
    /// at their defaults fold away under "More settings".
    fn settings_section(
        &self,
        settings: &[SettingParam],
        words: &dyn Words,
        entries: &mut Vec<(Ident, gtk::Entry)>,
    ) -> gtk::Box {
        let section = gtk::Box::new(gtk::Orientation::Vertical, 8);
        section.append(&text("SETTINGS", &["stave-section"]));
        let unset = settings
            .iter()
            .filter(|setting| matches!(setting.value, SettingValue::Unset));
        let given = settings.iter().filter(|setting| {
            matches!(
                setting.value,
                SettingValue::Copied { .. } | SettingValue::Chosen(_)
            )
        });
        let defaults: Vec<&SettingParam> = settings
            .iter()
            .filter(|setting| matches!(setting.value, SettingValue::Default))
            .collect();
        let rows = card();
        for setting in unset.chain(given) {
            let (row, entry) = self.setting_row(setting, words);
            rows.append(&row);
            entries.push((setting.name.clone(), entry));
        }
        if rows.first_child().is_some() {
            section.append(&rows);
        }
        if !defaults.is_empty() {
            let more = card();
            for setting in &defaults {
                let (row, entry) = self.setting_row(setting, words);
                more.append(&row);
                entries.push((setting.name.clone(), entry));
            }
            let count = defaults.len();
            let expander = gtk::Expander::builder()
                .label(if count == 1 {
                    "1 more setting, at its default".to_owned()
                } else {
                    format!("{count} more settings, at their defaults")
                })
                .child(&more)
                .css_classes(["stave-edit-more"])
                .build();
            section.append(&expander);
        }
        section
    }

    /// One setting: its name, its value in an entry, and under it its
    /// type and where the value comes from.
    fn setting_row(&self, setting: &SettingParam, words: &dyn Words) -> (gtk::Box, gtk::Entry) {
        let row = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(2)
            .css_classes(["stave-card-row", "stave-edit-setting"])
            .build();
        let line = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        let name = setting.name.as_str();
        let key = key_label(name);
        key.set_width_chars(12);
        line.append(&key);
        let value = value_text(&setting.value, words);
        let unset = matches!(setting.value, SettingValue::Unset);
        let entry = gtk::Entry::builder()
            .text(value.as_deref().unwrap_or(""))
            .placeholder_text(match setting.value {
                SettingValue::Unset => "Needs a choice".to_owned(),
                SettingValue::Default | SettingValue::Copied { .. } | SettingValue::Chosen(_) => {
                    "Use the default".to_owned()
                }
            })
            .hexpand(true)
            .width_chars(8)
            .tooltip_text(value.as_deref().unwrap_or("Type a value and press Enter"))
            .build();
        entry.update_property(&[gtk::accessible::Property::Label(name)]);
        if unset {
            entry.add_css_class("warning");
        }
        {
            let on_action = self.on_action.clone();
            let param = setting.name.clone();
            entry.connect_activate(move |entry| {
                let typed = entry.text().trim().to_owned();
                if typed.is_empty() {
                    return;
                }
                let callback = on_action.borrow().clone();
                if let Some(callback) = callback {
                    callback(CardAction::Choose {
                        param: param.clone(),
                        text: typed,
                    });
                }
            });
        }
        line.append(&entry);
        row.append(&line);
        let source = source_words(&setting.value, words);
        let caption = text(
            &if unset {
                format!(
                    "{}: needs a choice before it can be tried",
                    setting.ty.word()
                )
            } else {
                format!("{} · {source}", setting.ty.word())
            },
            &["stave-edit-source"],
        );
        if unset {
            caption.add_css_class("stave-edit-needs-choice");
        }
        entry.update_property(&[gtk::accessible::Property::Description(&source)]);
        row.append(&caption);
        (row, entry)
    }

    /// The verdict: checking, fits (with its warnings), or refused with
    /// each problem's message, help and fixes.
    fn verdict_section(
        &self,
        verdict: Option<&Verdict>,
        candidate: &Candidate,
        fixes: &mut Vec<((usize, usize), gtk::Button)>,
    ) -> gtk::Box {
        let section = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(8)
            .css_classes(["stave-edit-verdict"])
            .build();
        let status = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        section.append(&status);
        match verdict {
            None => {
                status.append(&gtk::Image::from_icon_name("dialog-information-symbolic"));
                let missing = if candidate.unbound_place().next().is_some() {
                    "Point at what is missing on the model; then it is checked."
                } else {
                    "Choose the settings that need a choice; then it is checked."
                };
                status.append(&text(missing, &["stave-description"]));
            }
            Some(Verdict::Checking) => {
                status.append(&gtk::Spinner::builder().spinning(true).build());
                status.append(&text("Checking whether it fits…", &["stave-edit-status"]));
            }
            Some(Verdict::Fits(fits)) => {
                section.add_css_class("fits");
                status.append(&gtk::Image::from_icon_name("object-select-symbolic"));
                status.append(&text("It fits", &["stave-edit-status"]));
                for warning in fits.warnings() {
                    section.append(&diagnostic_block(warning));
                }
            }
            Some(Verdict::Refused(refusal)) => {
                section.add_css_class("refused");
                status.append(&gtk::Image::from_icon_name("dialog-error-symbolic"));
                let count = refusal.problems().iter().count();
                status.append(&text(
                    &if count == 1 {
                        "It doesn't fit: 1 problem".to_owned()
                    } else {
                        format!("It doesn't fit: {count} problems")
                    },
                    &["stave-edit-status"],
                ));
                for (problem_index, problem) in refusal.problems().iter().enumerate() {
                    section.append(&self.problem_block(problem_index, problem, fixes));
                }
            }
        }
        section
    }

    /// One problem: its message and help as the checker words them, and a
    /// button per fix.
    fn problem_block(
        &self,
        problem_index: usize,
        problem: &Problem,
        fixes: &mut Vec<((usize, usize), gtk::Button)>,
    ) -> gtk::Box {
        let block = diagnostic_block(problem.diagnostic());
        if problem.fixes().is_empty() {
            block.append(&text(
                "No fix can be applied for you: change a setting or point somewhere else.",
                &["stave-footnote"],
            ));
            return block;
        }
        let buttons = gtk::FlowBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .column_spacing(6)
            .row_spacing(6)
            .max_children_per_line(3)
            .build();
        for (fix_index, fix) in problem.fixes().iter().enumerate() {
            let label = fix.label();
            let button = gtk::Button::builder()
                .label(&label)
                .css_classes(["pill", "stave-edit-fix"])
                .tooltip_text("Add this change to what is tried")
                .build();
            let on_action = self.on_action.clone();
            button.connect_clicked(move |_| {
                let callback = on_action.borrow().clone();
                if let Some(callback) = callback {
                    callback(CardAction::Fix {
                        problem: problem_index,
                        fix: fix_index,
                    });
                }
            });
            buttons.insert(&button, -1);
            fixes.push(((problem_index, fix_index), button));
        }
        block.append(&buttons);
        block
    }

    /// Handles a key as the card does: Enter adds when enabled, Escape
    /// cancels.
    #[cfg(test)]
    pub fn press(&self, key: gdk::Key) -> glib::Propagation {
        let on_action = self.on_action.clone();
        key_action(key, self.add_enabled.get(), &move |action| {
            let callback = on_action.borrow().clone();
            if let Some(callback) = callback {
                callback(action);
            }
        })
    }

    /// The Add and Cancel buttons (for tests).
    #[cfg(test)]
    pub fn buttons(&self) -> (gtk::Button, gtk::Button) {
        let cancel = self
            .add
            .prev_sibling()
            .and_then(|widget| widget.downcast::<gtk::Button>().ok())
            .expect("Cancel sits before Add");
        (self.add.clone(), cancel)
    }

    /// The name field (for tests).
    #[cfg(test)]
    pub fn name_field(&self) -> gtk::EditableLabel {
        self.name.clone()
    }

    /// Each setting's entry (for tests).
    #[cfg(test)]
    pub fn entries(&self) -> Vec<(Ident, gtk::Entry)> {
        self.entries.borrow().clone()
    }

    /// Each fix's button with its problem and fix index (for tests).
    #[cfg(test)]
    pub fn fix_buttons(&self) -> Vec<((usize, usize), gtk::Button)> {
        self.fixes.borrow().clone()
    }
}

/// What a key does on the card.
fn key_action(key: gdk::Key, add_enabled: bool, send: &dyn Fn(CardAction)) -> glib::Propagation {
    if key == gdk::Key::Escape {
        send(CardAction::Cancel);
        glib::Propagation::Stop
    } else if matches!(
        key,
        gdk::Key::Return | gdk::Key::KP_Enter | gdk::Key::ISO_Enter
    ) {
        if add_enabled {
            send(CardAction::Add);
        }
        glib::Propagation::Stop
    } else {
        glib::Propagation::Proceed
    }
}

/// A diagnostic as the checker words it: its head line, its notes and
/// its help line.
fn diagnostic_block(diagnostic: &Diagnostic) -> gtk::Box {
    let block = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(4)
        .css_classes(["stave-edit-problem"])
        .build();
    let head = text(&diagnostic_head(diagnostic), &["stave-edit-problem-head"]);
    head.add_css_class(if diagnostic.is_error() {
        "error"
    } else {
        "warning"
    });
    head.set_selectable(true);
    block.append(&head);
    for note in &diagnostic.notes {
        block.append(&text(&format!("note: {note}"), &["stave-edit-note"]));
    }
    if let Some(help) = &diagnostic.help {
        block.append(&text(&format!("help: {help}"), &["stave-edit-help"]));
    }
    block
}

fn text(words: &str, classes: &[&str]) -> gtk::Label {
    gtk::Label::builder()
        .label(words)
        .xalign(0.0)
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .css_classes(classes.to_vec())
        .build()
}

fn key_label(words: &str) -> gtk::Label {
    gtk::Label::builder()
        .label(words)
        .xalign(0.0)
        .valign(gtk::Align::Center)
        .css_classes(["stave-key"])
        .build()
}

fn value_label(words: &str) -> gtk::Label {
    gtk::Label::builder()
        .label(words)
        .xalign(1.0)
        .hexpand(true)
        .wrap(true)
        .justify(gtk::Justification::Right)
        .css_classes(["stave-value"])
        .build()
}

fn card() -> gtk::Box {
    gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .css_classes(["stave-card"])
        .overflow(gtk::Overflow::Hidden)
        .build()
}

/// The menu's and the card's own look, in libadwaita's named colours so
/// light and dark both follow the style manager. Installed once per
/// display, on top of the window's stylesheet.
const STYLE: &str = r#"
.stave-edit-entry-label { font-weight: 600; }
.stave-edit-hint { font-size: 12px; opacity: 0.75; }
.stave-edit-checking { font-size: 12px; color: @accent_color; }
.stave-edit-reason { font-size: 12px; color: @error_color; }
button.stave-edit-unavailable .stave-edit-entry-label,
button.stave-edit-unavailable .stave-edit-hint { opacity: 0.5; }
.stave-edit-name { font-weight: 600; }
.stave-edit-prompt { color: @accent_color; font-weight: 600; }
.stave-edit-source { font-size: 12px; opacity: 0.75; margin-left: 2px; }
.stave-card > .stave-card-row.stave-edit-setting { padding-top: 6px; padding-bottom: 6px; }
.stave-edit-more > title { font-size: 13px; font-weight: 600; }
.stave-edit-more > .stave-card { margin-top: 8px; }
.stave-edit-source.stave-edit-needs-choice { opacity: 1; color: @warning_color; font-weight: 600; }
.stave-edit-verdict { padding: 10px 12px; border-radius: 10px; background: alpha(currentColor, 0.05); }
.stave-edit-verdict.fits { background: alpha(@success_color, 0.12); }
.stave-edit-verdict.fits .stave-edit-status, .stave-edit-verdict.fits image { color: @success_color; }
.stave-edit-verdict.refused { background: alpha(@error_color, 0.10); }
.stave-edit-verdict.refused > box > image, .stave-edit-verdict.refused .stave-edit-status { color: @error_color; }
.stave-edit-status { font-weight: 700; }
.stave-edit-problem { margin-top: 4px; }
.stave-edit-problem-head { font-family: monospace; font-size: 12px; }
.stave-edit-problem-head.error { color: @error_color; }
.stave-edit-problem-head.warning { color: @warning_color; }
.stave-edit-help, .stave-edit-note { font-family: monospace; font-size: 12px; }
.stave-edit-buttons { padding: 10px 16px 14px 16px; }
"#;

/// Puts the menu's and the card's stylesheet on the display, once.
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

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use super::*;
    use crate::ui::edit_demo::FixtureWords;
    use crate::ui::edit_menu::tests::greyed;
    use crate::ui::edit_menu::{Checked, EditMenu, MenuItem};
    use stave_app::edit::{Declaration, Fits, clothing_cabinet_fixture};

    #[test]
    fn add_waits_for_fits_and_nothing_unset() {
        let fixture = clothing_cabinet_fixture().unwrap();
        let [drawer, door] = fixture.candidates.as_slice() else {
            panic!("two candidates");
        };
        let fits = Verdict::Fits(Fits::new(Vec::new()).unwrap());
        assert!(can_add(drawer, Some(&fits)));
        assert!(!can_add(drawer, Some(&Verdict::Checking)));
        assert!(!can_add(drawer, Some(&fixture.verdict)));
        assert!(!can_add(drawer, None));
        // A place param still to point at blocks Add whatever the verdict.
        assert!(!can_add(door, Some(&fits)));
        assert_eq!(
            add_blocked_reason(door, Some(&fits)),
            Some("Point at what is still missing on the model first")
        );
        let mut unset = drawer.clone();
        unset.settings[0].value = SettingValue::Unset;
        assert!(!can_add(&unset, Some(&fits)));
        assert_eq!(
            add_blocked_reason(drawer, Some(&fixture.verdict)),
            Some("It doesn't fit yet: use a fix or change a setting")
        );
        assert_eq!(add_blocked_reason(drawer, Some(&fits)), None);
    }

    #[test]
    fn the_card_words_things_plainly() {
        let fixture = clothing_cabinet_fixture().unwrap();
        let door = &fixture.candidates[1];
        let hinge_side = door.unbound_place().next().unwrap();
        assert_eq!(
            place_prompt(hinge_side),
            "Click a part on the model for hinge side"
        );
        let Verdict::Refused(refusal) = &fixture.verdict else {
            panic!("refused");
        };
        assert_eq!(
            diagnostic_head(refusal.problems().first().diagnostic()),
            "error[overlap]: drawer_2 and door_l overlap"
        );
        assert_eq!(
            source_words(&door.settings[0].value, &FixtureWords),
            "copied from door_l"
        );
        assert_eq!(
            source_words(&SettingValue::Unset, &FixtureWords),
            "needs a choice"
        );
    }

    /// One test builds every widget, since GTK lives on one thread. It
    /// needs a display (a desktop session, or Broadway as
    /// `tools/window-shot` starts); without one it says so and stops.
    #[test]
    fn the_menu_and_the_card_report_what_the_user_does() {
        if gtk::init().is_err() {
            eprintln!("skipped: GTK can't start here (no display)");
            return;
        }
        let fixture = clothing_cabinet_fixture().unwrap();
        let [drawer, door] = fixture.candidates.as_slice() else {
            panic!("two candidates");
        };

        // The menu: Drawer can be chosen; a door still being checked, or
        // greyed, can't.
        let menu = EditMenu::new();
        let chosen: Rc<RefCell<Vec<Declaration>>> = Rc::default();
        {
            let chosen = chosen.clone();
            menu.connect_chosen(move |declaration| chosen.borrow_mut().push(declaration));
        }
        let shut = greyed(door, "door_bay[left] already holds door_l");
        for door_item in [
            MenuItem {
                candidate: door,
                checked: Checked::Pending,
            },
            MenuItem {
                candidate: &shut,
                checked: Checked::Done,
            },
        ] {
            let items = [
                door_item,
                MenuItem {
                    candidate: drawer,
                    checked: Checked::Done,
                },
            ];
            menu.show(fixture.picked.heading(), &items);
            let rows = menu.rows();
            let order: Vec<Declaration> =
                rows.iter().map(|(declaration, _)| *declaration).collect();
            assert_eq!(order, [drawer.declaration, door.declaration]);
            for (_, button) in &rows {
                button.emit_clicked();
            }
        }
        assert_eq!(*chosen.borrow(), [drawer.declaration, drawer.declaration]);

        // The card, refused: Add is disabled, Enter does nothing, the fix
        // and Escape are reported.
        let card = EditCard::new();
        let actions: Rc<RefCell<Vec<CardAction>>> = Rc::default();
        {
            let actions = actions.clone();
            card.connect_action(move |action| actions.borrow_mut().push(action));
        }
        let show = |candidate, verdict| {
            card.show(
                &CardModel {
                    heading: fixture.picked.heading(),
                    candidate,
                    name: "drawer_2",
                    verdict,
                },
                &FixtureWords,
            );
        };
        show(drawer, Some(&fixture.verdict));
        let (add, cancel) = card.buttons();
        assert!(!add.is_sensitive());
        assert_eq!(card.press(gtk::gdk::Key::Return), glib::Propagation::Stop);
        let fixes = card.fix_buttons();
        assert_eq!(fixes.len(), 1);
        assert_eq!(
            fixes[0].1.label().as_deref(),
            Some("Remove door_l and door_r")
        );
        fixes[0].1.emit_clicked();
        card.press(gtk::gdk::Key::Escape);
        cancel.emit_clicked();
        assert_eq!(
            actions.borrow_mut().drain(..).collect::<Vec<_>>(),
            [
                CardAction::Fix { problem: 0, fix: 0 },
                CardAction::Cancel,
                CardAction::Cancel
            ]
        );

        // A setting typed and the name retyped are reported.
        let entries = card.entries();
        let (param, entry) = entries
            .iter()
            .find(|(param, _)| param.as_str() == "front_stock")
            .unwrap();
        assert_eq!(entry.text(), "oak_19");
        entry.set_text("walnut_19");
        entry.emit_activate();
        let name = card.name_field();
        assert_eq!(name.text(), "drawer_2");
        name.start_editing();
        name.set_text("tall_drawer");
        name.stop_editing(true);
        assert_eq!(
            actions.borrow_mut().drain(..).collect::<Vec<_>>(),
            [
                CardAction::Choose {
                    param: param.clone(),
                    text: "walnut_19".to_owned()
                },
                CardAction::Rename("tall_drawer".to_owned()),
            ]
        );

        // Fits: Add is enabled, and Enter adds.
        let fits = Verdict::Fits(Fits::new(Vec::new()).unwrap());
        show(drawer, Some(&fits));
        assert!(add.is_sensitive());
        card.press(gtk::gdk::Key::KP_Enter);
        add.emit_clicked();
        assert_eq!(
            actions.borrow_mut().drain(..).collect::<Vec<_>>(),
            [CardAction::Add, CardAction::Add]
        );

        // A setting with no value: shown first, and Add stays disabled.
        let mut unset = drawer.clone();
        unset.settings[1].value = SettingValue::Unset;
        show(&unset, Some(&fits));
        assert!(!add.is_sensitive());
        assert_eq!(card.entries()[0].0.as_str(), "front_stock");
        assert_eq!(card.entries()[0].1.text(), "");
        // Unbound place param: no verdict, Add disabled.
        show(door, None);
        assert!(!add.is_sensitive());
        assert!(card.fix_buttons().is_empty());
    }
}
