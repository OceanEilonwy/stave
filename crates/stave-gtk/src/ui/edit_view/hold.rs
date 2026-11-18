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

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, glib};
use stave_app::editing::{EditCommand, parse_length};
use stave_app::hold::{HoldRequest, Measured};
use stave_app::naming::Bound;
use stave_lang::units::Length;
use stave_scene::Target;

use super::super::Window;

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

/// The card that asks how to hold a measured distance.
pub(in crate::ui) struct HoldCard {
    /// The panel, ready to float over the 3D view.
    pub(in crate::ui) root: gtk::Box,
    title: gtk::Label,
    now: gtk::Label,
    at_least: gtk::ToggleButton,
    at_most: gtk::ToggleButton,
    value: gtk::Entry,
    reason: gtk::Entry,
    try_it: gtk::Button,
    cancel: gtk::Button,
}

impl HoldCard {
    /// The card, hidden.
    pub(in crate::ui) fn new() -> HoldCard {
        let body = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(12)
            .css_classes(["stave-inspector"])
            .build();
        let head = gtk::Box::new(gtk::Orientation::Vertical, 2);
        head.append(&label("HOLD A DISTANCE", &["stave-eyebrow"]));
        let title = label("", &["stave-inspector-title"]);
        head.append(&title);
        let now = label("", &["stave-description"]);
        head.append(&now);
        body.append(&head);
        let rows = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .css_classes(["stave-card"])
            .overflow(gtk::Overflow::Hidden)
            .build();
        let bound_row = gtk::Box::builder()
            .spacing(10)
            .css_classes(["stave-card-row"])
            .build();
        bound_row.append(&label("Keep", &["stave-key"]));
        let toggles = gtk::Box::builder().css_classes(["linked"]).build();
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
        bound_row.append(&toggles);
        rows.append(&bound_row);
        let value_row = gtk::Box::builder()
            .spacing(10)
            .css_classes(["stave-card-row"])
            .build();
        value_row.append(&label("Distance", &["stave-key"]));
        let value = gtk::Entry::builder().hexpand(true).build();
        value.update_property(&[gtk::accessible::Property::Label("Distance")]);
        value_row.append(&value);
        rows.append(&value_row);
        let reason_row = gtk::Box::builder()
            .spacing(10)
            .css_classes(["stave-card-row"])
            .build();
        reason_row.append(&label("Why", &["stave-key"]));
        let reason = gtk::Entry::builder()
            .hexpand(true)
            .placeholder_text("room for a finger")
            .build();
        reason.update_property(&[gtk::accessible::Property::Label("Why")]);
        reason_row.append(&reason);
        rows.append(&reason_row);
        body.append(&rows);
        body.append(&label(
            "It becomes a rule (a require) in the text: every later change must keep it.",
            &["stave-footnote"],
        ));
        let cancel = gtk::Button::builder()
            .label("Cancel")
            .tooltip_text("Hold nothing (Escape)")
            .build();
        let try_it = gtk::Button::builder()
            .label("Try it")
            .css_classes(["suggested-action"])
            .tooltip_text("Check the rule on the design (Enter)")
            .build();
        let buttons = gtk::Box::builder()
            .spacing(8)
            .halign(gtk::Align::End)
            .css_classes(["stave-edit-buttons"])
            .build();
        buttons.append(&cancel);
        buttons.append(&try_it);
        let root = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .css_classes(["stave-card-panel", "stave-edit-card", "stave-hold-card"])
            .width_request(340)
            .halign(gtk::Align::End)
            .valign(gtk::Align::Start)
            .accessible_role(gtk::AccessibleRole::Group)
            .overflow(gtk::Overflow::Hidden)
            .visible(false)
            .build();
        root.append(&body);
        root.append(&buttons);
        HoldCard {
            root,
            title,
            now,
            at_least,
            at_most,
            value,
            reason,
            try_it,
            cancel,
        }
    }

    /// Fills the card for two things and the distance between them now
    /// (or why it can't be measured).
    fn show(&self, first: &Measured, second: &Measured, now: &Result<Length, String>) {
        self.title
            .set_label(&format!("{} to {}", first.path(), second.path()));
        match now {
            Ok(length) => {
                self.now
                    .set_label(&format!("It is {} now.", length_text(*length)));
                self.value.set_text(&length_text(*length));
            }
            Err(why) => {
                self.now.set_label(&format!("Can't measure it: {why}"));
                self.value.set_text("");
            }
        }
        self.at_least.set_active(true);
        self.reason.set_text("");
        self.root.set_visible(true);
        self.reason.grab_focus();
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
    /// Puts the hold card over the 3D view and connects it.
    pub(in crate::ui) fn connect_hold(self: &Rc<Self>) {
        let hold = &self.editing.hold;
        self.widgets.design.add_overlay(&hold.card.root);
        self.editing.menu.root.append(&hold.from_space);
        let window = Rc::downgrade(self);
        hold.from_space.connect_clicked(move |_| {
            if let Some(window) = window.upgrade() {
                window.hold_from_space();
            }
        });
        let window = Rc::downgrade(self);
        hold.card.try_it.connect_clicked(move |_| {
            if let Some(window) = window.upgrade() {
                window.try_hold();
            }
        });
        let window = Rc::downgrade(self);
        hold.card.reason.connect_activate(move |_| {
            if let Some(window) = window.upgrade() {
                window.try_hold();
            }
        });
        let window = Rc::downgrade(self);
        hold.card.cancel.connect_clicked(move |_| {
            if let Some(window) = window.upgrade() {
                window.cancel_hold();
            }
        });
        let keys = gtk::EventControllerKey::new();
        let window = Rc::downgrade(self);
        keys.connect_key_pressed(move |_, key, _, _| {
            if key != gdk::Key::Escape {
                return glib::Propagation::Proceed;
            }
            if let Some(window) = window.upgrade() {
                window.cancel_hold();
            }
            glib::Propagation::Stop
        });
        hold.card.root.add_controller(keys);
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
        self.widgets.card.set_visible(false);
        let card = &self.editing.hold.card;
        card.show(&first, &second, &now);
        let rail = self.widgets.files.rail.width();
        card.root.set_margin_end(
            super::super::EDGE
                .saturating_add(rail)
                .saturating_add(super::super::EDGE),
        );
        card.root.set_margin_top(super::super::EDGE);
        self.editing
            .hold
            .step
            .replace(HoldStep::Asking { first, second });
    }

    /// Fills the hold card's fields as the user would (scripts).
    pub(in crate::ui) fn fill_hold(&self, at_most: bool, value: Option<&str>, reason: &str) {
        let card = &self.editing.hold.card;
        if at_most {
            card.at_most.set_active(true);
        } else {
            card.at_least.set_active(true);
        }
        if let Some(value) = value {
            card.value.set_text(value);
        }
        card.reason.set_text(reason);
    }

    /// "Try it": the rule is tried as a change of the design.
    pub(in crate::ui) fn try_hold(self: &Rc<Self>) {
        let (first, second) = match &*self.editing.hold.step.borrow() {
            HoldStep::Asking { first, second } => (first.clone(), second.clone()),
            HoldStep::Idle | HoldStep::Picking { .. } => return,
        };
        let card = &self.editing.hold.card;
        match hold_request(
            &first,
            &second,
            card.at_most.is_active(),
            &card.value.text(),
            &card.reason.text(),
        ) {
            Ok(request) => {
                self.editing.hold.step.replace(HoldStep::Idle);
                card.root.set_visible(false);
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
}
