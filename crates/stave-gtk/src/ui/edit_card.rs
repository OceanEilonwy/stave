//! The card for a chosen candidate (`docs/editing.md`, "Verdicts and
//! strict commit"), the content of the docked Edit panel, drawn as the
//! Inspector is (`super::sections`): the name it would be written under
//! in the header's monospace field, then sections — Placed in (what it
//! fills, each space and part a link, or what is still to point at), the
//! verdict with the checker's problems and fixes, its settings one compact
//! label | value row each, and the lines Apply would write (Pending).
//! It draws the contract's values (`stave_app::edit`, `stave_app::choices`)
//! and reports what the user did as a [`CardAction`]; it decides nothing
//! about the design.
//!
//! ```text
//! ┌───────────────────────────────────────┐
//! │ [case                       ]    Case │  (the name, retyped in place)
//! │ A cabinet case: a base, two sides an… │
//! │ ▾ Placed in                           │
//! │   space        (envelope)             │  (a link: lit on hover)
//! │ ▾ Verdict                             │
//! │   ✓ It fits                           │  (scrolls in its own box)
//! │ ▾ Settings                         4  │
//! │   panel_stock  [oak_19            ▾]  │
//! │   joinery      [dowel(count: 4)   ▾]  │  (the default, dimmed)
//! │     ✓ dowel(count: 4)      default    │  (its short list, opened)
//! │       screw                used       │
//! │       Search all 18…                  │
//! │ ▾ Pending                             │  (the window's, `editor_link.rs`)
//! │ (the body scrolls)                    │
//! ├───────────────────────────────────────┤
//! │ Repeat…                               │  (the window's repeat fold)
//! │       [Cancel  Esc]  [Apply  Enter]   │  (always in sight)
//! └───────────────────────────────────────┘
//! ```
//!
//! Keys: Enter applies when Apply is enabled; Escape cancels. The keys reach the
//! card while the keyboard is in it, so the card takes the keyboard when it
//! appears and each time it is shown again (unless the keyboard is already
//! inside it, say in a setting being typed).
//!
//! It is a relm4 component, [`EditCardComponent`], owned by the window
//! (docs/relm4-migration.md): its widgets only send it messages and its
//! model alone holds the card's state. The window never asks it anything:
//! what the card offers follows from the [`CardView`] the window built
//! ([`CardView::offers`]), and the one thing people change on it that the
//! window wants to know, which short list is open, the card tells it
//! ([`EditCardOutput::OptionsOpen`]).

use stave_app::i18n::{fill_in, ntr, tr};

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

use relm4::{Component, ComponentParts, ComponentSender};
use stave_app::inspect::{LinkTarget, NameLink};

use super::sections::{self, LinkHandlers};

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
pub fn add_blocked_reason(candidate: &Candidate, verdict: Option<&Verdict>) -> Option<String> {
    if candidate.unbound_place().next().is_some() {
        Some(tr("Point at what is still missing on the model first"))
    } else if candidate.unset_settings().next().is_some() {
        Some(tr("Choose the settings marked as needing a choice first"))
    } else {
        match verdict {
            Some(Verdict::Fits(_)) => None,
            Some(Verdict::Checking) | None => Some(tr("Wait until the check has finished")),
            Some(Verdict::Refused(_)) => {
                Some(tr("It doesn't fit yet: use a fix or change a setting"))
            }
        }
    }
}

/// The prompt for a place param still to point at: "Click a space on the
/// model for opening".
pub fn place_prompt(param: &PlaceParam) -> String {
    let template = match param.slot {
        PlaceSlot::Region(_) => tr("Click a space on the model for {name}"),
        PlaceSlot::Anchor(_) => tr("Click a face of a part on the model for {name}"),
        PlaceSlot::Part(_) => tr("Click a part on the model for {name}"),
        PlaceSlot::Parts(_) => tr("Click the parts on the model for {name}"),
    };
    fill_in(&template, &[("name", &spaced(param.name.as_str()))])
}

/// A name with its underscores as spaces, for a sentence: `hinge side`.
fn spaced(name: &str) -> String {
    name.replace('_', " ")
}

/// Where a setting's value comes from, in words.
pub fn source_words(value: &SettingValue, words: &dyn Words) -> String {
    match value {
        SettingValue::Default => tr("the default"),
        SettingValue::Copied { from, .. } => fill_in(
            &tr("copied from {assembly}"),
            &[("assembly", &words.assembly(*from))],
        ),
        SettingValue::Chosen(_) => tr("your choice"),
        SettingValue::Unset => tr("needs a choice"),
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
        (None, None) => tr("Use the default"),
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
        (SettingValue::Default, Some(text), Some(value)) if text != value => fill_in(
            &tr("the default, {text} = {value}"),
            &[("text", &text), ("value", &value)],
        ),
        (SettingValue::Default, Some(text), _) => {
            fill_in(&tr("the default, {text}"), &[("text", &text)])
        }
        (SettingValue::Unset, _, _) => tr("needs a choice before it can be tried"),
        _ => source_words(&setting.value, words),
    };
    let about = format!("{} · {source}", setting.ty.word());
    match &setting.range {
        Some(range) => format!(
            "{about} · {}",
            fill_in(
                &tr("{low} to {high}"),
                &[("low", &range.low), ("high", &range.high)],
            )
        ),
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
        OptionFit::NotChecked => tr("Checking whether it fits…"),
        OptionFit::Fits => tr("Fits"),
        OptionFit::NeedsChoice(names) => {
            let names: Vec<String> = names.iter().map(|n| spaced(n.as_str())).collect();
            fill_in(
                &tr("Checked once you choose {names}"),
                &[("names", &names.join(", "))],
            )
        }
        OptionFit::Refused(refusal) => fill_in(
            &tr("Can't be used here: {reason}"),
            &[("reason", &refusal.line())],
        ),
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

/// What the card says about one place param: what fills it, as written,
/// with the names in it as links; no value while it is still to point
/// at.
#[derive(Debug, Clone, PartialEq)]
struct PlaceWords {
    value: Option<String>,
    links: Vec<NameLink>,
}

/// What the card says about one setting: the value written for it, if
/// there is one to show, and its type and where its value comes from
/// (the row's tooltip, [`setting_about`]).
#[derive(Debug, Clone, PartialEq)]
struct SettingWords {
    value: Option<String>,
    about: String,
}

/// What the card shows, owned, with the words for the contract's ids and
/// expressions already said, so it can travel in a message
/// ([`EditCardInput::Show`]).
#[derive(Debug, Clone)]
pub struct CardView {
    heading: Heading,
    candidate: Candidate,
    name: String,
    verdict: Option<Verdict>,
    applied: Vec<String>,
    blame: Option<Blame>,
    /// One for each of the candidate's place params, in order.
    places: Vec<PlaceWords>,
    /// One for each of the candidate's settings, in order.
    settings: Vec<SettingWords>,
}

impl CardView {
    /// What the card shows for `model`, its ids and expressions said in
    /// `words`.
    pub fn new(model: &CardModel<'_>, words: &dyn Words) -> CardView {
        let candidate = model.candidate;
        let places = candidate
            .place
            .iter()
            .map(|param| {
                let value = param.text.clone().or_else(|| words.place(&param.slot));
                let links = value
                    .as_deref()
                    .map(|value| words.links(value))
                    .unwrap_or_default();
                PlaceWords { value, links }
            })
            .collect();
        let settings = candidate
            .settings
            .iter()
            .map(|setting| SettingWords {
                value: value_text(&setting.value, words),
                about: setting_about(setting, words),
            })
            .collect();
        CardView {
            heading: model.heading,
            candidate: candidate.clone(),
            name: model.name.to_owned(),
            verdict: model.verdict.cloned(),
            applied: model.applied.to_vec(),
            blame: model.blame.cloned(),
            places,
            settings,
        }
    }

    /// `true` when Add may be pressed ([`can_add`]).
    fn can_add(&self) -> bool {
        can_add(&self.candidate, self.verdict.as_ref())
    }

    /// The setting change a refusal is blamed on, while the card says so
    /// (with the button that puts it back).
    fn blamed(&self) -> Option<&Blame> {
        match (&self.blame, &self.verdict) {
            (Some(blame), Some(Verdict::Refused(_))) => Some(blame),
            (Some(_) | None, _) => None,
        }
    }

    /// What a card drawn from this view offers, worked out the way the
    /// card draws it, so the owner knows without asking the card.
    pub fn offers(&self) -> CardOffers {
        CardOffers {
            short_lists: self
                .candidate
                .settings
                .iter()
                .filter(|setting| shows_short_list(setting, &short_list(setting)))
                .map(|setting| setting.name.as_str().to_owned())
                .collect(),
            put_back: self.blamed().is_some(),
        }
    }
}

/// What a card drawn from a [`CardView`] offers: what its owner would
/// otherwise have to ask the card.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CardOffers {
    /// The settings with a short list, by name.
    short_lists: Vec<String>,
    /// Whether it shows the button that puts back a blamed setting.
    put_back: bool,
}

impl CardOffers {
    /// `true` when the card shows a short list for this setting.
    pub fn has_short_list(&self, setting: &str) -> bool {
        self.short_lists.iter().any(|name| name == setting)
    }

    /// `true` when the card shows the button that puts back the setting a
    /// refusal blames.
    pub fn put_back(&self) -> bool {
        self.put_back
    }
}

/// `true` when a setting's row has a short list: values to choose from,
/// or options to search.
fn shows_short_list(setting: &SettingParam, choices: &[ShortChoice]) -> bool {
    !choices.is_empty() || !setting.options.is_empty()
}

/// What people did in the card's body, as its widgets report it.
#[derive(Debug, Clone, PartialEq)]
pub enum BodyEvent {
    /// A button or a setting's entry asks for this.
    Action(CardAction),
    /// People chose an entry of a short list: the list closes, then this.
    Picked(CardAction),
    /// People pressed a short list's button: it opened (`true`) or
    /// closed.
    Toggled {
        /// The setting.
        param: String,
        /// Whether it is open now.
        open: bool,
    },
}

/// Where the body's widgets report what people do: a closure that only
/// passes the event on (to a component's input, or to the old callback).
trait Report: Fn(BodyEvent) + Clone + 'static {}

impl<R: Fn(BodyEvent) + Clone + 'static> Report for R {}

/// One setting's short list on the card: shown in place under its row when
/// its button is pressed (a popover would close each time a check's answer
/// redraws it, and can't be drawn into a screenshot).
#[derive(Clone)]
struct ChoiceList {
    /// The setting.
    param: Ident,
    /// The button that shows and hides it.
    toggle: gtk::ToggleButton,
    /// The rows' box.
    list: gtk::Box,
    /// Each entry's row, by its text; the search row last, by "".
    rows: Vec<(String, gtk::Button)>,
}

/// The widgets a show draws that the card keeps: each setting's entry,
/// each fix's button, each short list and the put-back button.
#[derive(Clone, Default)]
struct Built {
    /// Each setting's entry, by param.
    entries: Vec<(Ident, gtk::Entry)>,
    /// Each fix's button, with its problem and fix index.
    fixes: Vec<((usize, usize), gtk::Button)>,
    /// Each setting's short list.
    options: Vec<ChoiceList>,
    /// The put-back button, while a refusal is blamed on a setting.
    put_back: Option<gtk::Button>,
}

/// The card's frame: what stays from one show to the next.
#[derive(Clone, Debug)]
struct CardFrame {
    /// The body, filled by each show; it scrolls.
    body: gtk::Box,
    scrolled: gtk::ScrolledWindow,
    /// The name it is written under, retyped in place.
    name: gtk::EditableLabel,
    add: gtk::Button,
    cancel: gtk::Button,
    /// Where the window puts the "Pending" section, last in the body.
    pending_slot: gtk::Box,
}

/// The card's root: it holds the keyboard itself, so Enter and Escape work
/// before anything in it is clicked. Empty until [`fill_card`].
fn card_root() -> gtk::Box {
    if let Some(display) = gdk::Display::default() {
        install_style(&display);
    }
    gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .css_classes(["stave-edit-card"])
        .hexpand(true)
        .vexpand(true)
        .accessible_role(gtk::AccessibleRole::Group)
        .focusable(true)
        .build()
}

/// The boxes on the card that its owner fills: made by the owner, which
/// keeps a clone of each, and given to the card when it is launched, as
/// they stay from one show to the next.
#[derive(Debug, Clone)]
pub struct CardSlots {
    /// Where the owner puts the repeat controls' count and direction,
    /// above the buttons.
    pub repeat_slot: gtk::Box,
    /// Where the owner puts "Repeat…", on the buttons' line.
    pub repeat_button_slot: gtk::Box,
    /// Where the owner puts the "Pending" section (the lines Apply would
    /// write), last in the body.
    pub pending_slot: gtk::Box,
}

impl CardSlots {
    /// Three empty slots.
    pub fn new() -> CardSlots {
        CardSlots {
            repeat_slot: gtk::Box::builder()
                .orientation(gtk::Orientation::Vertical)
                .css_classes(["stave-edit-repeat-slot"])
                .build(),
            repeat_button_slot: gtk::Box::builder()
                .hexpand(true)
                .halign(gtk::Align::Start)
                .valign(gtk::Align::Center)
                .build(),
            pending_slot: gtk::Box::builder()
                .orientation(gtk::Orientation::Vertical)
                .build(),
        }
    }
}

/// Puts the card's frame in its root: the scrolled body, then the foot
/// with the repeat slots and Cancel and Apply. No handlers are connected.
fn fill_card(root: &gtk::Box, slots: CardSlots) -> CardFrame {
    let CardSlots {
        repeat_slot,
        repeat_button_slot,
        pending_slot,
    } = slots;
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
        .tooltip_text(tr("The name it is written under. Click to retype it."))
        .build();
    name.update_property(&[gtk::accessible::Property::Label(&tr("Name"))]);
    let (apply_buttons, cancel, add) = sections::footer_buttons();
    let buttons = gtk::Box::builder()
        .spacing(8)
        .css_classes(["stave-edit-buttons"])
        .build();
    buttons.append(&repeat_button_slot);
    buttons.append(&apply_buttons);
    let foot = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .css_classes(["stave-edit-foot"])
        .build();
    foot.append(&repeat_slot);
    foot.append(&buttons);
    root.append(&scrolled);
    root.append(&foot);
    CardFrame {
        body,
        scrolled,
        name,
        add,
        cancel,
        pending_slot,
    }
}

/// What a key does on the card: Escape cancels; Enter adds, when Add is
/// enabled. Either is the card's alone, so it never reaches the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CardKey {
    Escape,
    Enter,
}

impl CardKey {
    fn of(key: gdk::Key) -> Option<CardKey> {
        match key {
            gdk::Key::Escape => Some(CardKey::Escape),
            gdk::Key::Return | gdk::Key::KP_Enter | gdk::Key::ISO_Enter => Some(CardKey::Enter),
            _ => None,
        }
    }
}

/// Listens for Enter and Escape on the card, passing them to `pressed`.
fn listen_for_keys(root: &gtk::Box, pressed: impl Fn(CardKey) + 'static) {
    let keys = gtk::EventControllerKey::new();
    keys.connect_key_pressed(move |_, key, _, _| match CardKey::of(key) {
        Some(card_key) => {
            pressed(card_key);
            glib::Propagation::Stop
        }
        None => glib::Propagation::Proceed,
    });
    root.add_controller(keys);
}

/// Draws a candidate, its name and its verdict into the card's body, and
/// returns the widgets the card keeps. `open` is the setting whose short
/// list stays open. Add, the keyboard and scrolling are the caller's.
fn build_body(
    root: &gtk::Box,
    frame: &CardFrame,
    view: &CardView,
    open: Option<&str>,
    links: &LinkHandlers,
    report: &impl Report,
) -> Built {
    let body = &frame.body;
    while let Some(child) = body.first_child() {
        body.remove(&child);
    }
    let candidate = &view.candidate;
    let label = candidate.entry.label.as_str();
    root.update_property(&[gtk::accessible::Property::Label(&fill_in(
        &tr("Add {name}"),
        &[("name", &label)],
    ))]);

    // The header: the name it is written under, retyped in place, in a
    // monospace field, and what it is and where it comes from on the
    // right; the hint under it on one line, in full in its tooltip.
    frame.name.set_text(&view.name);
    if let Some(parent) = frame.name.parent()
        && let Ok(parent) = parent.downcast::<gtk::Box>()
    {
        parent.remove(&frame.name);
    }
    let from = text(label, &["stave-edit-origin"]);
    from.set_wrap(false);
    from.set_xalign(1.0);
    from.set_ellipsize(gtk::pango::EllipsizeMode::End);
    from.set_max_width_chars(18);
    from.set_tooltip_text(Some(&format!(
        "{label} · {}",
        origin(view.heading, &candidate.entry.group)
    )));
    body.append(&sections::header(
        None,
        &frame.name,
        Some(from.upcast_ref()),
    ));
    if let Some(hint) = &candidate.entry.hint {
        let hint_label = text(hint, &["stave-edit-hint-line"]);
        hint_label.set_wrap(false);
        hint_label.set_ellipsize(gtk::pango::EllipsizeMode::End);
        hint_label.set_tooltip_text(Some(hint));
        body.append(&hint_label);
    }

    // Where it goes: each place it fills, its names as links.
    if !candidate.place.is_empty() {
        let (section, rows) = sections::section(&tr("Placed in"), "", true);
        for (param, words) in candidate.place.iter().zip(&view.places) {
            rows.append(&place_row(param, words, links));
        }
        body.append(&section);
    }

    if !view.applied.is_empty() {
        body.append(&applied_section(&view.applied));
    }
    let mut built = Built::default();
    let (section, verdict_body) = sections::section(&tr("Verdict"), "", true);
    verdict_body.append(&verdict_section(view, root.width(), report, &mut built));
    body.append(&section);
    if !candidate.settings.is_empty() {
        body.append(&settings_section(view, open, report, &mut built));
    }
    body.append(&frame.pending_slot);
    built
}

/// Enables Add when it may be pressed, its tooltip saying why not when it
/// may not. Returns whether it is enabled.
fn show_add(add: &gtk::Button, view: &CardView) -> bool {
    let enabled = view.can_add();
    add.set_sensitive(enabled);
    add.set_tooltip_text(Some(
        &add_blocked_reason(&view.candidate, view.verdict.as_ref())
            .unwrap_or_else(|| tr("Write it into the design (Enter)")),
    ));
    enabled
}

/// The settings, one row each: those that need a choice first, then those
/// with a value, then those left at their defaults (the default's value
/// dimmed in the entry). The type and where each value comes from are the
/// row's tooltip, not a line of its own.
fn settings_section(
    view: &CardView,
    open: Option<&str>,
    report: &impl Report,
    built: &mut Built,
) -> gtk::Box {
    let settings = &view.candidate.settings;
    let (section, body) = sections::section(&tr("Settings"), &settings.len().to_string(), true);
    let order = |(setting, _): &(&SettingParam, &SettingWords)| match setting.value {
        SettingValue::Unset => 0,
        SettingValue::Copied { .. } | SettingValue::Chosen(_) => 1,
        SettingValue::Default => 2,
    };
    let mut ordered: Vec<(&SettingParam, &SettingWords)> =
        settings.iter().zip(&view.settings).collect();
    ordered.sort_by_key(order);
    let rows = rows_card();
    for (setting, words) in ordered {
        let (row, entry, list) = setting_row(setting, words, open, report);
        rows.append(&row);
        if let Some(list) = list {
            if let Some(revealer) = list.list.parent() {
                rows.append(&revealer);
            }
            built.options.push(list);
        }
        built.entries.push((setting.name.clone(), entry));
    }
    body.append(&rows);
    section
}

/// One setting's row: its name on the left, its value on the right in an
/// entry (typed, Enter sets it) with the button that opens its short
/// list. Its short list, when it has one, goes under the row.
fn setting_row(
    setting: &SettingParam,
    words: &SettingWords,
    open: Option<&str>,
    report: &impl Report,
) -> (gtk::Box, gtk::Entry, Option<ChoiceList>) {
    let name = setting.name.as_str();
    let about = &words.about;
    let line = row_box();
    line.add_css_class("stave-edit-setting");
    line.set_tooltip_text(Some(about));
    let key = key_label(name, about);
    line.append(&key);
    let value = &words.value;
    let unset = matches!(setting.value, SettingValue::Unset);
    let entry = gtk::Entry::builder()
        .text(value.as_deref().unwrap_or(""))
        .placeholder_text(match setting.value {
            SettingValue::Unset => tr("Choose…"),
            SettingValue::Default => default_placeholder(setting),
            SettingValue::Copied { .. } | SettingValue::Chosen(_) => String::new(),
        })
        .hexpand(true)
        .width_chars(4)
        .css_classes(["stave-edit-entry"])
        .tooltip_text(match value {
            Some(value) => format!("{value}\n{about}"),
            None => format!("{}\n{about}", default_placeholder(setting)),
        })
        .build();
    entry.update_property(&[
        gtk::accessible::Property::Label(name),
        gtk::accessible::Property::Description(about),
    ]);
    if unset {
        entry.add_css_class("warning");
        line.add_css_class("stave-edit-needs-choice");
    }
    {
        let report = report.clone();
        let param = setting.name.clone();
        entry.connect_activate(move |entry| {
            let typed = entry.text().trim().to_owned();
            if typed.is_empty() {
                return;
            }
            report(BodyEvent::Action(CardAction::Choose {
                param: param.clone(),
                text: typed,
            }));
        });
    }
    let control = gtk::Box::builder()
        .css_classes(["linked", "stave-edit-control"])
        .hexpand(true)
        .build();
    control.append(&entry);
    let choices = short_list(setting);
    let list = shows_short_list(setting, &choices)
        .then(|| choice_list(setting, &choices, open == Some(name), report));
    if let Some(list) = &list {
        control.append(&list.toggle);
    }
    line.append(&control);
    // A long value is said in full under its row, wrapped, so it is never
    // cut off (`overlay(left: 18mm, right: 18mm, …)`).
    let shown = value
        .clone()
        .or_else(|| (setting.value == SettingValue::Default).then(|| default_placeholder(setting)));
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
/// default, the values the project already uses, the value it holds now,
/// then "Search…" over every option; and the button that shows it in
/// place under the setting. The list sits in a revealer, the rows' box's
/// parent, which follows the button.
fn choice_list(
    setting: &SettingParam,
    choices: &[ShortChoice],
    open: bool,
    report: &impl Report,
) -> ChoiceList {
    let name = setting.name.as_str();
    let list = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .css_classes(["stave-edit-choices"])
        .accessible_role(gtk::AccessibleRole::List)
        .build();
    list.update_property(&[gtk::accessible::Property::Label(&fill_in(
        &tr("What {setting} can be"),
        &[("setting", &spaced(name))],
    ))]);
    // A slide (of no length, so it is instant): a revealer that doesn't
    // slide keeps the room of what it hides.
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
        .tooltip_text(fill_in(
            &tr("Choose {setting}"),
            &[("setting", &spaced(name))],
        ))
        .build();
    toggle.update_property(&[gtk::accessible::Property::Label(&fill_in(
        &tr("Choose {setting}"),
        &[("setting", &spaced(name))],
    ))]);
    toggle.update_state(&[gtk::accessible::State::Expanded(Some(open))]);
    // The list shows while the button is down: a binding, which holds
    // neither widget.
    toggle
        .bind_property("active", &revealer, "reveal-child")
        .sync_create()
        .build();
    {
        let report = report.clone();
        let param = name.to_owned();
        toggle.connect_toggled(move |toggle| {
            let open = toggle.is_active();
            toggle.update_state(&[gtk::accessible::State::Expanded(Some(open))]);
            report(BodyEvent::Toggled {
                param: param.clone(),
                open,
            });
        });
    }
    let mut choice_list = ChoiceList {
        param: setting.name.clone(),
        toggle,
        list,
        rows: Vec::new(),
    };
    fill_choices(&mut choice_list, choices, setting.options.len(), report);
    choice_list
}

/// The rows of a short list, from the setting as the session has it.
fn fill_choices(
    list: &mut ChoiceList,
    choices: &[ShortChoice],
    options: usize,
    report: &impl Report,
) {
    while let Some(child) = list.list.first_child() {
        list.list.remove(&child);
    }
    let mut rows = Vec::with_capacity(choices.len().saturating_add(1));
    for choice in choices {
        let row = choice_row(&list.param, choice, report);
        list.list.append(&row);
        rows.push((choice.text.clone(), row));
    }
    if options > 0 {
        let row = search_row(&list.param, options, report);
        list.list.append(&row);
        rows.push((String::new(), row));
    }
    list.rows = rows;
}

/// One entry of a short list: a tick for the value it holds now, its
/// text, why it is listed ("default", "used", "now") and its fit, never by
/// colour alone. A refused one is greyed and never reports.
fn choice_row(param: &Ident, choice: &ShortChoice, report: &impl Report) -> gtk::Button {
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
        ChoiceRole::Default => tr("default"),
        ChoiceRole::Recent => tr("used here"),
        ChoiceRole::Current => tr("now"),
    };
    content.append(&text(&tag, &["stave-edit-choice-tag"]));
    let button = gtk::Button::builder()
        .child(&content)
        .css_classes(["flat", "stave-edit-choice"])
        .build();
    let state = choice.fit.as_ref().map(fit_words);
    let description = [Some(tag.to_owned()), state, choice.hint.clone()]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(". ");
    button.update_property(&[
        gtk::accessible::Property::Label(&choice.text),
        gtk::accessible::Property::Description(&description),
    ]);
    button.set_tooltip_text(Some(&description));
    if choice.choosable() {
        let report = report.clone();
        let param = param.clone();
        let chosen = choice.text.clone();
        let default = choice.role == ChoiceRole::Default;
        button.connect_clicked(move |_| {
            report(BodyEvent::Picked(if default {
                CardAction::UseDefault {
                    param: param.clone(),
                }
            } else {
                CardAction::Choose {
                    param: param.clone(),
                    text: chosen.clone(),
                }
            }));
        });
    } else {
        // Still focusable, so the reason can be read, but marked
        // unavailable and never reported.
        button.add_css_class("stave-edit-unavailable");
        button.update_state(&[gtk::accessible::State::Disabled(true)]);
    }
    button
}

/// "Search all 18…": the dialog over every option of the setting.
fn search_row(param: &Ident, options: usize, report: &impl Report) -> gtk::Button {
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let icon = gtk::Image::from_icon_name("system-search-symbolic");
    icon.set_pixel_size(14);
    content.append(&icon);
    let words = text(
        &fill_in(
            &ntr("Search the {count} option…", "Search all {count}…", options),
            &[("count", &options)],
        ),
        &[],
    );
    words.set_hexpand(true);
    content.append(&words);
    let button = gtk::Button::builder()
        .child(&content)
        .css_classes(["flat", "stave-edit-choice", "stave-edit-search"])
        .tooltip_text(tr(
            "Search every option by its name and what it is, each with whether it fits",
        ))
        .build();
    let report = report.clone();
    let param = param.clone();
    button.connect_clicked(move |_| {
        report(BodyEvent::Action(CardAction::Search {
            param: param.clone(),
        }));
    });
    button
}

/// Redraws a short list's rows from `candidate`'s setting of the same
/// name, keeping the keyboard on the entry it was on. Returns the row to
/// give the keyboard back to, if any: the caller focuses it once nothing
/// of its own is borrowed.
fn refill_choices(
    list: &mut ChoiceList,
    candidate: &Candidate,
    report: &impl Report,
) -> Option<gtk::Button> {
    let setting = candidate
        .settings
        .iter()
        .find(|setting| setting.name.as_str() == list.param.as_str())?;
    let focused = focused_choice(list);
    fill_choices(list, &short_list(setting), setting.options.len(), report);
    let focused = focused?;
    list.rows
        .iter()
        .find(|(text, _)| *text == focused)
        .map(|(_, row)| row.clone())
}

/// The verdict: checking, fits (with its warnings), or refused with each
/// problem's message, help and fixes. Its words scroll inside a box of
/// their own, so a long refusal never pushes the settings, Add or Cancel
/// out of reach. `width` is the card's.
fn verdict_section(
    view: &CardView,
    width: i32,
    report: &impl Report,
    built: &mut Built,
) -> gtk::Widget {
    let verdict = view.verdict.as_ref();
    let section = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(6)
        .css_classes(["stave-edit-verdict"])
        .build();
    // What the user just did comes first, with the way back; the checker's
    // own words stay below.
    if let Some(blame) = view.blamed() {
        let (block, button) = blame_block(blame, report);
        section.append(&block);
        built.put_back = Some(button);
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
            let missing = if view.candidate.unbound_place().next().is_some() {
                tr("Point at what is missing on the model; then it is checked.")
            } else {
                tr("Choose the settings that need a choice; then it is checked.")
            };
            let words = text(&missing, &["stave-description"]);
            words.set_hexpand(true);
            status.append(&words);
        }
        Some(Verdict::Checking) => {
            status.append(&gtk::Spinner::builder().spinning(true).build());
            status.append(&text(
                &tr("Checking whether it fits…"),
                &["stave-edit-status"],
            ));
        }
        Some(Verdict::Fits(fits)) => {
            section.add_css_class("fits");
            status.append(&gtk::Image::from_icon_name("object-select-symbolic"));
            status.append(&text(&tr("It fits"), &["stave-edit-status"]));
            for warning in fits.warnings() {
                words.append(&diagnostic_block(warning));
            }
        }
        Some(Verdict::Refused(refusal)) => {
            section.add_css_class("refused");
            status.append(&gtk::Image::from_icon_name("dialog-error-symbolic"));
            let count = refusal.problems().iter().count();
            status.append(&text(
                &fill_in(
                    &ntr(
                        "It doesn't fit: {count} problem",
                        "It doesn't fit: {count} problems",
                        count,
                    ),
                    &[("count", &count)],
                ),
                &["stave-edit-status"],
            ));
            for (problem_index, problem) in refusal.problems().iter().enumerate() {
                words.append(&problem_block(
                    problem_index,
                    problem,
                    &buttons,
                    report,
                    built,
                ));
            }
        }
    }
    // The checker's words scroll in a box of their own, as tall as they
    // are at the card's width up to the bound (a scrolled box inside the
    // card's own scrolled body is otherwise squeezed to its scrollbar's
    // height); the blame, the verdict's line and the buttons (the fixes,
    // each with its problem in its tooltip) stay in sight.
    if words.first_child().is_some() {
        let across = width.max(LEAST_WIDTH).saturating_sub(24);
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
fn blame_block(blame: &Blame, report: &impl Report) -> (gtk::Box, gtk::Button) {
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
        .tooltip_text(tr("Put the setting back as it was, when it still fitted"))
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
    let reporter = report.clone();
    button.connect_clicked(move |_| reporter(BodyEvent::Action(CardAction::PutBack)));
    block.append(&button);
    (block, button)
}

/// One problem: its message and help as the checker words them, and a
/// button per fix.
fn problem_block(
    problem_index: usize,
    problem: &Problem,
    buttons: &gtk::Box,
    report: &impl Report,
    built: &mut Built,
) -> gtk::Box {
    let block = diagnostic_block(problem.diagnostic());
    if problem.fixes().is_empty() {
        block.append(&text(
            &tr("No fix can be applied for you: change a setting or point somewhere else."),
            &["stave-footnote"],
        ));
        return block;
    }
    // Each fix's button on its own line, its label wrapped (a FlowBox asks
    // for its widest child, which held the card open wider than its
    // panel), under the checker's words, out of their scrolling.
    let head = diagnostic_head(problem.diagnostic());
    for (fix_index, fix) in problem.fixes().iter().enumerate() {
        let label = fix.label();
        let button = gtk::Button::builder()
            .label(&label)
            .halign(gtk::Align::Start)
            .css_classes(["pill", "stave-edit-fix"])
            .tooltip_text(fill_in(
                &tr("Add this change to what is tried. It fixes {problem}"),
                &[("problem", &head)],
            ))
            .build();
        if let Some(child) = button.child()
            && let Ok(label) = child.downcast::<gtk::Label>()
        {
            label.set_wrap(true);
            label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        }
        let report = report.clone();
        button.connect_clicked(move |_| {
            report(BodyEvent::Action(CardAction::Fix {
                problem: problem_index,
                fix: fix_index,
            }));
        });
        buttons.append(&button);
        built.fixes.push(((problem_index, fix_index), button));
    }
    block
}

/// The row a short list belongs to, in the card of rows (a long value's
/// row holds its line and the value in full); its short list comes after
/// it.
fn options_row(list: &ChoiceList) -> Option<gtk::Widget> {
    let mut at = list.toggle.parent()?;
    while !at.parent()?.has_css_class("stave-edit-rows") {
        at = at.parent()?;
    }
    Some(at)
}

/// Lays the window out to find where the open short list is, scrolls it
/// into sight and lays the window out again to show it scrolled: for an
/// edit script, whose picture is taken from the last layout. Returns
/// whether the target is in sight (so let go).
fn scroll_now(root: &gtk::Box, frame: &CardFrame, target: &gtk::Widget) -> bool {
    let Some(window) = root
        .root()
        .and_then(|root| root.downcast::<gtk::Window>().ok())
    else {
        return false;
    };
    let (width, height) = (window.width(), window.height());
    if width <= 0 || height <= 0 {
        return false;
    }
    window.allocate(width, height, -1, None);
    let done = scroll_to_target(&frame.scrolled.vadjustment(), &frame.body, target);
    window.allocate(width, height, -1, None);
    done
}

/// The card for a chosen candidate as a relm4 component. Its model is the
/// card's state: what the last show drew, the open short list, the name
/// as shown and whether Add is enabled. Only
/// [`EditCardComponent::update_with_view`] changes it; the card's widgets
/// only send [`EditCardInput`] messages, so nothing they do runs while an
/// update does.
pub struct EditCardComponent {
    /// The setting whose short list is open, kept across redraws.
    open_options: Option<String>,
    /// The name as last shown, so only a retyped one is reported.
    shown_name: String,
    add_enabled: bool,
    /// What the last show drew that the card keeps.
    built: Built,
    /// Where the links in "Placed in" report: to the card's own input.
    links: LinkHandlers,
    /// A row to bring into sight once the layout has room for it.
    scroll_target: Option<gtk::Widget>,
}

/// What the card is told: what to show, and what people did on it.
#[derive(Debug)]
pub enum EditCardInput {
    /// Show this candidate, its name and its verdict.
    Show(Box<CardView>),
    /// Redraw the settings' short lists from this candidate (the one the
    /// card shows) without rebuilding the card.
    UpdateOptions(Box<Candidate>),
    /// Open the short list of this setting, as pressing its button does
    /// (nothing, when the card shows no list for it: the owner knows
    /// which it shows from [`CardView::offers`]).
    OpenOptions(String),
    /// Bring the open short list into sight at once, laying the window
    /// out (for an edit script's picture).
    ScrollToOpenOptions,
    /// People did something in the body.
    Body(BodyEvent),
    /// People pressed Add.
    Add,
    /// People pressed Cancel, or Escape.
    Cancel,
    /// People pressed Enter: Add, when it is enabled.
    Enter,
    /// People finished retyping the name: the text it holds.
    NameEdited(String),
    /// The body's layout changed: try to bring the target into sight.
    LaidOut,
    /// A link in "Placed in" was clicked.
    Link(LinkTarget),
    /// The pointer entered a link (`Some`) or left it.
    Hover(Option<LinkTarget>),
}

/// What the card tells its owner.
#[derive(Debug, Clone, PartialEq)]
pub enum EditCardOutput {
    /// People did this on the card.
    Action(CardAction),
    /// The setting whose short list is open changed: this one, or none.
    /// Sent whatever opened or closed it (people, a pick from the list,
    /// [`EditCardInput::OpenOptions`]), so the owner's copy follows the
    /// card's.
    OptionsOpen(Option<String>),
    /// A link in "Placed in" was clicked.
    Link(LinkTarget),
    /// The pointer entered a link (`Some`) or left it.
    Hover(Option<LinkTarget>),
}

impl EditCardComponent {
    /// Where the body's widgets report: the card's own input.
    fn report(sender: &ComponentSender<Self>) -> impl Report {
        let input = sender.input_sender().clone();
        move |event| input.emit(EditCardInput::Body(event))
    }

    /// Notes the row of the open short list as the one to bring into
    /// sight, and tries again every few milliseconds until it is laid out
    /// (the adjustment's own changes try too).
    fn scroll_to_open_options(&mut self, sender: &ComponentSender<Self>) {
        let Some(open) = &self.open_options else {
            return;
        };
        let Some(row) = self
            .built
            .options
            .iter()
            .find(|list| list.param.as_str() == open.as_str())
            .and_then(options_row)
        else {
            return;
        };
        self.scroll_target = Some(row);
        let input = sender.input_sender().clone();
        let mut tries = 0_u32;
        glib::timeout_add_local(SCROLL_TRY, move || {
            tries = tries.saturating_add(1);
            // Stops once the card is gone, or after a bound; a try with
            // nothing left to scroll does nothing.
            if tries > SCROLL_TRIES || input.send(EditCardInput::LaidOut).is_err() {
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        });
    }

    /// Keeps the setting whose short list is open, telling the owner when
    /// it changes.
    fn set_open_options(&mut self, open: Option<String>, sender: &ComponentSender<Self>) {
        if self.open_options != open {
            self.open_options.clone_from(&open);
            // Nobody listening is not a mistake.
            let _ = sender.output(EditCardOutput::OptionsOpen(open));
        }
    }

    /// Passes on what people did.
    fn act(sender: &ComponentSender<Self>, action: CardAction) {
        // Nobody listening is not a mistake.
        let _ = sender.output(EditCardOutput::Action(action));
    }
}

/// What tests read of the card's model.
#[cfg(test)]
impl EditCardComponent {
    /// Each setting's entry.
    pub fn entries(&self) -> Vec<(Ident, gtk::Entry)> {
        self.built.entries.clone()
    }

    /// Each setting's short list, with the button that shows it and its
    /// rows by text (the search row by "").
    pub fn option_lists(&self) -> Vec<(Ident, gtk::ToggleButton, ChoiceRows)> {
        self.built
            .options
            .iter()
            .map(|list| (list.param.clone(), list.toggle.clone(), list.rows.clone()))
            .collect()
    }

    /// Each fix's button with its problem and fix index.
    pub fn fix_buttons(&self) -> Vec<((usize, usize), gtk::Button)> {
        self.built.fixes.clone()
    }

    /// The button that puts back the setting a refusal blames, while the
    /// card shows one.
    pub fn put_back_button(&self) -> Option<gtk::Button> {
        self.built.put_back.clone()
    }
}

/// The card component's widgets: its frame.
#[derive(Debug)]
pub struct EditCardWidgets {
    frame: CardFrame,
}

/// What tests read of the card's widgets.
#[cfg(test)]
impl EditCardWidgets {
    /// The Add (Apply) and Cancel buttons.
    pub fn buttons(&self) -> (gtk::Button, gtk::Button) {
        (self.frame.add.clone(), self.frame.cancel.clone())
    }

    /// The name field.
    pub fn name_field(&self) -> gtk::EditableLabel {
        self.frame.name.clone()
    }

    /// The scrolled body.
    pub fn scrolled(&self) -> gtk::ScrolledWindow {
        self.frame.scrolled.clone()
    }
}

impl Component for EditCardComponent {
    type Init = CardSlots;
    type Input = EditCardInput;
    type Output = EditCardOutput;
    type CommandOutput = ();
    type Root = gtk::Box;
    type Widgets = EditCardWidgets;

    fn init_root() -> gtk::Box {
        card_root()
    }

    fn init(
        slots: CardSlots,
        root: gtk::Box,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let frame = fill_card(&root, slots);
        let input = sender.input_sender();
        {
            let input = input.clone();
            frame
                .add
                .connect_clicked(move |_| input.emit(EditCardInput::Add));
        }
        {
            let input = input.clone();
            frame
                .cancel
                .connect_clicked(move |_| input.emit(EditCardInput::Cancel));
        }
        {
            let input = input.clone();
            frame.name.connect_editing_notify(move |label| {
                if !label.is_editing() {
                    input.emit(EditCardInput::NameEdited(label.text().into()));
                }
            });
        }
        {
            let input = input.clone();
            listen_for_keys(&root, move |key| {
                input.emit(match key {
                    CardKey::Escape => EditCardInput::Cancel,
                    CardKey::Enter => EditCardInput::Enter,
                });
            });
        }
        root.connect_map(take_focus);
        {
            let input = input.clone();
            frame
                .scrolled
                .vadjustment()
                .connect_changed(move |_| input.emit(EditCardInput::LaidOut));
        }
        let links = LinkHandlers::default();
        {
            let input = input.clone();
            links.connect_click(move |target| input.emit(EditCardInput::Link(target)));
        }
        {
            let input = input.clone();
            links.connect_hover(move |target| input.emit(EditCardInput::Hover(target)));
        }
        ComponentParts {
            model: EditCardComponent {
                open_options: None,
                shown_name: String::new(),
                add_enabled: false,
                built: Built::default(),
                links,
                scroll_target: None,
            },
            widgets: EditCardWidgets { frame },
        }
    }

    fn update_with_view(
        &mut self,
        widgets: &mut EditCardWidgets,
        message: EditCardInput,
        sender: ComponentSender<Self>,
        root: &gtk::Box,
    ) {
        let frame = &widgets.frame;
        match message {
            EditCardInput::Show(view) => {
                self.shown_name.clone_from(&view.name);
                self.built = build_body(
                    root,
                    frame,
                    &view,
                    self.open_options.as_deref(),
                    &self.links,
                    &Self::report(&sender),
                );
                self.add_enabled = show_add(&frame.add, &view);
                take_focus(root);
                self.scroll_to_open_options(&sender);
            }
            EditCardInput::UpdateOptions(candidate) => {
                let report = Self::report(&sender);
                let rows: Vec<gtk::Button> = self
                    .built
                    .options
                    .iter_mut()
                    .filter_map(|list| refill_choices(list, &candidate, &report))
                    .collect();
                for row in rows {
                    row.grab_focus();
                }
            }
            EditCardInput::OpenOptions(param) => {
                let toggle = self
                    .built
                    .options
                    .iter()
                    .find(|list| list.param.as_str() == param)
                    .map(|list| list.toggle.clone());
                if let Some(toggle) = toggle {
                    self.set_open_options(Some(param), &sender);
                    // Its handler only reports the toggle, which the
                    // model already holds.
                    toggle.set_active(true);
                    self.scroll_to_open_options(&sender);
                }
            }
            EditCardInput::ScrollToOpenOptions => {
                if let Some(target) = &self.scroll_target
                    && scroll_now(root, frame, target)
                {
                    self.scroll_target = None;
                }
            }
            EditCardInput::Body(BodyEvent::Action(action)) => Self::act(&sender, action),
            EditCardInput::Body(BodyEvent::Picked(action)) => {
                self.set_open_options(None, &sender);
                Self::act(&sender, action);
            }
            EditCardInput::Body(BodyEvent::Toggled { param, open }) => {
                if open {
                    self.set_open_options(Some(param), &sender);
                } else if self.open_options.as_deref() == Some(param.as_str()) {
                    self.set_open_options(None, &sender);
                }
            }
            EditCardInput::Add => Self::act(&sender, CardAction::Add),
            EditCardInput::Cancel => Self::act(&sender, CardAction::Cancel),
            EditCardInput::Enter => {
                if self.add_enabled {
                    Self::act(&sender, CardAction::Add);
                }
            }
            EditCardInput::NameEdited(text) => {
                let text = text.trim();
                if text.is_empty() {
                    frame.name.set_text(&self.shown_name);
                } else if text != self.shown_name {
                    self.shown_name = text.to_owned();
                    Self::act(&sender, CardAction::Rename(text.to_owned()));
                }
            }
            EditCardInput::LaidOut => {
                if let Some(target) = &self.scroll_target
                    && scroll_to_target(&frame.scrolled.vadjustment(), &frame.body, target)
                {
                    self.scroll_target = None;
                }
            }
            EditCardInput::Link(target) => {
                let _ = sender.output(EditCardOutput::Link(target));
            }
            EditCardInput::Hover(target) => {
                let _ = sender.output(EditCardOutput::Hover(target));
            }
        }
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

/// How often the card tries to bring an open short list into sight while
/// it is being laid out, and how many times at most.
const SCROLL_TRY: std::time::Duration = std::time::Duration::from_millis(40);
const SCROLL_TRIES: u32 = 50;

/// Brings the target row near the top of what the card shows, once the
/// layout has room to. Returns `true` once it is in sight, when the target
/// is let go, so the user's own scrolling isn't undone.
fn scroll_to_target(adjustment: &gtk::Adjustment, body: &gtk::Box, row: &gtk::Widget) -> bool {
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
        return true;
    }
    // As little as brings the list's foot into sight, never past the
    // row's top.
    let wanted = (bottom - page).min(top - 4.0).max(0.0);
    let reach = adjustment.upper() - page;
    adjustment.set_value(wanted.min(reach));
    reach >= wanted
}

/// The text of the short list's entry whose row holds the keyboard.
fn focused_choice(list: &ChoiceList) -> Option<String> {
    let focus = list.list.root()?.focus()?;
    list.rows
        .iter()
        .find(|(_, row)| focus == *row.upcast_ref::<gtk::Widget>() || focus.is_ancestor(row))
        .map(|(text, _)| text.clone())
}

/// A place param's row: what fills it (each name a link to what it
/// names), or the prompt to point at it.
fn place_row(param: &PlaceParam, words: &PlaceWords, handlers: &LinkHandlers) -> gtk::Box {
    let row = row_box();
    row.append(&key_label(param.name.as_str(), &param.prompt));
    match &words.value {
        Some(value) => {
            if words.links.is_empty() {
                let label = value_label(value);
                label.set_tooltip_text(Some(value));
                row.append(&label);
            } else {
                let shown = handlers.links(&words.links);
                shown.set_tooltip_text(Some(value));
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
    let (section, body) = sections::section(&tr("Fixes added"), &labels.len().to_string(), true);
    let rows = rows_card();
    for label in labels {
        let row = row_box();
        row.append(&gtk::Image::from_icon_name("object-select-symbolic"));
        let words = text(label, &[]);
        words.set_tooltip_text(Some(&tr(
            "Written after the settings below, so it wins over them",
        )));
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
        block.append(&text(
            &fill_in(&tr("note: {note}"), &[("note", &note)]),
            &["stave-edit-note"],
        ));
    }
    if let Some(help) = &diagnostic.help {
        block.append(&text(
            &fill_in(&tr("help: {help}"), &[("help", &help)]),
            &["stave-edit-help"],
        ));
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
        static INSTALLED: std::cell::RefCell<Vec<gdk::Display>> = const { std::cell::RefCell::new(Vec::new()) };
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
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    use super::*;
    use crate::ui::edit_demo::FixtureWords;
    use crate::ui::edit_menu::tests::greyed;
    use crate::ui::edit_menu::{
        Checked, EditMenuComponent, EditMenuInput, EditMenuOutput, MenuEntry, MenuItem,
    };
    use crate::ui::relm4_shim::Bridge;
    use relm4::ComponentController;
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
            add_blocked_reason(door, Some(&fits)).as_deref(),
            Some("Point at what is still missing on the model first")
        );
        let mut unset = drawer.clone();
        unset.settings[0].value = SettingValue::Unset;
        assert!(!can_add(&unset, Some(&fits)));
        assert_eq!(
            add_blocked_reason(drawer, Some(&fixture.verdict)).as_deref(),
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
        use crate::ui::gtk_test::settle;
        let fixture = clothing_cabinet_fixture().unwrap();
        let [drawer, door] = fixture.candidates.as_slice() else {
            panic!("two candidates");
        };

        // The menu: Drawer can be chosen; a door still being checked, or
        // greyed, can't.
        let chosen: Rc<RefCell<Vec<Declaration>>> = Rc::default();
        let menu = {
            let chosen = chosen.clone();
            EditMenuComponent::builder().launch(()).connect_receiver(
                move |_, output| match output {
                    EditMenuOutput::Chosen(declaration) => chosen.borrow_mut().push(declaration),
                    EditMenuOutput::Drawn => {}
                },
            )
        };
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
            menu.sender()
                .send(EditMenuInput::Show {
                    heading: fixture.picked.heading(),
                    items: MenuEntry::list(&items),
                })
                .unwrap();
            settle();
            let rows = menu.model().rows().to_vec();
            let order: Vec<Declaration> =
                rows.iter().map(|(declaration, _)| *declaration).collect();
            assert_eq!(order, [drawer.declaration, door.declaration]);
            for (_, button) in &rows {
                button.emit_clicked();
            }
            settle();
        }
        assert_eq!(*chosen.borrow(), [drawer.declaration, drawer.declaration]);

        // The card, on screen in a window that, like Stave's, takes
        // Escape for itself when nothing inside does.
        let actions: Rc<RefCell<Vec<CardAction>>> = Rc::default();
        let card = {
            let actions = actions.clone();
            EditCardComponent::builder()
                .launch(CardSlots::new())
                .connect_receiver(move |_, output| {
                    if let EditCardOutput::Action(action) = output {
                        actions.borrow_mut().push(action);
                    }
                })
        };
        let root = card.widget().clone();
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
        layout.append(&root);
        window.set_child(Some(&layout));
        // The keyboard starts elsewhere, as it starts in the 3D view.
        gtk::prelude::GtkWindowExt::set_focus(&window, Some(&other));
        window.present();
        let context = glib::MainContext::default();
        let started = std::time::Instant::now();
        while !root.is_mapped() && started.elapsed() < std::time::Duration::from_secs(10) {
            context.iteration(false);
        }
        assert!(root.is_mapped(), "the window never came up");
        // Appearing, the card takes the keyboard.
        assert_eq!(
            gtk::prelude::GtkWindowExt::focus(&window).as_ref(),
            Some(root.upcast_ref::<gtk::Widget>())
        );
        assert_eq!(
            key_controller(&root).widget().as_ref(),
            Some(root.upcast_ref::<gtk::Widget>())
        );

        // Refused: Add is disabled, Enter does nothing, the fix and
        // Escape are reported, and Escape never reaches the window.
        let show = |candidate, verdict| {
            card.sender()
                .send(EditCardInput::Show(Box::new(CardView::new(
                    &CardModel {
                        applied: &[],
                        blame: None,
                        heading: fixture.picked.heading(),
                        candidate,
                        name: "drawer_2",
                        verdict,
                    },
                    &FixtureWords,
                ))))
                .unwrap();
            settle();
        };
        show(drawer, Some(&fixture.verdict));
        let (add, cancel) = card.widgets().buttons();
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
            Some(root.clone().upcast())
        );
        settle();
        let fixes = card.model().fix_buttons();
        assert_eq!(fixes.len(), 1);
        assert_eq!(
            fixes[0].1.label().as_deref(),
            Some("Remove door_l and door_r")
        );
        fixes[0].1.emit_clicked();
        settle();
        assert_eq!(
            press(&window, gdk::Key::Escape),
            Some(root.clone().upcast())
        );
        settle();
        assert_eq!(window_escapes.get(), 0);
        cancel.emit_clicked();
        settle();
        assert_eq!(
            actions.borrow_mut().drain(..).collect::<Vec<_>>(),
            [
                CardAction::Fix { problem: 0, fix: 0 },
                CardAction::Cancel,
                CardAction::Cancel
            ]
        );

        // A setting typed and the name retyped are reported.
        let entries = card.model().entries();
        let (param, entry) = entries
            .iter()
            .find(|(param, _)| param.as_str() == "front_stock")
            .unwrap();
        assert_eq!(entry.text(), "oak_19");
        entry.set_text("walnut_19");
        entry.emit_activate();
        settle();
        let name = card.widgets().name_field();
        assert_eq!(name.text(), "drawer_2");
        name.start_editing();
        name.set_text("tall_drawer");
        name.stop_editing(true);
        settle();
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
            Some(root.upcast_ref::<gtk::Widget>())
        );
        assert_eq!(
            press(&window, gdk::Key::KP_Enter),
            Some(root.clone().upcast())
        );
        settle();
        add.emit_clicked();
        settle();
        assert_eq!(
            actions.borrow_mut().drain(..).collect::<Vec<_>>(),
            [CardAction::Add, CardAction::Add]
        );

        // A setting with no value: shown first, and Add stays disabled.
        let mut unset = drawer.clone();
        unset.settings[1].value = SettingValue::Unset;
        show(&unset, Some(&fits));
        assert!(!add.is_sensitive());
        let entries = card.model().entries();
        assert_eq!(entries[0].0.as_str(), "front_stock");
        assert_eq!(entries[0].1.text(), "");
        // Unbound place param: no verdict, Add disabled.
        show(door, None);
        assert!(!add.is_sensitive());
        assert!(card.model().fix_buttons().is_empty());
        // Without the card's controller, Escape would fall to the window.
        root.remove_controller(&key_controller(&root));
        assert_eq!(
            press(&window, gdk::Key::Escape),
            Some(window.clone().upcast())
        );
        assert_eq!(window_escapes.get(), 1);
        settle();
        assert!(actions.borrow().is_empty(), "nothing reached the card");
        window.destroy();
        drop(card);
        drop(menu);
        settle();
    }

    /// The card's own key controller.
    fn key_controller(root: &gtk::Box) -> gtk::EventControllerKey {
        let controllers = root.observe_controllers();
        (0..controllers.n_items())
            .filter_map(|index| controllers.item(index))
            .find_map(|item| item.downcast::<gtk::EventControllerKey>().ok())
            .expect("the card listens for keys")
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

    /// The card's view of the fixture's Drawer: refused (the fixture's
    /// verdict) or fitting, optionally with a recent value for
    /// `front_stock` (one more row in its short list).
    fn drawer_view(fits: bool, recent: Option<&str>) -> CardView {
        let fixture = clothing_cabinet_fixture().unwrap();
        let mut drawer = fixture.candidates[0].clone();
        if let Some(recent) = recent {
            drawer.settings[1].recent = vec![recent.to_owned()];
        }
        let fitting = Verdict::Fits(Fits::new(Vec::new()).unwrap());
        CardView::new(
            &CardModel {
                applied: &[],
                blame: None,
                heading: fixture.picked.heading(),
                candidate: &drawer,
                name: "drawer_2",
                verdict: Some(if fits { &fitting } else { &fixture.verdict }),
            },
            &FixtureWords,
        )
    }

    /// The card component's Apply and Cancel buttons, found in its frame.
    fn footer_of(root: &gtk::Box) -> (gtk::Button, gtk::Button) {
        let mut stack: Vec<gtk::Widget> = vec![root.clone().upcast()];
        while let Some(widget) = stack.pop() {
            if widget.has_css_class("suggested-action")
                && let Ok(add) = widget.clone().downcast::<gtk::Button>()
            {
                let cancel = add
                    .prev_sibling()
                    .and_then(|widget| widget.downcast::<gtk::Button>().ok())
                    .expect("Cancel sits before Apply");
                return (add, cancel);
            }
            let mut child = widget.first_child();
            while let Some(each) = child {
                child = each.next_sibling();
                stack.push(each);
            }
        }
        panic!("the card has an Apply button");
    }

    /// Fires the card's key controller as a key press would.
    fn press_key(root: &gtk::Box, key: gdk::Key) -> bool {
        key_controller(root).emit_by_name::<bool>(
            "key-pressed",
            &[
                &glib::translate::IntoGlib::into_glib(key),
                &0u32,
                &gdk::ModifierType::empty(),
            ],
        )
    }

    #[test]
    fn a_dropped_card_component_is_freed() {
        crate::ui::gtk_test::on_gtk_thread(card_component_is_freed);
    }

    /// Dropping the bridge shuts the card down when the main loop next
    /// runs: its widget, what its model kept of the last show, its
    /// links' handlers and the callback are all freed.
    fn card_component_is_freed() {
        use crate::ui::gtk_test::settle;
        let card = Bridge::<EditCardComponent>::launch(CardSlots::new());
        card.send(EditCardInput::Show(Box::new(drawer_view(false, None))));
        let token = Rc::new(());
        let held = Rc::clone(&token);
        card.connect(move |_| {
            let _ = &held;
        });
        let callback = Rc::downgrade(&token);
        drop(token);
        settle();
        let root = card.widget().downgrade();
        let entry = card.model().built.entries[0].1.downgrade();
        let fix = card.model().built.fixes[0].1.downgrade();
        let input = card.sender().clone();
        assert!(root.upgrade().is_some(), "the card lives while held");
        drop(card);
        settle();
        assert!(root.upgrade().is_none(), "the card is freed");
        assert!(
            entry.upgrade().is_none() && fix.upgrade().is_none(),
            "what the show drew is freed"
        );
        assert!(
            input.send(EditCardInput::Cancel).is_err(),
            "the component's runtime, and with it the model, is gone"
        );
        assert!(callback.upgrade().is_none(), "the callback is freed");
    }

    #[test]
    fn the_card_components_setters_draw_it_and_report_no_action() {
        crate::ui::gtk_test::on_gtk_thread(card_component_setters);
    }

    /// Show, UpdateOptions and OpenOptions change the model and the
    /// widgets, are only queued until the main loop runs, and never
    /// report an action; opening a list says which is open now, and
    /// asking for a list the card doesn't show does nothing.
    fn card_component_setters() {
        use crate::ui::gtk_test::settle;
        let card = Bridge::<EditCardComponent>::launch(CardSlots::new());
        let heard: Rc<RefCell<Vec<EditCardOutput>>> = Rc::default();
        let log = Rc::clone(&heard);
        card.connect(move |output| log.borrow_mut().push(output));

        card.send(EditCardInput::Show(Box::new(drawer_view(false, None))));
        assert!(card.model().built.entries.is_empty(), "only queued");
        settle();
        let (add, _) = footer_of(card.widget());
        {
            let model = card.model();
            assert_eq!(model.shown_name, "drawer_2");
            assert!(!model.add_enabled && !add.is_sensitive());
            assert_eq!(model.built.fixes.len(), 1);
            assert!(
                !model.built.options.is_empty(),
                "copied settings list their value"
            );
        }
        let front_stock_rows = |card: &Bridge<EditCardComponent>| {
            card.model()
                .built
                .options
                .iter()
                .find(|list| list.param.as_str() == "front_stock")
                .map(|list| list.rows.len())
        };
        let rows = front_stock_rows(&card).expect("front_stock has a short list");

        // A redraw of the short lists alone: one more row for a value the
        // project uses.
        let fixture = clothing_cabinet_fixture().unwrap();
        let mut drawer = fixture.candidates[0].clone();
        drawer.settings[1].recent = vec!["walnut_19".to_owned()];
        card.send(EditCardInput::UpdateOptions(Box::new(drawer)));
        settle();
        assert_eq!(front_stock_rows(&card), Some(rows + 1));

        // Opening a list says so, and keeps it open across a redraw.
        card.send(EditCardInput::OpenOptions("front_stock".to_owned()));
        card.send(EditCardInput::OpenOptions("no_such_setting".to_owned()));
        // Off screen there is no window to lay out: it waits, quietly.
        card.send(EditCardInput::ScrollToOpenOptions);
        settle();
        assert_eq!(card.model().open_options.as_deref(), Some("front_stock"));
        card.send(EditCardInput::Show(Box::new(drawer_view(true, None))));
        settle();
        {
            let model = card.model();
            assert_eq!(model.open_options.as_deref(), Some("front_stock"));
            let list = model
                .built
                .options
                .iter()
                .find(|list| list.param.as_str() == "front_stock")
                .expect("still listed");
            assert!(list.toggle.is_active(), "drawn open again");
            assert!(model.add_enabled && add.is_sensitive());
        }
        assert_eq!(
            *heard.borrow(),
            [EditCardOutput::OptionsOpen(Some("front_stock".to_owned()))],
            "only which list is open; no action"
        );
    }

    #[test]
    fn what_people_do_while_an_action_is_heard_is_queued_not_re_entered() {
        crate::ui::gtk_test::on_gtk_thread(card_input_is_queued);
    }

    /// The old card called the window from inside the button's signal,
    /// with its own state borrowed; a window that redrew the card, or a
    /// key pressed meanwhile, re-entered it. Now a press only queues a
    /// message, the window hears the action after the update has
    /// returned (reading the model there would panic if it had not), and
    /// a redraw, a key and a button pressed from inside the callback are
    /// handled afterwards, in order: Enter finds Add disabled by the
    /// redraw sent before it.
    fn card_input_is_queued() {
        use crate::ui::gtk_test::settle;
        let card = Rc::new(Bridge::<EditCardComponent>::launch(CardSlots::new()));
        card.send(EditCardInput::Show(Box::new(drawer_view(true, None))));
        settle();
        let (add, cancel) = footer_of(card.widget());
        // (what was heard, whether Add was enabled in the model then)
        let heard: Rc<RefCell<Vec<(EditCardOutput, bool)>>> = Rc::default();
        let log = Rc::clone(&heard);
        let weak = Rc::downgrade(&card);
        let inner_cancel = cancel.clone();
        // Only the first action heard sends more (a panic in here would be
        // lost in the main loop, not fail the test, so it must not run
        // again once the redraw has disabled Add).
        let redrawn = Rc::new(Cell::new(false));
        card.connect(move |output| {
            let Some(card) = weak.upgrade() else {
                return;
            };
            let enabled = card.model().add_enabled;
            log.borrow_mut().push((output, enabled));
            if !redrawn.replace(true) {
                card.send(EditCardInput::Show(Box::new(drawer_view(false, None))));
                assert!(press_key(card.widget(), gdk::Key::Return), "the card's key");
                inner_cancel.emit_clicked();
                assert!(card.model().add_enabled, "nothing has run yet");
            }
        });
        add.emit_clicked();
        assert!(heard.borrow().is_empty(), "the press is only queued");
        settle();
        assert_eq!(
            *heard.borrow(),
            [
                (EditCardOutput::Action(CardAction::Add), true),
                (EditCardOutput::Action(CardAction::Cancel), false)
            ],
            "Enter came after the refused redraw, so it added nothing"
        );

        // A retyped name is reported once; an emptied one is put back.
        heard.borrow_mut().clear();
        card.send(EditCardInput::NameEdited("  ".to_owned()));
        card.send(EditCardInput::NameEdited("tall_drawer".to_owned()));
        card.send(EditCardInput::NameEdited("tall_drawer".to_owned()));
        settle();
        assert_eq!(
            *heard.borrow(),
            [(
                EditCardOutput::Action(CardAction::Rename("tall_drawer".to_owned())),
                false
            )]
        );
    }
}
