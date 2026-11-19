//! Hold this (`docs/editing.md`, "Hold this"): with a part selected (or
//! a space's menu open), "Hold a distance…" asks for a second part or
//! space; the next click picks it. The hold card then offers the distance
//! as the design has it now (`Session::measure`), a minimum or a maximum,
//! and a reason; "Try it" sends `EditCommand::Hold`, and the change's card
//! shows the rule with its verdict until it is added.
//!
//! ```text
//! "Hold a distance…" ─▶ picking (the hint asks for the second thing)
//!   click a part or a space ─▶ the hold card: 412mm now · [At least|At most] · [412mm] · [why]
//!     Try it ─▶ EditCommand::Hold ─▶ the change's card ─▶ Add
//! ```
//!
//! The hold card is a relm4 component ([`HoldCardComponent`]) following
//! `docs/relm4-migration.md`; [`HoldCard`] keeps the older API in front of
//! it while the window (`impl Window`) is old code. Where a hold stands
//! ([`HoldStep`]) is the window's.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, glib};
use relm4::{Component, ComponentParts, ComponentSender};
use stave_app::editing::{EditCommand, parse_length};
use stave_app::hold::{HoldRequest, Measured};
use stave_app::naming::Bound;
use stave_lang::units::Length;
use stave_scene::Target;

use super::super::Window;
use super::super::relm4_shim::Bridge;
use super::super::sections;

/// Where a hold stands.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(in crate::ui) enum HoldStep {
    /// No hold is being set up.
    #[default]
    Idle,
    /// The first thing is chosen; the next click picks the second.
    Picking {
        /// The first thing.
        first: Measured,
    },
    /// Both are chosen; the hold card asks for the bound and the reason.
    Asking {
        /// The first thing.
        first: Measured,
        /// The second.
        second: Measured,
        /// What the window last filled the card's fields with (the
        /// distance now, then a script's): what a script's "Try it"
        /// tries. People's "Try it" sends what they typed.
        fields: HoldFields,
    },
}

/// The value a hold's field starts at: the distance as it is now, in the
/// project's words (`412mm`, `412.5mm`).
pub(in crate::ui) fn length_text(length: Length) -> String {
    let nm = length.nm();
    let whole = nm / stave_lang::units::NM_PER_MM;
    let rest = (nm % stave_lang::units::NM_PER_MM).abs();
    if rest == 0 {
        return format!("{whole}mm");
    }
    let mut fraction = format!("{rest:06}");
    while fraction.ends_with('0') {
        fraction.pop();
    }
    let sign = if nm < 0 && whole == 0 { "-" } else { "" };
    format!("{sign}{whole}.{fraction}mm")
}

/// The request the hold card's fields make, or what is wrong with them,
/// in words.
pub(in crate::ui) fn hold_request(
    first: &Measured,
    second: &Measured,
    at_most: bool,
    value: &str,
    reason: &str,
) -> Result<HoldRequest, String> {
    let value = parse_length(value.trim())
        .map_err(|_| format!("'{}' isn't a length: write it as 412mm", value.trim()))?;
    let reason = reason.trim();
    if reason.is_empty() {
        return Err("Say why it must be held: the reason is the rule's message".to_owned());
    }
    Ok(HoldRequest {
        first: first.clone(),
        second: second.clone(),
        bound: if at_most { Bound::Max } else { Bound::Min },
        value,
        reason: reason.to_owned(),
    })
}

/// What the hold card's fields hold: the bound, the length and the reason.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(in crate::ui) struct HoldFields {
    /// "At most" (else "At least").
    pub(in crate::ui) at_most: bool,
    /// The length, as typed.
    pub(in crate::ui) value: String,
    /// Why it must be held.
    pub(in crate::ui) reason: String,
}

/// The card that asks how to hold a measured distance, as a relm4
/// component. What people type stays in its fields; it is read when they
/// press "Try it" (or Enter in the reason) and sent with
/// [`HoldCardOutput::TryIt`]. Its owner shows and hides the card.
#[derive(Debug, Default)]
pub(in crate::ui) struct HoldCardComponent;

/// The hold card's widgets.
#[derive(Debug)]
pub(in crate::ui) struct HoldCardWidgets {
    title: gtk::Label,
    now: gtk::Label,
    at_least: gtk::ToggleButton,
    at_most: gtk::ToggleButton,
    value: gtk::Entry,
    reason: gtk::Entry,
}

impl HoldCardWidgets {
    /// What the fields hold now.
    fn fields(&self) -> HoldFields {
        HoldFields {
            at_most: self.at_most.is_active(),
            value: self.value.text().to_string(),
            reason: self.reason.text().to_string(),
        }
    }
}

/// What the hold card is told.
#[derive(Debug)]
pub(in crate::ui) enum HoldCardInput {
    /// Ask about the distance between two things: their names
    /// ("a to b"), and the distance now or why it can't be measured.
    Show {
        /// "divider.panel to case.top".
        title: String,
        /// The distance now, or why it can't be measured.
        now: Result<Length, String>,
    },
    /// Fill the fields as people would (scripts).
    Fill(HoldFields),
    /// People pressed "Try it", or Enter in the reason.
    Try,
    /// People pressed Cancel, or Escape.
    Cancel,
}

/// What the hold card tells its owner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::ui) enum HoldCardOutput {
    /// Try the rule the fields make.
    TryIt(HoldFields),
    /// Hold nothing.
    Cancel,
}

impl Component for HoldCardComponent {
    type Init = ();
    type Input = HoldCardInput;
    type Output = HoldCardOutput;
    type CommandOutput = ();
    type Root = gtk::Box;
    type Widgets = HoldCardWidgets;

    fn init_root() -> gtk::Box {
        gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .css_classes(["stave-edit-card", "stave-hold-card"])
            .hexpand(true)
            .vexpand(true)
            .accessible_role(gtk::AccessibleRole::Group)
            .visible(false)
            .build()
    }

    fn init(_: (), root: gtk::Box, sender: ComponentSender<Self>) -> ComponentParts<Self> {
        let body = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(0)
            .css_classes(["stave-edit-body"])
            .build();
        // The header: the two things, in a monospace field; what it is on
        // the right.
        let title = sections::name_field("");
        let kind = label("hold a distance", &["stave-edit-origin"]);
        kind.set_wrap(false);
        body.append(&sections::header(None, &title, Some(kind.upcast_ref())));
        let (section, rows) = sections::section("Hold", "", true);
        let now = label("", &["stave-row-note"]);
        rows.append(&now);
        let toggles = gtk::Box::builder()
            .css_classes(["linked"])
            .halign(gtk::Align::Start)
            .build();
        let at_least = gtk::ToggleButton::builder()
            .label("At least")
            .active(true)
            .build();
        let at_most = gtk::ToggleButton::builder()
            .label("At most")
            .group(&at_least)
            .build();
        toggles.append(&at_least);
        toggles.append(&at_most);
        rows.append(&sections::row("Keep", &toggles));
        let value = gtk::Entry::builder().hexpand(true).build();
        value.update_property(&[gtk::accessible::Property::Label("Distance")]);
        rows.append(&sections::row("Distance", &value));
        let reason = gtk::Entry::builder()
            .hexpand(true)
            .placeholder_text("room for a finger")
            .build();
        reason.update_property(&[gtk::accessible::Property::Label("Why")]);
        rows.append(&sections::row("Why", &reason));
        rows.append(&label(
            "It becomes a rule (a require) in the text: every later change must keep it.",
            &["stave-footnote"],
        ));
        body.append(&section);
        let (buttons, cancel, try_it) = sections::footer_buttons();
        buttons.add_css_class("stave-edit-buttons");
        cancel.set_tooltip_text(Some("Hold nothing (Esc)"));
        // Not yet the commit: it checks the rule, and the change's card
        // then offers Apply.
        sections::set_button_words(&try_it, "Try it");
        try_it.set_tooltip_text(Some("Check the rule on the design (Enter)"));
        let scrolled = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .child(&body)
            .build();
        let foot = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .css_classes(["stave-edit-foot"])
            .build();
        foot.append(&buttons);
        root.append(&scrolled);
        root.append(&foot);
        let input = sender.input_sender().clone();
        try_it.connect_clicked(move |_| input.emit(HoldCardInput::Try));
        let input = sender.input_sender().clone();
        reason.connect_activate(move |_| input.emit(HoldCardInput::Try));
        let input = sender.input_sender().clone();
        cancel.connect_clicked(move |_| input.emit(HoldCardInput::Cancel));
        let keys = gtk::EventControllerKey::new();
        let input = sender.input_sender().clone();
        keys.connect_key_pressed(move |_, key, _, _| {
            if key != gdk::Key::Escape {
                return glib::Propagation::Proceed;
            }
            input.emit(HoldCardInput::Cancel);
            glib::Propagation::Stop
        });
        root.add_controller(keys);
        ComponentParts {
            model: HoldCardComponent,
            widgets: HoldCardWidgets {
                title,
                now,
                at_least,
                at_most,
                value,
                reason,
            },
        }
    }

    fn update_with_view(
        &mut self,
        widgets: &mut HoldCardWidgets,
        message: HoldCardInput,
        sender: ComponentSender<Self>,
        _root: &gtk::Box,
    ) {
        let output = match message {
            HoldCardInput::Show { title, now } => {
                widgets.title.set_label(&title);
                match now {
                    Ok(length) => {
                        widgets
                            .now
                            .set_label(&format!("It is {} now.", length_text(length)));
                        widgets.value.set_text(&length_text(length));
                    }
                    Err(why) => {
                        widgets.now.set_label(&format!("Can't measure it: {why}"));
                        widgets.value.set_text("");
                    }
                }
                widgets.at_least.set_active(true);
                widgets.reason.set_text("");
                widgets.reason.grab_focus();
                None
            }
            HoldCardInput::Fill(fields) => {
                if fields.at_most {
                    widgets.at_most.set_active(true);
                } else {
                    widgets.at_least.set_active(true);
                }
                widgets.value.set_text(&fields.value);
                widgets.reason.set_text(&fields.reason);
                None
            }
            HoldCardInput::Try => Some(HoldCardOutput::TryIt(widgets.fields())),
            HoldCardInput::Cancel => Some(HoldCardOutput::Cancel),
        };
        if let Some(output) = output {
            // Nobody listening is not a mistake.
            let _ = sender.output(output);
        }
    }
}

/// The hold card, behind the API the window still uses: it owns the
/// component and passes what it is told on as [`HoldCardInput`] messages.
pub(in crate::ui) struct HoldCard {
    /// The card, in the Edit panel; shown by [`HoldCard::show`], hidden by
    /// its owner.
    pub(in crate::ui) root: gtk::Box,
    bridge: Bridge<HoldCardComponent>,
}

impl HoldCard {
    /// The card, hidden.
    pub(in crate::ui) fn new() -> HoldCard {
        let bridge = Bridge::<HoldCardComponent>::launch(());
        HoldCard {
            root: bridge.widget().clone(),
            bridge,
        }
    }

    /// Shows the card at once for two things and the distance between
    /// them now (or why it can't be measured); its fields are filled in,
    /// and the keyboard put in the reason, when the main loop next runs.
    fn show(&self, first: &Measured, second: &Measured, now: &Result<Length, String>) {
        self.root.set_visible(true);
        self.bridge.send(HoldCardInput::Show {
            title: format!("{} to {}", first.path(), second.path()),
            now: now.clone(),
        });
    }

    /// Fills the fields as people would, when the main loop next runs.
    fn fill(&self, fields: HoldFields) {
        self.bridge.send(HoldCardInput::Fill(fields));
    }

    /// Calls `callback` with what people do on the card.
    fn connect(&self, callback: impl Fn(HoldCardOutput) + 'static) {
        self.bridge.connect(callback);
    }
}

fn label(words: &str, classes: &[&str]) -> gtk::Label {
    gtk::Label::builder()
        .label(words)
        .xalign(0.0)
        .wrap(true)
        .css_classes(classes.to_vec())
        .build()
}

/// The hold being set up, and its card.
pub(in crate::ui) struct Hold {
    /// The card.
    pub(in crate::ui) card: HoldCard,
    /// Where it stands.
    pub(in crate::ui) step: RefCell<HoldStep>,
    /// On a space's menu: hold a distance from that space.
    pub(in crate::ui) from_space: gtk::Button,
}

impl Hold {
    /// Nothing being held.
    pub(in crate::ui) fn new() -> Hold {
        Hold {
            card: HoldCard::new(),
            step: RefCell::new(HoldStep::Idle),
            from_space: gtk::Button::builder()
                .label("Hold a distance from this space…")
                .css_classes(["flat", "stave-hold-from-space"])
                .tooltip_text("Pick a part or another space and keep the distance to it")
                .build(),
        }
    }
}

impl Window {
    /// Connects the hold card (in the Edit panel) and the space menu's
    /// button.
    pub(in crate::ui) fn connect_hold(self: &Rc<Self>) {
        let hold = &self.editing.hold;
        self.editing.menu.root.append(&hold.from_space);
        let window = Rc::downgrade(self);
        hold.from_space.connect_clicked(move |_| {
            if let Some(window) = window.upgrade() {
                window.hold_from_space();
            }
        });
        let window = Rc::downgrade(self);
        hold.card.connect(move |output| {
            let Some(window) = window.upgrade() else {
                return;
            };
            match output {
                HoldCardOutput::TryIt(fields) => window.try_hold_with(fields),
                HoldCardOutput::Cancel => {
                    window.cancel_hold();
                }
            }
        });
    }

    /// "Hold a distance…" on a part's card: the next click picks the
    /// second thing.
    pub(in crate::ui) fn hold_from_selected(self: &Rc<Self>) {
        let first = {
            let session = self.session.borrow();
            let Some((design, _)) = session.drawable() else {
                return;
            };
            let Some(selection) = session.selection() else {
                self.edit_toast("Select a part first, then hold a distance from it");
                return;
            };
            super::item_tools::measured_of(design, selection)
        };
        match first {
            Ok(first) => self.start_hold(first),
            Err(why) => self.edit_toast(&format!("Can't hold a distance from that: {why}")),
        }
    }

    /// "Hold a distance from this space…" on a space's menu.
    pub(in crate::ui) fn hold_from_space(self: &Rc<Self>) {
        let region = {
            let session = self.session.borrow();
            let edit = session.edit();
            let (_, graph) = match session.drawable() {
                Some(drawn) => drawn,
                None => return,
            };
            match edit.map(stave_app::editing::Edit::picked) {
                Some(stave_app::edit::Picked::Region(region)) => {
                    graph.region(*region).map(|region| region.name.clone())
                }
                _ => None,
            }
        };
        if let Some(region) = region {
            self.cancel_edit();
            self.start_hold(Measured::Item(region));
        }
    }

    /// Begins a hold from `first`: the hint asks for the second thing.
    pub(in crate::ui) fn start_hold(self: &Rc<Self>, first: Measured) {
        if !self.gestures_allowed() {
            self.edit_toast(&self.parse_block_line().unwrap_or_default());
            return;
        }
        if self.edit_open() {
            self.cancel_edit();
        }
        self.show_hint(&format!(
            "Click the part or space to measure to from {} · Esc stops",
            first.path()
        ));
        self.editing.hold.step.replace(HoldStep::Picking { first });
        self.editing.hold.card.root.set_visible(false);
        self.update_edit_panel();
    }

    /// A click while a hold waits for its second thing: a part or a space
    /// under the pointer is it. `true` when the click was used here.
    pub(in crate::ui) fn hold_click(self: &Rc<Self>, x: f64, y: f64) -> bool {
        let first = match &*self.editing.hold.step.borrow() {
            HoldStep::Picking { first } => first.clone(),
            HoldStep::Idle | HoldStep::Asking { .. } => return false,
        };
        let (target, _) = self.pointed_at(x, y);
        let second = {
            let session = self.session.borrow();
            let Some((design, graph)) = session.drawable() else {
                return true;
            };
            match target {
                Some(Target::Region(hit)) => graph
                    .region(hit.region)
                    .map(|region| Measured::Item(region.name.clone())),
                Some(Target::Node(hit)) => hit
                    .part_face()
                    .and_then(|face| design.part(face.part))
                    .map(|part| Measured::Item(part.name.clone())),
                None => None,
            }
        };
        match second {
            Some(second) if second != first => self.ask_hold(first, second),
            Some(_) => self.edit_toast("Pick something else: that is what it measures from"),
            None => self.edit_toast("Click a part or a space to measure to"),
        }
        true
    }

    /// Both things are chosen: the hold card asks how to hold the distance.
    pub(in crate::ui) fn ask_hold(self: &Rc<Self>, first: Measured, second: Measured) {
        let now = self.session.borrow().measure(&first, &second);
        self.show_hint("");
        self.editing.hold.card.show(&first, &second, &now);
        self.update_edit_panel();
        let fields = HoldFields {
            at_most: false,
            value: now
                .as_ref()
                .map(|length| length_text(*length))
                .unwrap_or_default(),
            reason: String::new(),
        };
        self.editing.hold.step.replace(HoldStep::Asking {
            first,
            second,
            fields,
        });
    }

    /// Fills the hold card's fields as the user would (scripts).
    pub(in crate::ui) fn fill_hold(&self, at_most: bool, value: Option<&str>, reason: &str) {
        let fields = {
            let mut step = self.editing.hold.step.borrow_mut();
            let HoldStep::Asking { fields, .. } = &mut *step else {
                return;
            };
            fields.at_most = at_most;
            if let Some(value) = value {
                value.clone_into(&mut fields.value);
            }
            reason.clone_into(&mut fields.reason);
            fields.clone()
        };
        self.editing.hold.card.fill(fields);
    }

    /// "Try it", as a script presses it: the rule the window last filled
    /// the card with is tried as a change of the design.
    pub(in crate::ui) fn try_hold(self: &Rc<Self>) {
        let fields = match &*self.editing.hold.step.borrow() {
            HoldStep::Asking { fields, .. } => fields.clone(),
            HoldStep::Idle | HoldStep::Picking { .. } => return,
        };
        self.try_hold_with(fields);
    }

    /// "Try it": the rule the card's fields make is tried as a change of
    /// the design.
    fn try_hold_with(self: &Rc<Self>, fields: HoldFields) {
        let (first, second) = match &*self.editing.hold.step.borrow() {
            HoldStep::Asking { first, second, .. } => (first.clone(), second.clone()),
            HoldStep::Idle | HoldStep::Picking { .. } => return,
        };
        match hold_request(
            &first,
            &second,
            fields.at_most,
            &fields.value,
            &fields.reason,
        ) {
            Ok(request) => {
                self.editing.hold.step.replace(HoldStep::Idle);
                self.editing.hold.card.root.set_visible(false);
                self.update_edit_panel();
                self.start_change(EditCommand::Hold(request));
            }
            Err(words) => self.edit_toast(&words),
        }
    }

    /// Stops a hold being set up. `true` when there was one.
    pub(in crate::ui) fn cancel_hold(self: &Rc<Self>) -> bool {
        let was = self.editing.hold.step.replace(HoldStep::Idle) != HoldStep::Idle;
        if was {
            self.editing.hold.card.root.set_visible(false);
            self.update_edit_panel();
            self.show_hint("");
            self.show_card();
        }
        was
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_length_is_written_as_the_project_writes_it() {
        assert_eq!(length_text(Length::from_mm(412)), "412mm");
        assert_eq!(length_text(Length::from_nm(412_500_000)), "412.5mm");
        assert_eq!(length_text(Length::from_nm(12_250_000)), "12.25mm");
    }

    #[test]
    fn a_hold_needs_a_length_and_a_reason() {
        let first = Measured::Item("divider.panel".to_owned());
        let second = Measured::Item("case.top".to_owned());
        let request =
            hold_request(&first, &second, false, "412mm", "room for the drawers").unwrap();
        assert_eq!(request.bound, Bound::Min);
        assert_eq!(request.value, Length::from_mm(412));
        assert_eq!(request.reason, "room for the drawers");
        let at_most = hold_request(&first, &second, true, " 400mm ", "x").unwrap();
        assert_eq!(at_most.bound, Bound::Max);
        assert!(
            hold_request(&first, &second, false, "oak", "why")
                .unwrap_err()
                .contains("isn't a length")
        );
        assert!(
            hold_request(&first, &second, false, "4mm", "  ")
                .unwrap_err()
                .starts_with("Say why")
        );
    }

    use std::cell::RefCell;

    use crate::ui::gtk_test::{on_gtk_thread, settle};

    /// Every widget of type `T` under `widget`, in order.
    fn descendants<T: IsA<gtk::Widget>>(widget: &gtk::Widget) -> Vec<T> {
        let mut found = Vec::new();
        let mut stack = vec![widget.clone()];
        while let Some(widget) = stack.pop() {
            if let Ok(typed) = widget.clone().downcast::<T>() {
                found.push(typed);
            }
            let mut child = widget.last_child();
            while let Some(next) = child {
                child = next.prev_sibling();
                stack.push(next);
            }
        }
        found
    }

    fn show_divider(card: &HoldCard) {
        card.show(
            &Measured::Item("divider.panel".to_owned()),
            &Measured::Item("case.top".to_owned()),
            &Ok(Length::from_mm(412)),
        );
    }

    #[test]
    fn a_dropped_hold_card_is_freed() {
        on_gtk_thread(hold_card_is_freed);
    }

    fn hold_card_is_freed() {
        let card = HoldCard::new();
        let token = Rc::new(());
        let held = Rc::clone(&token);
        card.connect(move |_| {
            let _ = &held;
        });
        let callback = Rc::downgrade(&token);
        drop(token);
        show_divider(&card);
        assert!(card.root.is_visible(), "shown at once");
        settle();
        let root = card.root.downgrade();
        let input = card.bridge.sender().clone();
        drop(card);
        settle();
        assert!(root.upgrade().is_none(), "the card is freed");
        assert!(
            input.send(HoldCardInput::Cancel).is_err(),
            "the component's runtime, and with it the model, is gone"
        );
        assert!(callback.upgrade().is_none(), "the callback is freed");
    }

    #[test]
    fn filling_the_hold_card_reports_nothing_and_try_it_sends_the_fields() {
        on_gtk_thread(hold_card_fields);
    }

    /// Showing and filling it report nothing; "Try it" sends what the
    /// fields hold. A Cancel pressed from inside the callback is queued
    /// behind it, not run under it.
    fn hold_card_fields() {
        let card = Rc::new(HoldCard::new());
        let heard: Rc<RefCell<Vec<HoldCardOutput>>> = Rc::default();
        let log = Rc::clone(&heard);
        let weak = Rc::downgrade(&card);
        card.connect(move |output| {
            let Some(card) = weak.upgrade() else {
                return;
            };
            let first = log.borrow().is_empty();
            log.borrow_mut().push(output);
            if first {
                let buttons = descendants::<gtk::Button>(card.root.upcast_ref());
                if let Some(cancel) = buttons
                    .iter()
                    .find(|button| button.has_css_class("stave-footer-button"))
                {
                    cancel.emit_clicked();
                }
                assert_eq!(log.borrow().len(), 1, "nothing ran under the callback");
            }
        });
        show_divider(&card);
        card.fill(HoldFields {
            at_most: true,
            value: "400mm".to_owned(),
            reason: "knee room".to_owned(),
        });
        settle();
        assert!(
            heard.borrow().is_empty(),
            "showing and filling report nothing"
        );
        let entries = descendants::<gtk::Entry>(card.root.upcast_ref());
        let texts: Vec<String> = entries
            .iter()
            .map(|entry| entry.text().to_string())
            .collect();
        assert_eq!(texts, ["400mm", "knee room"]);
        // Enter in the reason is "Try it".
        let Some(reason) = entries.get(1) else {
            panic!("the card has a reason");
        };
        reason.emit_activate();
        assert!(heard.borrow().is_empty(), "the press is only queued");
        settle();
        assert_eq!(
            *heard.borrow(),
            vec![
                HoldCardOutput::TryIt(HoldFields {
                    at_most: true,
                    value: "400mm".to_owned(),
                    reason: "knee room".to_owned(),
                }),
                HoldCardOutput::Cancel,
            ]
        );
    }
}
