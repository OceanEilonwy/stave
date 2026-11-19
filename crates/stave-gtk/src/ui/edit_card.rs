//! The card for a chosen candidate (`docs/editing.md`, "Verdicts and
//! strict commit"), the content of the docked Edit panel: the name it
//! would be written under, what is still to point at, the verdict with the
//! checker's problems and fixes, and its settings, one compact row each.
//! It draws the contract's values (`stave_app::edit`, `stave_app::choices`)
//! and reports what the user did as a [`CardAction`]; it decides nothing
//! about the design.
//!
//! ```text
//! ┌ Case ─────────────────────── casework ┐
//! │ A cabinet case: a base, two sides an… │
//! │ Name     case                         │
//! │ space    envelope                     │
//! │ ┌ ✓ It fits ────────────────────────┐ │  (scrolls in its own box)
//! │ └───────────────────────────────────┘ │
//! │ SETTINGS                              │
//! │ panel_stock   [oak_19            ▾]   │
//! │ joinery       [dowel(count: 4)   ▾]   │  (the default, dimmed)
//! │   ✓ dowel(count: 4)        default    │  (its short list, opened)
//! │     screw                  used       │
//! │     Search all 18…                    │
//! │ (the body scrolls)                    │
//! ├───────────────────────────────────────┤
//! │ Repeat…                               │  (the window's repeat fold)
//! │                     [Cancel]  [Add]   │  (always in sight)
//! └───────────────────────────────────────┘
//! ```
//!
//! Keys: Enter adds when Add is enabled; Escape cancels. The keys reach the
//! card while the keyboard is in it, so the card takes the keyboard when it
//! appears and each time it is shown again (unless the keyboard is already
//! inside it, say in a setting being typed).

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, glib};
use stave_app::choices::{ChoiceRole, ShortChoice, short_list};
use stave_app::edit::{
    Blame, Candidate, Heading, MenuGroup, OptionFit, PlaceParam, PlaceSlot, Problem, SettingParam,
    SettingValue, Verdict,
};
use stave_lang::Diagnostic;
use stave_lang::ast::{Expr, Ident};
use stave_model::ids::AssemblyId;

use super::sections::{self, LinkHandlers};
use stave_app::inspect::NameLink;

type ActionCallback = Rc<dyn Fn(CardAction)>;
/// A short list's rows, each by its entry's text (the search row by "").
#[cfg(test)]
pub type ChoiceRows = Vec<(String, gtk::Button)>;

/// What the user did on the card.
#[derive(Debug, Clone, PartialEq)]
pub enum CardAction {
    /// Retyped the generated name (a `rename`).
    Rename(String),
    /// Typed a value for a setting and pressed Enter, or chose one from its
    /// short list or its search: the text, for the app to read as an
    /// expression.
    Choose {
        /// The setting param.
        param: Ident,
        /// What was typed or chosen.
        text: String,
    },
    /// Chose the default from a setting's short list: the setting goes
    /// back to its param's own default.
    UseDefault {
        /// The setting param.
        param: Ident,
    },
    /// Chose "Search…" from a setting's short list: the search over every
    /// option of the setting opens.
    Search {
        /// The setting param.
        param: Ident,
    },
    /// Pressed a fix's button: the problem's index in the refusal, in the
    /// checker's order, and the fix's index among its fixes.
    Fix {
        /// Which problem.
        problem: usize,
        /// Which of its fixes.
        fix: usize,
    },
    /// Pressed the button that puts back the setting a refusal blames.
    PutBack,
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
    /// The names in a place's value (`bay[doors]`, `[side_l, side_r]`)
    /// as links to what they name (`stave_app::inspect::place_links`);
    /// none when the design isn't known.
    fn links(&self, text: &str) -> Vec<NameLink> {
        let _ = text;
        Vec::new()
    }
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
    /// The labels of the fixes added to what is tried, in order ("Use
    /// left: 17mm"): they change the text after the settings shown.
    pub applied: &'a [String],
    /// The setting change a refusal is blamed on, put first on the card
    /// with the button that puts it back.
    pub blame: Option<&'a Blame>,
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

/// What an empty entry for a setting at its default shows, dimmed: the
/// value the default stands for once the candidate was tried (`220mm`),
/// else the default as written (`dowel(count: 4)`). The words "the
/// default" are in its tooltip, not in the entry, so the value is not cut
/// off.
pub fn default_placeholder(setting: &SettingParam) -> String {
    match (&setting.default_value, &setting.default_text) {
        (Some(value), _) => value.clone(),
        (None, Some(text)) => text.clone(),
        (None, None) => "Use the default".to_owned(),
    }
}

/// A setting's type and where its value comes from, with the default
/// spelled out and the range a `within` allows: "Length · the default,
/// opening.up - bottom_gap - top_gap = 220mm · 100mm to 400mm". The row's
/// tooltip and its accessible description; no line on the card.
pub fn setting_about(setting: &SettingParam, words: &dyn Words) -> String {
    let source = match (
        &setting.value,
        &setting.default_text,
        &setting.default_value,
    ) {
        (SettingValue::Default, Some(text), Some(value)) if text != value => {
            format!("the default, {text} = {value}")
        }
        (SettingValue::Default, Some(text), _) => format!("the default, {text}"),
        (SettingValue::Unset, _, _) => "needs a choice before it can be tried".to_owned(),
        _ => source_words(&setting.value, words),
    };
    let about = format!("{} · {source}", setting.ty.word());
    match &setting.range {
        Some(range) => format!("{about} · {} to {}", range.low, range.high),
        None => about,
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

/// Where the candidate comes from, beside its title: "casework · in this
/// space".
fn origin(heading: Heading, group: &MenuGroup) -> String {
    let from = super::edit_menu::group_title(group);
    format!("{from} · {}", heading.text().to_lowercase())
}

/// A short list's fit in words, for its tooltip.
pub fn fit_words(fit: &OptionFit) -> String {
    match fit {
        OptionFit::NotChecked => "Checking whether it fits…".to_owned(),
        OptionFit::Fits => "Fits".to_owned(),
        OptionFit::NeedsChoice(names) => {
            let names: Vec<String> = names.iter().map(|n| spaced(n.as_str())).collect();
            format!("Checked once you choose {}", names.join(", "))
        }
        OptionFit::Refused(refusal) => format!("Can't be used here: {}", refusal.line()),
    }
}

/// The icon that says a fit, never by colour alone.
pub fn fit_icon(fit: &OptionFit) -> &'static str {
    match fit {
        OptionFit::NotChecked => "content-loading-symbolic",
        OptionFit::Fits => "object-select-symbolic",
        OptionFit::NeedsChoice(_) => "dialog-information-symbolic",
        OptionFit::Refused(_) => "action-unavailable-symbolic",
    }
}

/// The card for a chosen candidate.
pub struct EditCard {
    /// The card: its body scrolls; the repeat fold and Add and Cancel stay
    /// at its foot.
    pub root: gtk::Box,
    body: gtk::Box,
    scrolled: gtk::ScrolledWindow,
    /// Where the window puts the repeat controls' count and direction,
    /// above the buttons.
    pub repeat_slot: gtk::Box,
    /// Where the window puts "Repeat…", on the buttons' line.
    pub repeat_button_slot: gtk::Box,
    /// Where the window puts the "Pending" section (the lines Apply
    /// would write), last in the body.
    pub pending_slot: gtk::Box,
    /// Where the links in "Placed in" report clicks and hovers.
    pub links: LinkHandlers,
    /// A row to bring into sight once the layout has room for it (an open
    /// short list).
    scroll_target: Rc<RefCell<Option<gtk::Widget>>>,
    add: gtk::Button,
    name: gtk::EditableLabel,
    /// The name as last shown, so only a retyped one is reported.
    shown_name: Rc<RefCell<String>>,
    add_enabled: Rc<Cell<bool>>,
    /// Each setting's entry, by param (for tests).
    entries: RefCell<Vec<(Ident, gtk::Entry)>>,
    /// Each fix's button, with its problem and fix index (for tests).
    fixes: RefCell<Vec<((usize, usize), gtk::Button)>>,
    /// Each setting's short list, by param, and the button that opens it.
    options: RefCell<Vec<ChoiceList>>,
    /// The setting whose short list is open, kept across redraws.
    open_options: Rc<RefCell<Option<String>>>,
    /// The put-back button, while a refusal is blamed on a setting (for
    /// tests).
    put_back: RefCell<Option<gtk::Button>>,
    on_action: Rc<RefCell<Option<ActionCallback>>>,
}

/// One setting's short list on the card: shown in place under its row when
/// its button is pressed (a popover would close each time a check's answer
/// redraws it, and can't be drawn into a screenshot).
#[derive(Clone)]
struct ChoiceList {
    /// The setting.
    param: Ident,
    /// The button that shows and hides it.
    toggle: gtk::ToggleButton,
    /// What shows and hides.
    revealer: gtk::Revealer,
    /// The rows.
    list: gtk::Box,
    /// Each entry's row, by its text; the search row last, by "".
    rows: Rc<RefCell<Vec<(String, gtk::Button)>>>,
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
            .spacing(0)
            .css_classes(["stave-edit-body"])
            .build();
        let scrolled = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .child(&body)
            .build();
        let name = gtk::EditableLabel::builder()
            .hexpand(true)
            .css_classes(["stave-edit-name", "stave-name-field"])
            .tooltip_text("The name it is written under. Click to retype it.")
            .build();
        name.update_property(&[gtk::accessible::Property::Label("Name")]);
        let (apply_buttons, cancel, add) = sections::footer_buttons();
        let repeat_button_slot = gtk::Box::builder()
            .hexpand(true)
            .halign(gtk::Align::Start)
            .valign(gtk::Align::Center)
            .build();
        let buttons = gtk::Box::builder()
            .spacing(8)
            .css_classes(["stave-edit-buttons"])
            .build();
        buttons.append(&repeat_button_slot);
        buttons.append(&apply_buttons);
        let repeat_slot = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .css_classes(["stave-edit-repeat-slot"])
            .build();
        let foot = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .css_classes(["stave-edit-foot"])
            .build();
        foot.append(&repeat_slot);
        foot.append(&buttons);
        let pending_slot = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .build();
        let root = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .css_classes(["stave-edit-card"])
            .hexpand(true)
            .vexpand(true)
            .accessible_role(gtk::AccessibleRole::Group)
            // It holds the keyboard itself, so Enter and Escape work
            // before anything in it is clicked.
            .focusable(true)
            .build();
        root.append(&scrolled);
        root.append(&foot);

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
        root.connect_map(take_focus);

        let scroll_target: Rc<RefCell<Option<gtk::Widget>>> = Rc::new(RefCell::new(None));
        {
            let target = scroll_target.clone();
            let body = body.clone();
            scrolled.vadjustment().connect_changed(move |adjustment| {
                scroll_to_target(adjustment, &body, &target);
            });
        }
        EditCard {
            root,
            body,
            scrolled,
            repeat_slot,
            repeat_button_slot,
            pending_slot,
            links: LinkHandlers::default(),
            scroll_target,
            add,
            name,
            shown_name,
            add_enabled,
            entries: RefCell::new(Vec::new()),
            fixes: RefCell::new(Vec::new()),
            options: RefCell::new(Vec::new()),
            open_options: Rc::new(RefCell::new(None)),
            put_back: RefCell::new(None),
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

        // The header: the name it is written under, retyped in place, in
        // a monospace field, and what it is and where it comes from on
        // the right; the hint under it on one line, in full in its
        // tooltip.
        self.shown_name.replace(model.name.to_owned());
        self.name.set_text(model.name);
        if let Some(parent) = self.name.parent()
            && let Ok(parent) = parent.downcast::<gtk::Box>()
        {
            parent.remove(&self.name);
        }
        let from = text(label, &["stave-edit-origin"]);
        from.set_wrap(false);
        from.set_xalign(1.0);
        from.set_ellipsize(gtk::pango::EllipsizeMode::End);
        from.set_max_width_chars(18);
        from.set_tooltip_text(Some(&format!(
            "{label} · {}",
            origin(model.heading, &candidate.entry.group)
        )));
        self.body
            .append(&sections::header(None, &self.name, Some(from.upcast_ref())));
        if let Some(hint) = &candidate.entry.hint {
            let hint_label = text(hint, &["stave-edit-hint-line"]);
            hint_label.set_wrap(false);
            hint_label.set_ellipsize(gtk::pango::EllipsizeMode::End);
            hint_label.set_tooltip_text(Some(hint));
            self.body.append(&hint_label);
        }

        // Where it goes: each place it fills, its names as links.
        if !candidate.place.is_empty() {
            let (section, rows) = sections::section("Placed in", "", true);
            for param in &candidate.place {
                rows.append(&place_row(param, words, &self.links));
            }
            self.body.append(&section);
        }

        if !model.applied.is_empty() {
            self.body.append(&applied_section(model.applied));
        }
        let mut fixes = Vec::new();
        self.put_back.replace(None);
        let (section, verdict_body) = sections::section("Verdict", "", true);
        verdict_body.append(&self.verdict_section(
            model.verdict,
            model.blame,
            candidate,
            &mut fixes,
        ));
        self.body.append(&section);
        self.fixes.replace(fixes);
        let mut entries = Vec::new();
        self.options.borrow_mut().clear();
        if !candidate.settings.is_empty() {
            self.body
                .append(&self.settings_section(&candidate.settings, words, &mut entries));
        }
        self.entries.replace(entries);
        self.body.append(&self.pending_slot);

        let enabled = can_add(candidate, model.verdict);
        self.add_enabled.set(enabled);
        self.add.set_sensitive(enabled);
        self.add.set_tooltip_text(Some(
            add_blocked_reason(candidate, model.verdict)
                .unwrap_or("Write it into the design (Enter)"),
        ));
        take_focus(&self.root);
        self.scroll_to_open_options();
    }

    /// The settings, one row each: those that need a choice first, then
    /// those with a value, then those left at their defaults (the
    /// default's value dimmed in the entry). The type and where each value
    /// comes from are the row's tooltip, not a line of its own.
    fn settings_section(
        &self,
        settings: &[SettingParam],
        words: &dyn Words,
        entries: &mut Vec<(Ident, gtk::Entry)>,
    ) -> gtk::Box {
        let (section, body) = sections::section("Settings", &settings.len().to_string(), true);
        let order = |setting: &&SettingParam| match setting.value {
            SettingValue::Unset => 0,
            SettingValue::Copied { .. } | SettingValue::Chosen(_) => 1,
            SettingValue::Default => 2,
        };
        let mut ordered: Vec<&SettingParam> = settings.iter().collect();
        ordered.sort_by_key(order);
        let rows = rows_card();
        for setting in ordered {
            let (row, entry, list) = self.setting_row(setting, words);
            rows.append(&row);
            if let Some(list) = list {
                rows.append(&list.revealer);
                self.options.borrow_mut().push(list);
            }
            entries.push((setting.name.clone(), entry));
        }
        body.append(&rows);
        section
    }

    /// One setting's row: its name on the left, its value on the right in
    /// an entry (typed, Enter sets it) with the button that opens its
    /// short list. Its short list, when it has one, goes under the row.
    fn setting_row(
        &self,
        setting: &SettingParam,
        words: &dyn Words,
    ) -> (gtk::Box, gtk::Entry, Option<ChoiceList>) {
        let name = setting.name.as_str();
        let about = setting_about(setting, words);
        let line = row_box();
        line.add_css_class("stave-edit-setting");
        line.set_tooltip_text(Some(&about));
        let key = key_label(name, &about);
        line.append(&key);
        let value = value_text(&setting.value, words);
        let unset = matches!(setting.value, SettingValue::Unset);
        let entry = gtk::Entry::builder()
            .text(value.as_deref().unwrap_or(""))
            .placeholder_text(match setting.value {
                SettingValue::Unset => "Choose…".to_owned(),
                SettingValue::Default => default_placeholder(setting),
                SettingValue::Copied { .. } | SettingValue::Chosen(_) => String::new(),
            })
            .hexpand(true)
            .width_chars(4)
            .css_classes(["stave-edit-entry"])
            .tooltip_text(match &value {
                Some(value) => format!("{value}\n{about}"),
                None => format!("{}\n{about}", default_placeholder(setting)),
            })
            .build();
        entry.update_property(&[
            gtk::accessible::Property::Label(name),
            gtk::accessible::Property::Description(&about),
        ]);
        if unset {
            entry.add_css_class("warning");
            line.add_css_class("stave-edit-needs-choice");
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
        let control = gtk::Box::builder()
            .css_classes(["linked", "stave-edit-control"])
            .hexpand(true)
            .build();
        control.append(&entry);
        let choices = short_list(setting);
        let list = (!choices.is_empty() || !setting.options.is_empty())
            .then(|| self.choice_list(setting, &choices));
        if let Some(list) = &list {
            control.append(&list.toggle);
        }
        line.append(&control);
        // A long value is said in full under its row, wrapped, so it is
        // never cut off (`overlay(left: 18mm, right: 18mm, …)`).
        let shown = value.clone().or_else(|| {
            (setting.value == SettingValue::Default).then(|| default_placeholder(setting))
        });
        if let Some(shown) = shown.filter(|shown| shown.chars().count() > LONG_VALUE) {
            let row = gtk::Box::builder()
                .orientation(gtk::Orientation::Vertical)
                .css_classes(["stave-card-row", "stave-edit-row", "stave-edit-long"])
                .build();
            line.remove_css_class("stave-card-row");
            line.remove_css_class("stave-edit-row");
            row.append(&line);
            let full = text(&shown, &["stave-edit-long-value"]);
            full.set_selectable(true);
            row.append(&full);
            return (row, entry, list);
        }
        (line, entry, list)
    }

    /// A setting's short list ([`stave_app::choices::short_list`]): its
    /// default, the values the project already uses, the value it holds
    /// now, then "Search…" over every option; and the button that shows it
    /// in place under the setting.
    fn choice_list(&self, setting: &SettingParam, choices: &[ShortChoice]) -> ChoiceList {
        let name = setting.name.as_str();
        let list = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .css_classes(["stave-edit-choices"])
            .accessible_role(gtk::AccessibleRole::List)
            .build();
        list.update_property(&[gtk::accessible::Property::Label(&format!(
            "What {} can be",
            spaced(name)
        ))]);
        let open = self.open_options.borrow().as_deref() == Some(name);
        // A slide (of no length, so it is instant): a revealer that
        // doesn't slide keeps the room of what it hides.
        let revealer = gtk::Revealer::builder()
            .child(&list)
            .reveal_child(open)
            .transition_type(gtk::RevealerTransitionType::SlideDown)
            .transition_duration(0)
            .build();
        let toggle = gtk::ToggleButton::builder()
            .icon_name("pan-down-symbolic")
            .active(open)
            .valign(gtk::Align::Center)
            .css_classes(["stave-edit-options-toggle"])
            .tooltip_text(format!("Choose {}", spaced(name)))
            .build();
        toggle.update_property(&[gtk::accessible::Property::Label(&format!(
            "Choose {}",
            spaced(name)
        ))]);
        toggle.update_state(&[gtk::accessible::State::Expanded(Some(open))]);
        {
            let revealer = revealer.downgrade();
            let open_options = self.open_options.clone();
            let param = name.to_owned();
            toggle.connect_toggled(move |toggle| {
                let active = toggle.is_active();
                toggle.update_state(&[gtk::accessible::State::Expanded(Some(active))]);
                if let Some(revealer) = revealer.upgrade() {
                    revealer.set_reveal_child(active);
                }
                let mut open = open_options.borrow_mut();
                if active {
                    *open = Some(param.clone());
                } else if open.as_deref() == Some(param.as_str()) {
                    *open = None;
                }
            });
        }
        let choice_list = ChoiceList {
            param: setting.name.clone(),
            toggle,
            revealer,
            list,
            rows: Rc::new(RefCell::new(Vec::new())),
        };
        self.fill_choices(&choice_list, choices, setting.options.len());
        choice_list
    }

    /// The rows of a short list, from the setting as the session has it.
    fn fill_choices(&self, list: &ChoiceList, choices: &[ShortChoice], options: usize) {
        while let Some(child) = list.list.first_child() {
            list.list.remove(&child);
        }
        let mut rows = Vec::new();
        for choice in choices {
            let row = self.choice_row(&list.param, choice);
            list.list.append(&row);
            rows.push((choice.text.clone(), row));
        }
        if options > 0 {
            let row = self.search_row(&list.param, options);
            list.list.append(&row);
            rows.push((String::new(), row));
        }
        list.rows.replace(rows);
    }

    /// One entry of a short list: a tick for the value it holds now, its
    /// text, why it is listed ("default", "used", "now") and its fit,
    /// never by colour alone. A refused one is greyed and never reports.
    fn choice_row(&self, param: &Ident, choice: &ShortChoice) -> gtk::Button {
        let content = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let mark = gtk::Image::from_icon_name(if choice.current {
            "object-select-symbolic"
        } else {
            match &choice.fit {
                Some(fit @ OptionFit::Refused(_)) => fit_icon(fit),
                Some(_) | None => "",
            }
        });
        mark.add_css_class("stave-edit-choice-mark");
        mark.set_pixel_size(14);
        content.append(&mark);
        let words = text(&choice.text, &["stave-edit-option"]);
        words.set_hexpand(true);
        content.append(&words);
        let tag = match choice.role {
            ChoiceRole::Default => "default",
            ChoiceRole::Recent => "used here",
            ChoiceRole::Current => "now",
        };
        content.append(&text(tag, &["stave-edit-choice-tag"]));
        let button = gtk::Button::builder()
            .child(&content)
            .css_classes(["flat", "stave-edit-choice"])
            .build();
        let state = choice.fit.as_ref().map(fit_words);
        let description = [Some(tag.to_owned()), state.clone(), choice.hint.clone()]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(". ");
        button.update_property(&[
            gtk::accessible::Property::Label(&choice.text),
            gtk::accessible::Property::Description(&description),
        ]);
        button.set_tooltip_text(Some(&description));
        let choosable = choice.choosable();
        if !choosable {
            // Still focusable, so the reason can be read, but marked
            // unavailable and never reported.
            button.add_css_class("stave-edit-unavailable");
            button.update_state(&[gtk::accessible::State::Disabled(true)]);
        }
        let on_action = self.on_action.clone();
        let param = param.clone();
        let chosen = choice.text.clone();
        let default = choice.role == ChoiceRole::Default;
        let open_options = self.open_options.clone();
        button.connect_clicked(move |_| {
            if !choosable {
                return;
            }
            open_options.replace(None);
            let callback = on_action.borrow().clone();
            if let Some(callback) = callback {
                callback(if default {
                    CardAction::UseDefault {
                        param: param.clone(),
                    }
                } else {
                    CardAction::Choose {
                        param: param.clone(),
                        text: chosen.clone(),
                    }
                });
            }
        });
        button
    }

    /// "Search all 18…": the dialog over every option of the setting.
    fn search_row(&self, param: &Ident, options: usize) -> gtk::Button {
        let content = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let icon = gtk::Image::from_icon_name("system-search-symbolic");
        icon.set_pixel_size(14);
        content.append(&icon);
        let words = text(
            &if options == 1 {
                "Search the 1 option…".to_owned()
            } else {
                format!("Search all {options}…")
            },
            &[],
        );
        words.set_hexpand(true);
        content.append(&words);
        let button = gtk::Button::builder()
            .child(&content)
            .css_classes(["flat", "stave-edit-choice", "stave-edit-search"])
            .tooltip_text(
                "Search every option by its name and what it is, each with whether it fits",
            )
            .build();
        let on_action = self.on_action.clone();
        let param = param.clone();
        button.connect_clicked(move |_| {
            let callback = on_action.borrow().clone();
            if let Some(callback) = callback {
                callback(CardAction::Search {
                    param: param.clone(),
                });
            }
        });
        button
    }

    /// Redraws the settings' short lists from `candidate` (the same
    /// candidate the card shows) without rebuilding the card, so an open
    /// list stays open and the keyboard stays on the entry it was on.
    pub fn update_options(&self, candidate: &Candidate) {
        let lists = self.options.borrow().clone();
        for list in lists {
            let Some(setting) = candidate
                .settings
                .iter()
                .find(|setting| setting.name.as_str() == list.param.as_str())
            else {
                continue;
            };
            let focused = focused_choice(&list);
            self.fill_choices(&list, &short_list(setting), setting.options.len());
            if let Some(text) = focused
                && let Some((_, row)) = list.rows.borrow().iter().find(|(t, _)| *t == text)
            {
                row.grab_focus();
            }
        }
    }

    /// The verdict: checking, fits (with its warnings), or refused with
    /// each problem's message, help and fixes. Its words scroll inside a
    /// box of their own, so a long refusal never pushes the settings, Add
    /// or Cancel out of reach.
    fn verdict_section(
        &self,
        verdict: Option<&Verdict>,
        blame: Option<&Blame>,
        candidate: &Candidate,
        fixes: &mut Vec<((usize, usize), gtk::Button)>,
    ) -> gtk::Widget {
        let section = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(6)
            .css_classes(["stave-edit-verdict"])
            .build();
        // What the user just did comes first, with the way back; the
        // checker's own words stay below.
        if let (Some(blame), Some(Verdict::Refused(_))) = (blame, verdict) {
            section.append(&self.blame_block(blame));
        }
        let status = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        section.append(&status);
        // The checker's words, and the fixes' buttons.
        let words = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(6)
            .build();
        let buttons = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(4)
            .css_classes(["stave-edit-fixes"])
            .build();
        match verdict {
            None => {
                status.append(&gtk::Image::from_icon_name("dialog-information-symbolic"));
                let missing = if candidate.unbound_place().next().is_some() {
                    "Point at what is missing on the model; then it is checked."
                } else {
                    "Choose the settings that need a choice; then it is checked."
                };
                let words = text(missing, &["stave-description"]);
                words.set_hexpand(true);
                status.append(&words);
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
                    words.append(&diagnostic_block(warning));
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
                    words.append(&self.problem_block(problem_index, problem, &buttons, fixes));
                }
            }
        }
        // The checker's words scroll in a box of their own, as tall as
        // they are at the card's width up to the bound (a scrolled box
        // inside the card's own scrolled body is otherwise squeezed to its
        // scrollbar's height); the blame, the verdict's line and the
        // buttons (the fixes, each with its problem in its tooltip) stay in
        // sight.
        if words.first_child().is_some() {
            let across = self.root.width().max(LEAST_WIDTH).saturating_sub(24);
            let (_, tall, _, _) = words.measure(gtk::Orientation::Vertical, across);
            section.append(
                &gtk::ScrolledWindow::builder()
                    .hscrollbar_policy(gtk::PolicyType::Never)
                    .propagate_natural_height(true)
                    .min_content_height(tall.min(VERDICT_HEIGHT))
                    .max_content_height(VERDICT_HEIGHT)
                    .css_classes(["stave-edit-verdict-box"])
                    .child(&words)
                    .build(),
            );
        }
        if buttons.first_child().is_some() {
            section.append(&buttons);
        }
        section.upcast()
    }

    /// The setting change a refusal is blamed on, in plain words, and the
    /// button that puts it back.
    fn blame_block(&self, blame: &Blame) -> gtk::Box {
        let block = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(6)
            .css_classes(["stave-edit-blame"])
            .build();
        let line = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let icon = gtk::Image::from_icon_name("edit-undo-symbolic");
        icon.set_valign(gtk::Align::Start);
        line.append(&icon);
        let words = text(&blame.text(), &["stave-edit-blame-text"]);
        words.set_hexpand(true);
        line.append(&words);
        block.append(&line);
        let label = blame.put_back_label();
        let button = gtk::Button::builder()
            .label(&label)
            .halign(gtk::Align::Start)
            .css_classes(["pill", "stave-edit-put-back"])
            .tooltip_text("Put the setting back as it was, when it still fitted")
            .build();
        if let Some(child) = button.child()
            && let Ok(label) = child.downcast::<gtk::Label>()
        {
            label.set_wrap(true);
            label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
            label.set_justify(gtk::Justification::Left);
        }
        button.update_property(&[
            gtk::accessible::Property::Label(&label),
            gtk::accessible::Property::Description(&blame.text()),
        ]);
        let on_action = self.on_action.clone();
        button.connect_clicked(move |_| {
            let callback = on_action.borrow().clone();
            if let Some(callback) = callback {
                callback(CardAction::PutBack);
            }
        });
        block.append(&button);
        self.put_back.replace(Some(button));
        block
    }

    /// One problem: its message and help as the checker words them, and a
    /// button per fix.
    fn problem_block(
        &self,
        problem_index: usize,
        problem: &Problem,
        buttons: &gtk::Box,
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
        // Each fix's button on its own line, its label wrapped (a FlowBox
        // asks for its widest child, which held the card open wider than
        // its panel), under the checker's words, out of their scrolling.
        let head = diagnostic_head(problem.diagnostic());
        for (fix_index, fix) in problem.fixes().iter().enumerate() {
            let label = fix.label();
            let button = gtk::Button::builder()
                .label(&label)
                .halign(gtk::Align::Start)
                .css_classes(["pill", "stave-edit-fix"])
                .tooltip_text(format!("Add this change to what is tried. It fixes {head}"))
                .build();
            if let Some(child) = button.child()
                && let Ok(label) = child.downcast::<gtk::Label>()
            {
                label.set_wrap(true);
                label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
            }
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
            buttons.append(&button);
            fixes.push(((problem_index, fix_index), button));
        }
        block
    }

    /// The card's own key controller (for tests).
    #[cfg(test)]
    pub fn key_controller(&self) -> gtk::EventControllerKey {
        let controllers = self.root.observe_controllers();
        (0..controllers.n_items())
            .filter_map(|index| controllers.item(index))
            .find_map(|item| item.downcast::<gtk::EventControllerKey>().ok())
            .expect("the card listens for keys")
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

    /// Each setting's short list, with the button that shows it and its
    /// rows by text (the search row by "") (for tests).
    #[cfg(test)]
    pub fn option_lists(&self) -> Vec<(Ident, gtk::ToggleButton, ChoiceRows)> {
        self.options
            .borrow()
            .iter()
            .map(|list| {
                (
                    list.param.clone(),
                    list.toggle.clone(),
                    list.rows.borrow().clone(),
                )
            })
            .collect()
    }

    /// The scrolled body (for tests).
    #[cfg(test)]
    pub fn scrolled(&self) -> gtk::ScrolledWindow {
        self.scrolled.clone()
    }

    /// Opens the short list of a setting, as pressing its button does;
    /// `false` when the card shows no list for it.
    pub fn open_options(&self, param: &str) -> bool {
        let found = self
            .options
            .borrow()
            .iter()
            .find(|list| list.param.as_str() == param)
            .cloned();
        let Some(list) = found else {
            return false;
        };
        list.toggle.set_active(true);
        self.scroll_to_open_options();
        true
    }

    /// Scrolls the card so the open short list is in sight, once it is
    /// laid out.
    fn scroll_to_open_options(&self) {
        let open = self.open_options.borrow().clone();
        let Some(open) = open else {
            return;
        };
        // The setting's row in the card of rows (a long value's row holds
        // its line and the value in full); its short list comes after it.
        let row = self
            .options
            .borrow()
            .iter()
            .find(|list| list.param.as_str() == open)
            .and_then(|list| {
                let mut at = list.toggle.parent()?;
                while !at.parent()?.has_css_class("stave-edit-rows") {
                    at = at.parent()?;
                }
                Some(at)
            });
        let Some(row) = row else {
            return;
        };
        self.scroll_target.replace(Some(row));
        // Tried until it is laid out and in sight (a few frames at most);
        // the adjustment's own changes try too.
        let target = self.scroll_target.clone();
        let body = self.body.clone();
        let scrolled = self.scrolled.downgrade();
        let tries = Cell::new(0_u32);
        glib::timeout_add_local(std::time::Duration::from_millis(40), move || {
            tries.set(tries.get().saturating_add(1));
            let Some(scrolled) = scrolled.upgrade() else {
                return glib::ControlFlow::Break;
            };
            let done = scroll_to_target(&scrolled.vadjustment(), &body, &target);
            if done || tries.get() > 50 {
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        });
    }

    /// Brings the open short list into sight at once, laying the window
    /// out to find where it is and again to show it scrolled: for an edit
    /// script, whose picture is taken from the last layout (the window's
    /// own frames do this a frame later otherwise).
    pub fn scroll_to_open_options_now(&self) {
        let Some(window) = self
            .root
            .root()
            .and_then(|root| root.downcast::<gtk::Window>().ok())
        else {
            return;
        };
        let (width, height) = (window.width(), window.height());
        if width <= 0 || height <= 0 {
            return;
        }
        window.allocate(width, height, -1, None);
        let adjustment = self.scrolled.vadjustment();
        scroll_to_target(&adjustment, &self.body, &self.scroll_target);
        window.allocate(width, height, -1, None);
    }

    /// The setting whose short list is open, if any.
    pub fn open_setting(&self) -> Option<String> {
        self.open_options.borrow().clone()
    }

    /// The button that puts back the setting a refusal blames, while the
    /// card shows one.
    pub fn put_back_button(&self) -> Option<gtk::Button> {
        self.put_back.borrow().clone()
    }

    /// Each fix's button with its problem and fix index (for tests).
    #[cfg(test)]
    pub fn fix_buttons(&self) -> Vec<((usize, usize), gtk::Button)> {
        self.fixes.borrow().clone()
    }
}

/// The tallest the verdict's own box grows before its words scroll, in
/// logical pixels.
const VERDICT_HEIGHT: i32 = 120;

/// The width the card is laid out for before it has one of its own: the
/// Edit panel's in the default layout.
const LEAST_WIDTH: i32 = 300;

/// A value longer than this many characters may not fit its entry beside
/// the setting's name: it is also said in full under the row.
const LONG_VALUE: usize = 18;

/// Brings the target row near the top of what the card shows, once the
/// layout has room to: then the target is let go, so the user's own
/// scrolling isn't undone.
fn scroll_to_target(
    adjustment: &gtk::Adjustment,
    body: &gtk::Box,
    target: &Rc<RefCell<Option<gtk::Widget>>>,
) -> bool {
    let Some(row) = target.borrow().clone() else {
        return true;
    };
    if !row.is_mapped() {
        return false;
    }
    let Some(bounds) = row.compute_bounds(body) else {
        return false;
    };
    let top = f64::from(bounds.y());
    // The row and its short list, which follows it.
    let list = row
        .next_sibling()
        .and_then(|list| list.compute_bounds(body))
        .map_or(0.0, |list| f64::from(list.height()));
    let bottom = top + f64::from(bounds.height()) + list;
    let (value, page) = (adjustment.value(), adjustment.page_size());
    if top >= value && bottom <= value + page {
        target.replace(None);
        return true;
    }
    // As little as brings the list's foot into sight, never past the
    // row's top.
    let wanted = (bottom - page).min(top - 4.0).max(0.0);
    let reach = adjustment.upper() - page;
    adjustment.set_value(wanted.min(reach));
    if reach >= wanted {
        target.replace(None);
        return true;
    }
    false
}

/// The text of the short list's entry whose row holds the keyboard.
fn focused_choice(list: &ChoiceList) -> Option<String> {
    let focus = list.list.root()?.focus()?;
    list.rows
        .borrow()
        .iter()
        .find(|(_, row)| focus == *row.upcast_ref::<gtk::Widget>() || focus.is_ancestor(row))
        .map(|(text, _)| text.clone())
}

/// A place param's row: what fills it (each name a link to what it
/// names), or the prompt to point at it.
fn place_row(param: &PlaceParam, words: &dyn Words, handlers: &LinkHandlers) -> gtk::Box {
    let row = row_box();
    row.append(&key_label(param.name.as_str(), &param.prompt));
    match param.text.clone().or_else(|| words.place(&param.slot)) {
        Some(value) => {
            let links = words.links(&value);
            if links.is_empty() {
                let label = value_label(&value);
                label.set_tooltip_text(Some(&value));
                row.append(&label);
            } else {
                let shown = handlers.links(&links);
                shown.set_tooltip_text(Some(&value));
                row.append(&shown);
            }
        }
        None => {
            let prompt = gtk::Box::builder()
                .spacing(6)
                .hexpand(true)
                .css_classes(["stave-edit-prompt"])
                .build();
            let icon = gtk::Image::from_icon_name("find-location-symbolic");
            icon.set_valign(gtk::Align::Start);
            prompt.append(&icon);
            // The param's own sentence, from its doc comment.
            let words = if param.prompt.trim().is_empty() {
                place_prompt(param)
            } else {
                param.prompt.clone()
            };
            let label = text(&words, &[]);
            label.set_hexpand(true);
            prompt.append(&label);
            row.append(&prompt);
            row.add_css_class("stave-edit-waiting");
        }
    }
    row
}

/// The fixes added to what is tried, each by its label.
fn applied_section(labels: &[String]) -> gtk::Box {
    let (section, body) = sections::section("Fixes added", &labels.len().to_string(), true);
    let rows = rows_card();
    for label in labels {
        let row = row_box();
        row.append(&gtk::Image::from_icon_name("object-select-symbolic"));
        let words = text(label, &[]);
        words.set_tooltip_text(Some(
            "Written after the settings below, so it wins over them",
        ));
        row.append(&words);
        rows.append(&row);
    }
    body.append(&rows);
    section
}

/// Moves the keyboard into the card, unless it is there already, so
/// Enter and Escape reach it. Does nothing until the card is on screen.
fn take_focus(root: &gtk::Box) {
    if !root.is_mapped() {
        return;
    }
    let Some(window) = root.root() else {
        return;
    };
    let inside = window
        .focus()
        .is_some_and(|focus| focus == *root.upcast_ref::<gtk::Widget>() || focus.is_ancestor(root));
    if !inside {
        root.grab_focus();
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
        .spacing(3)
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

/// A row's name on the left: one line, cut short with "…" when long (in
/// full in its tooltip), so the control beside it keeps its room.
fn key_label(words: &str, about: &str) -> gtk::Label {
    let label = gtk::Label::builder()
        .label(words)
        .xalign(0.0)
        .valign(gtk::Align::Center)
        .width_chars(9)
        .max_width_chars(14)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .css_classes(["stave-key", "stave-edit-key"])
        .build();
    let tooltip = if about.is_empty() {
        words.to_owned()
    } else {
        format!("{words}: {about}")
    };
    label.set_tooltip_text(Some(&tooltip));
    label
}

fn value_label(words: &str) -> gtk::Label {
    gtk::Label::builder()
        .label(words)
        .xalign(0.0)
        .hexpand(true)
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .css_classes(["stave-value", "stave-edit-value"])
        .build()
}

/// One compact row of the card.
fn row_box() -> gtk::Box {
    gtk::Box::builder()
        .spacing(8)
        .css_classes(["stave-card-row", "stave-edit-row"])
        .build()
}

/// The rows' card, at the density of the Details panel's compact rows.
fn rows_card() -> gtk::Box {
    gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .css_classes(["stave-card", "stave-edit-rows"])
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
.stave-edit-needs { font-size: 12px; color: @warning_color; }
button.stave-edit-unavailable .stave-edit-entry-label,
button.stave-edit-unavailable .stave-edit-hint { opacity: 0.5; }
.stave-edit-card { background: @window_bg_color; color: @window_fg_color; }
.stave-edit-body { padding: 10px 12px 8px 12px; }
.stave-edit-title { font-size: 15px; font-weight: 700; }
.stave-edit-origin { font-size: 11px; color: alpha(currentColor, 0.65); }
.stave-edit-hint-line { font-size: 12px; color: alpha(currentColor, 0.7); margin-top: -6px; }
.stave-edit-caption { font-size: 11px; font-weight: 700; letter-spacing: 0.03em; color: alpha(currentColor, 0.6); margin-top: 2px; }
.stave-card.stave-edit-rows { border-radius: 8px; font-size: 13px; }
.stave-card.stave-edit-rows > :first-child { border-top-left-radius: 8px; border-top-right-radius: 8px; }
.stave-card.stave-edit-rows > :last-child { border-bottom-left-radius: 8px; border-bottom-right-radius: 8px; }
.stave-card > .stave-card-row.stave-edit-row { padding: 3px 4px 3px 10px; min-height: 28px; border-spacing: 8px; }
.stave-edit-key { font-size: 13px; }
.stave-edit-card .stave-card.stave-card-row { padding: 3px 4px 3px 10px; border-radius: 8px; }
.stave-edit-long-value { font-family: monospace; font-size: 12px; color: alpha(currentColor, 0.7); margin: 2px 0 2px 2px; }
.stave-edit-long > .stave-edit-setting { border-spacing: 8px; }
.stave-edit-value { font-weight: 600; }
.stave-edit-name { font-weight: 600; }
.stave-edit-name > stack > label { min-height: 26px; }
entry.stave-edit-entry { min-height: 26px; padding-top: 0; padding-bottom: 0; font-size: 13px; }
entry.stave-edit-entry > text > placeholder { opacity: 0.55; }
button.stave-edit-options-toggle { min-height: 26px; min-width: 26px; padding: 0 4px; }
.stave-edit-prompt { color: @accent_color; font-weight: 600; font-size: 12px; }
.stave-card > .stave-edit-row.stave-edit-needs-choice .stave-edit-key { color: @warning_color; opacity: 1; font-weight: 600; }
.stave-edit-choices { padding: 2px 4px 4px 18px; border-bottom: 1px solid alpha(currentColor, 0.08); }
button.stave-edit-choice { padding: 2px 6px; min-height: 24px; font-weight: normal; }
.stave-edit-choice-tag { font-size: 11px; color: alpha(currentColor, 0.6); }
button.stave-edit-choice:not(.stave-edit-unavailable) .stave-edit-choice-mark { color: @accent_color; }
button.stave-edit-search { color: @accent_color; }
.stave-edit-verdict-box { border-radius: 8px; }
.stave-edit-verdict { padding: 6px 10px; border-radius: 8px; background: alpha(currentColor, 0.05); }
.stave-edit-verdict.fits { background: alpha(@success_color, 0.12); }
.stave-edit-verdict.fits .stave-edit-status, .stave-edit-verdict.fits image { color: @success_color; }
.stave-edit-verdict.refused { background: alpha(@error_color, 0.10); }
.stave-edit-verdict.refused > box > image, .stave-edit-verdict.refused .stave-edit-status { color: @error_color; }
.stave-edit-status { font-weight: 700; }
.stave-edit-problem { margin-top: 2px; }
.stave-edit-problem-head { font-family: monospace; font-size: 12px; }
.stave-edit-problem-head.error { color: @error_color; }
.stave-edit-problem-head.warning { color: @warning_color; }
.stave-edit-help, .stave-edit-note { font-family: monospace; font-size: 12px; }
button.stave-edit-fix, button.stave-edit-put-back { padding: 2px 12px; min-height: 26px; }
.stave-edit-foot { border-top: 1px solid alpha(currentColor, 0.1); }
.stave-edit-buttons { padding: 8px 12px 10px 12px; }
.stave-edit-option { font-family: monospace; font-size: 13px; }
button.stave-edit-face { margin-top: 4px; }
button.stave-edit-unavailable .stave-edit-option,
button.stave-edit-unavailable .stave-edit-option-mark,
button.stave-edit-unavailable .stave-edit-choice-mark { opacity: 0.55; }
.stave-edit-blame { padding: 0 0 6px 0; border-bottom: 1px solid alpha(@error_color, 0.3); }
.stave-edit-blame-text { font-weight: 700; }
.stave-repeat-fold { padding: 4px 12px 0 12px; }
.stave-edit-repeat-slot > .stave-repeat-row { padding: 6px 12px 0 12px; }
.stave-repeat-short > label { font-size: 15px; font-weight: 700; }
.stave-repeat-short spinbutton > button { min-width: 22px; padding-left: 2px; padding-right: 2px; }
button.stave-repeat-unfold { padding: 2px 8px; min-height: 24px; font-weight: normal; }
.stave-repeat-fold > .stave-repeat-row { padding: 2px 0; }
.stave-edit-empty { padding: 12px; }
.stave-option-search { padding: 0; }
.stave-option-search-backdrop { background: alpha(black, 0.32); }
.stave-option-search-head { padding: 10px 8px 0 16px; }
.stave-option-search-title { font-size: 15px; font-weight: 700; }
.stave-option-search-group { font-size: 11px; font-weight: 700; letter-spacing: 0.03em; color: alpha(currentColor, 0.6); padding: 8px 12px 2px 12px; }
row.stave-option-search-hit { padding: 4px 12px; }
.stave-option-search-name { font-family: monospace; font-size: 13px; font-weight: 600; }
.stave-option-search-hint { font-size: 12px; color: alpha(currentColor, 0.75); }
.stave-option-search-fit { font-size: 12px; }
.stave-option-search-fit.fits { color: @success_color; }
.stave-option-search-fit.refused { color: @error_color; }
.stave-option-search-fit.needs { color: @warning_color; }
.stave-option-search-fit.checking { color: @accent_color; }
row.stave-option-search-hit.refused .stave-option-search-name,
row.stave-option-search-hit.refused .stave-option-search-hint { opacity: 0.55; }
.stave-option-search-foot { font-size: 12px; color: alpha(currentColor, 0.65); padding: 6px 12px 10px 12px; border-spacing: 16px; }
/* ---- lane: Inspector and editing — the cards as Inspector sections ---- */
.stave-edit-card .stave-edit-body { padding: 0 0 8px 0; border-spacing: 0; }
.stave-edit-card .stave-edit-hint-line { margin: -2px 8px 4px 8px; font-size: 11.5px; }
.stave-edit-card .stave-card.stave-edit-rows { background: none; box-shadow: none; border-radius: 0; font-size: 12px; }
.stave-edit-card .stave-card.stave-edit-rows > .stave-card-row.stave-edit-row { padding: 1px 0; min-height: 22px;
  border-bottom: none; border-radius: 0; border-spacing: 6px; }
.stave-edit-card .stave-edit-key { min-width: 104px; font-size: 12px; color: alpha(currentColor, 0.6); }
.stave-edit-card .stave-edit-value { font-size: 12px; font-weight: normal; padding: 2px 6px; border-radius: 4px;
  background: alpha(black, 0.18); }
.stave-edit-card entry.stave-edit-entry { min-height: 22px; font-size: 12px; }
.stave-edit-card button.stave-edit-options-toggle { min-height: 22px; min-width: 22px; }
.stave-edit-card .stave-edit-verdict { padding: 4px 8px; border-radius: 4px; }
.stave-edit-card .stave-edit-status { font-size: 12px; }
.stave-edit-card .stave-edit-foot { border-top: 1px solid alpha(currentColor, 0.1); }
.stave-edit-card .stave-edit-buttons { padding: 6px 8px 8px 8px; }
.stave-edit-card .stave-edit-name { font-family: monospace; font-weight: normal; }
.stave-edit-card .stave-edit-origin { font-size: 11.5px; }
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
        crate::ui::gtk_test::on_gtk_thread(menu_and_card_report);
    }

    fn menu_and_card_report() {
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

        // The card, on screen in a window that, like Stave's, takes
        // Escape for itself when nothing inside does.
        let card = EditCard::new();
        let actions: Rc<RefCell<Vec<CardAction>>> = Rc::default();
        {
            let actions = actions.clone();
            card.connect_action(move |action| actions.borrow_mut().push(action));
        }
        let window = gtk::Window::builder()
            .default_width(420)
            .default_height(720)
            .build();
        let window_escapes = Rc::new(Cell::new(0));
        {
            let keys = gtk::EventControllerKey::new();
            let window_escapes = window_escapes.clone();
            keys.connect_key_pressed(move |_, key, _, _| {
                if key == gdk::Key::Escape {
                    window_escapes.set(window_escapes.get() + 1);
                    glib::Propagation::Stop
                } else {
                    glib::Propagation::Proceed
                }
            });
            window.add_controller(keys);
        }
        let other = gtk::Button::with_label("Elsewhere");
        let layout = gtk::Box::new(gtk::Orientation::Vertical, 0);
        layout.append(&other);
        layout.append(&card.root);
        window.set_child(Some(&layout));
        // The keyboard starts elsewhere, as it starts in the 3D view.
        gtk::prelude::GtkWindowExt::set_focus(&window, Some(&other));
        window.present();
        let context = glib::MainContext::default();
        let started = std::time::Instant::now();
        while !card.root.is_mapped() && started.elapsed() < std::time::Duration::from_secs(10) {
            context.iteration(false);
        }
        assert!(card.root.is_mapped(), "the window never came up");
        // Appearing, the card takes the keyboard.
        assert_eq!(
            gtk::prelude::GtkWindowExt::focus(&window).as_ref(),
            Some(card.root.upcast_ref::<gtk::Widget>())
        );
        assert_eq!(
            card.key_controller().widget().as_ref(),
            Some(card.root.upcast_ref::<gtk::Widget>())
        );

        // Refused: Add is disabled, Enter does nothing, the fix and
        // Escape are reported, and Escape never reaches the window.
        let show = |candidate, verdict| {
            card.show(
                &CardModel {
                    applied: &[],
                    blame: None,
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
        // The commit button reads Apply, whatever is added.
        assert_eq!(
            crate::ui::sections::button_words(&add).as_deref(),
            Some("Apply")
        );
        assert_eq!(
            crate::ui::sections::button_words(&cancel).as_deref(),
            Some("Cancel")
        );
        assert_eq!(
            press(&window, gdk::Key::Return),
            Some(card.root.clone().upcast())
        );
        let fixes = card.fix_buttons();
        assert_eq!(fixes.len(), 1);
        assert_eq!(
            fixes[0].1.label().as_deref(),
            Some("Remove door_l and door_r")
        );
        fixes[0].1.emit_clicked();
        assert_eq!(
            press(&window, gdk::Key::Escape),
            Some(card.root.clone().upcast())
        );
        assert_eq!(window_escapes.get(), 0);
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
        gtk::prelude::GtkWindowExt::set_focus(&window, Some(&other));
        show(drawer, Some(&fits));
        assert_eq!(
            gtk::prelude::GtkWindowExt::focus(&window).as_ref(),
            Some(card.root.upcast_ref::<gtk::Widget>())
        );
        assert_eq!(
            press(&window, gdk::Key::KP_Enter),
            Some(card.root.clone().upcast())
        );
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
        // Without the card's controller, Escape would fall to the window.
        card.root.remove_controller(&card.key_controller());
        assert_eq!(
            press(&window, gdk::Key::Escape),
            Some(window.clone().upcast())
        );
        assert_eq!(window_escapes.get(), 1);
        window.destroy();
    }

    /// Delivers a key press as GTK does: from the window down to the
    /// focused widget through each key controller set to capture, then
    /// back up through each set to bubble (or target), until one stops
    /// it. Returns the widget whose controller stopped it. (GTK has no
    /// public way to make a key event, so this walks its order.)
    fn press(window: &gtk::Window, key: gdk::Key) -> Option<gtk::Widget> {
        let mut path = Vec::new();
        let mut at = gtk::prelude::GtkWindowExt::focus(window);
        while let Some(widget) = at {
            at = widget.parent();
            path.push(widget);
        }
        let keys_of = |widget: &gtk::Widget| {
            let controllers = widget.observe_controllers();
            (0..controllers.n_items())
                .filter_map(|index| controllers.item(index))
                .filter_map(|item| item.downcast::<gtk::EventControllerKey>().ok())
                .collect::<Vec<_>>()
        };
        let stops = |controller: &gtk::EventControllerKey| {
            controller.emit_by_name::<bool>(
                "key-pressed",
                &[
                    &glib::translate::IntoGlib::into_glib(key),
                    &0u32,
                    &gdk::ModifierType::empty(),
                ],
            )
        };
        for widget in path.iter().rev() {
            for controller in keys_of(widget) {
                if controller.propagation_phase() == gtk::PropagationPhase::Capture
                    && stops(&controller)
                {
                    return Some(widget.clone());
                }
            }
        }
        for widget in &path {
            for controller in keys_of(widget) {
                let phase = controller.propagation_phase();
                if matches!(
                    phase,
                    gtk::PropagationPhase::Bubble | gtk::PropagationPhase::Target
                ) && stops(&controller)
                {
                    return Some(widget.clone());
                }
            }
        }
        None
    }
}
