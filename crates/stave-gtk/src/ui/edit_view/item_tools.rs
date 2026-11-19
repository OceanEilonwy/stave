//! Changes of a selected item, on its card (`docs/gui.md`, "Changing
//! what is there"): its name, retyped in place (a rename everywhere it is
//! used); how many there are, with a stepper (a repeat filling its space);
//! "And the other side" when the session offers a mirror image; holding a
//! distance from it; removing it. Each control and each verb of the
//! finder calls the same method of the window, which sends the session's
//! command: one path for the mouse and the keyboard.
//!
//! ```text
//! CHANGE IT
//! Name        [drawer          ]
//! How many    [- 2 +]  stacked [up ▾]
//! And the other side · adds door_right   (o )
//! [Hold a distance…]  [Remove…]
//! ```

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use stave_app::change::ChangeRequest;
use stave_app::editing::{EditCommand, parse_name};
use stave_app::hold::Measured;
use stave_app::item_offer::{ItemOffer, MirrorGesture};
pub(in crate::ui) use stave_app::item_offer::{piece_count, selected_item};
use stave_app::naming::RepeatCount;
use stave_app::session::Selection;
use stave_lang::placement::Axis;
use stave_model::solved::SolvedDesign;

use super::super::Window;

/// The axes a repeat can split along, in the order the list shows them,
/// with the word for how the pieces stand.
pub(in crate::ui) const AXES: [(Axis, &str); 3] = [
    (Axis::Up, "stacked up"),
    (Axis::Across, "side by side"),
    (Axis::Deep, "front to back"),
];

/// The most pieces the stepper and the drag handle go to.
pub(in crate::ui) const MOST: usize = 12;

/// The thing a hold measures from, for a selection: a part by its whole
/// name; `None` for an assembly (a distance is measured between parts or
/// spaces), with the reason.
pub(in crate::ui) fn measured_of(
    design: &SolvedDesign,
    selection: &Selection,
) -> Result<Measured, &'static str> {
    match selection {
        Selection::Part(part) => design
            .part(*part)
            .map(|part| Measured::Item(part.name.clone()))
            .ok_or("that part is gone"),
        Selection::Assembly(_) => Err("pick one of its parts to measure from"),
        Selection::Hardware(_)
        | Selection::Joint { .. }
        | Selection::Member(_)
        | Selection::Location(_)
        | Selection::Problem(_) => Err("a distance is measured from a part or a space"),
    }
}

/// The repeat a stepper's count and axis ask for on a candidate: `None`
/// for one piece.
pub(in crate::ui) fn candidate_repeat(
    count: usize,
    axis: Axis,
) -> Option<stave_app::candidates::Repeat> {
    RepeatCount::new(count).map(|count| stave_app::candidates::Repeat { count, axis })
}

/// A count stepper with the axis to split along: on a candidate's card
/// and on a selected item's.
pub(in crate::ui) struct RepeatRow {
    /// The row.
    pub(in crate::ui) root: gtk::Box,
    count: gtk::SpinButton,
    axis: gtk::DropDown,
    /// Set while the row is filled from the session, so it reports nothing.
    filling: Rc<Cell<bool>>,
}

impl RepeatRow {
    /// A row starting at one piece, stacked up; the smallest count it
    /// offers is `least`.
    pub(in crate::ui) fn new(least: usize) -> RepeatRow {
        let root = gtk::Box::builder()
            .spacing(8)
            .css_classes(["stave-card-row", "stave-repeat-row"])
            .build();
        root.append(
            &gtk::Label::builder()
                .label("How many")
                .xalign(0.0)
                .css_classes(["stave-key"])
                .build(),
        );
        let adjustment = gtk::Adjustment::new(
            1.0,
            f64::from(u32::try_from(least).unwrap_or(1)),
            f64::from(u32::try_from(MOST).unwrap_or(12)),
            1.0,
            1.0,
            0.0,
        );
        let count = gtk::SpinButton::builder()
            .adjustment(&adjustment)
            .digits(0)
            .numeric(true)
            .tooltip_text("How many pieces fill the space, each the same size")
            .build();
        count.update_property(&[gtk::accessible::Property::Label("How many")]);
        root.append(&count);
        let words: Vec<&str> = AXES.iter().map(|(_, words)| *words).collect();
        let axis = gtk::DropDown::from_strings(&words);
        axis.set_tooltip_text(Some("Which way the pieces are split"));
        axis.update_property(&[gtk::accessible::Property::Label("Which way")]);
        root.append(&axis);
        RepeatRow {
            root,
            count,
            axis,
            filling: Rc::new(Cell::new(false)),
        }
    }

    /// Shows a count and an axis without reporting them; the axis list
    /// shows only while the count can still choose it.
    pub(in crate::ui) fn show(&self, count: usize, axis: Axis, axis_shown: bool) {
        self.filling.set(true);
        self.count
            .set_value(f64::from(u32::try_from(count).unwrap_or(1)));
        if let Some(index) = AXES.iter().position(|(each, _)| *each == axis) {
            self.axis.set_selected(u32::try_from(index).unwrap_or(0));
        }
        self.axis.set_visible(axis_shown);
        self.filling.set(false);
    }

    /// The count shown.
    pub(in crate::ui) fn count(&self) -> usize {
        usize::try_from(self.count.value_as_int()).unwrap_or(1)
    }

    /// The axis chosen.
    pub(in crate::ui) fn axis(&self) -> Axis {
        usize::try_from(self.axis.selected())
            .ok()
            .and_then(|index| AXES.get(index))
            .map_or(Axis::Up, |(axis, _)| *axis)
    }

    /// Calls `callback` with the count and axis when the user changes
    /// either.
    pub(in crate::ui) fn connect_changed(&self, callback: impl Fn(usize, Axis) + 'static) {
        let callback = Rc::new(callback);
        {
            let callback = callback.clone();
            let filling = self.filling.clone();
            let axis = self.axis.clone();
            self.count.connect_value_changed(move |count| {
                if filling.get() {
                    return;
                }
                let chosen = usize::try_from(axis.selected())
                    .ok()
                    .and_then(|index| AXES.get(index))
                    .map_or(Axis::Up, |(axis, _)| *axis);
                callback(usize::try_from(count.value_as_int()).unwrap_or(1), chosen);
            });
        }
        let filling = self.filling.clone();
        let count = self.count.clone();
        self.axis.connect_selected_notify(move |axis| {
            if filling.get() {
                return;
            }
            let chosen = usize::try_from(axis.selected())
                .ok()
                .and_then(|index| AXES.get(index))
                .map_or(Axis::Up, |(axis, _)| *axis);
            callback(usize::try_from(count.value_as_int()).unwrap_or(1), chosen);
        });
    }
}

/// The changes a selected item's card offers.
pub(in crate::ui) struct ItemTools {
    /// The panel, at the bottom of the part's card.
    pub(in crate::ui) root: gtk::Box,
    title: gtk::Label,
    name: gtk::EditableLabel,
    repeat: RepeatRow,
    mirror_row: gtk::Box,
    mirror_words: gtk::Label,
    mirror: gtk::Switch,
    hold: gtk::Button,
    remove: gtk::Button,
    /// The item shown, and the result it was worked out for.
    shown: RefCell<Option<Shown>>,
    filling: Rc<Cell<bool>>,
}

/// What the panel was last filled with.
#[derive(Debug, Clone, PartialEq)]
struct Shown {
    item: String,
    /// The design it was worked out from, by address: a new result works
    /// the offer out again.
    design: usize,
    /// What the session offers for it.
    offer: Option<ItemOffer>,
}

impl ItemTools {
    /// The panel, hidden.
    pub(in crate::ui) fn new() -> ItemTools {
        let root = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(8)
            .css_classes(["stave-item-tools"])
            .margin_start(16)
            .margin_end(16)
            .margin_bottom(16)
            .visible(false)
            .build();
        let title = gtk::Label::builder()
            .label("CHANGE IT")
            .xalign(0.0)
            .css_classes(["stave-section"])
            .build();
        root.append(&title);
        let rows = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .css_classes(["stave-card"])
            .overflow(gtk::Overflow::Hidden)
            .build();
        let name_row = gtk::Box::builder()
            .spacing(10)
            .css_classes(["stave-card-row"])
            .build();
        name_row.append(
            &gtk::Label::builder()
                .label("Name")
                .xalign(0.0)
                .css_classes(["stave-key"])
                .build(),
        );
        let name = gtk::EditableLabel::builder()
            .hexpand(true)
            .css_classes(["stave-edit-name"])
            .tooltip_text("Click to retype the name: it changes everywhere it is used")
            .build();
        name.update_property(&[gtk::accessible::Property::Label("Name")]);
        name_row.append(&name);
        rows.append(&name_row);
        let repeat = RepeatRow::new(1);
        rows.append(&repeat.root);
        let mirror_row = gtk::Box::builder()
            .spacing(10)
            .css_classes(["stave-card-row"])
            .build();
        let mirror_words = gtk::Label::builder()
            .xalign(0.0)
            .hexpand(true)
            .wrap(true)
            .build();
        let mirror = gtk::Switch::builder()
            .valign(gtk::Align::Center)
            .tooltip_text("Add its mirror image on the other side")
            .build();
        mirror.update_property(&[gtk::accessible::Property::Label("And the other side")]);
        mirror_row.append(&mirror_words);
        mirror_row.append(&mirror);
        rows.append(&mirror_row);
        let buttons = gtk::Box::builder()
            .spacing(8)
            .css_classes(["stave-card-row", "stave-item-buttons"])
            .build();
        let hold = gtk::Button::builder()
            .label("Hold a distance…")
            .css_classes(["flat"])
            .hexpand(true)
            .tooltip_text("Pick a second part or space and keep the distance to it")
            .build();
        let remove = gtk::Button::builder()
            .label("Remove…")
            .css_classes(["flat", "error"])
            .hexpand(true)
            .tooltip_text("Take it out of the design (Delete)")
            .build();
        buttons.append(&hold);
        buttons.append(&remove);
        rows.append(&buttons);
        root.append(&rows);
        ItemTools {
            root,
            title,
            name,
            repeat,
            mirror_row,
            mirror_words,
            mirror,
            hold,
            remove,
            shown: RefCell::new(None),
            filling: Rc::new(Cell::new(false)),
        }
    }
}

impl ItemTools {
    /// Its count stepper.
    pub(in crate::ui) fn repeat_row(&self) -> &RepeatRow {
        &self.repeat
    }
}

impl Window {
    /// Puts the item's changes in the Details panel, under the
    /// selection's account, and connects them.
    pub(in crate::ui) fn connect_item_tools(self: &Rc<Self>) {
        let tools = &self.editing.tools;
        self.widgets.details.append(&tools.root);
        let window = Rc::downgrade(self);
        let filling = tools.filling.clone();
        tools.name.connect_editing_notify(move |label| {
            if label.is_editing() || filling.get() {
                return;
            }
            let Some(window) = window.upgrade() else {
                return;
            };
            let text = label.text().trim().to_owned();
            window.rename_selected(&text);
        });
        let window = Rc::downgrade(self);
        tools.repeat.connect_changed(move |count, axis| {
            if let Some(window) = window.upgrade() {
                window.repeat_selected(count, Some(axis));
            }
        });
        let window = Rc::downgrade(self);
        let filling = tools.filling.clone();
        tools.mirror.connect_active_notify(move |switch| {
            if filling.get() || !switch.is_active() {
                return;
            }
            if let Some(window) = window.upgrade() {
                window.mirror_selected();
            }
        });
        let window = Rc::downgrade(self);
        tools.hold.connect_clicked(move |_| {
            if let Some(window) = window.upgrade() {
                window.hold_from_selected();
            }
        });
        let window = Rc::downgrade(self);
        tools.remove.connect_clicked(move |_| {
            if let Some(window) = window.upgrade() {
                window.remove_selected();
            }
        });
    }

    /// The item of the project the selection belongs to, by name.
    pub(in crate::ui) fn selected_item(&self) -> Option<String> {
        let session = self.session.borrow();
        let (design, _) = session.drawable()?;
        selected_item(design, session.selection()?)
    }

    /// Shows the item's changes on its card when the selection is a part,
    /// a piece of hardware or an assembly and nothing is being edited.
    pub(in crate::ui) fn update_item_tools(&self) {
        let tools = &self.editing.tools;
        let found = {
            let session = self.session.borrow();
            session.drawable().and_then(|(design, _)| {
                let selection = session.selection()?;
                let item = selected_item(design, selection)?;
                let measurable = measured_of(design, selection);
                let address = std::ptr::from_ref(design).addr();
                Some((item, measurable.err(), address))
            })
        };
        let Some((item, unmeasurable, address)) =
            found.filter(|_| !self.edit_open() && self.gestures_allowed())
        else {
            tools.root.set_visible(false);
            return;
        };
        let stale = tools
            .shown
            .borrow()
            .as_ref()
            .is_none_or(|shown| shown.item != item || shown.design != address);
        if stale {
            // The offer reads the project, so it is worked out once per
            // item and result.
            let offer = self.session.borrow().item_offer(&item);
            tools.shown.replace(Some(Shown {
                item: item.clone(),
                design: address,
                offer,
            }));
        }
        let Some(offer) = tools
            .shown
            .borrow()
            .as_ref()
            .and_then(|shown| shown.offer.clone())
        else {
            tools.root.set_visible(false);
            return;
        };
        let count = offer.pieces;
        tools.filling.set(true);
        tools
            .title
            .set_label(&format!("CHANGE {item}").to_uppercase());
        tools.name.set_text(&item);
        tools.repeat.show(count, tools.repeat.axis(), count < 2);
        tools.repeat.root.set_visible(offer.repeatable);
        tools.mirror.set_active(false);
        tools.mirror_row.set_visible(offer.mirror.is_some());
        match &offer.mirror {
            Some(MirrorGesture::Image(image)) => tools.mirror_words.set_label(&format!(
                "And the other side · adds {}",
                image.image.as_str()
            )),
            Some(MirrorGesture::Split(_)) => tools
                .mirror_words
                .set_label("And the other side · splits its space across into a mirrored pair"),
            None => {}
        }
        tools.hold.set_sensitive(unmeasurable.is_none());
        tools.hold.set_tooltip_text(Some(match unmeasurable {
            Some(why) => why,
            None => "Pick a second part or space and keep the distance to it",
        }));
        tools.filling.set(false);
        tools.root.set_visible(true);
    }

    /// Puts the keyboard in the item's name, ready to retype it.
    pub(in crate::ui) fn focus_item_name(&self) {
        let tools = &self.editing.tools;
        if tools.root.is_visible() {
            tools.name.start_editing();
        } else {
            self.edit_toast("Select a part or an assembly first, then rename it");
        }
    }

    // ---- the changes, one path for the card's controls and the finder ----

    /// Renames the selected item everywhere it is used: the change is
    /// tried and shown on its card.
    pub(in crate::ui) fn rename_selected(self: &Rc<Self>, to: &str) {
        let Some(item) = self.selected_item() else {
            return;
        };
        if to.is_empty() || to == item {
            return;
        }
        match (parse_name(&item), parse_name(to)) {
            (Ok(item), Ok(to)) => self.start_change(EditCommand::RenameItem { item, to }),
            (_, Err(error)) => self.edit_toast(&format!("'{to}' can't be a name: {}", error.0)),
            (Err(_), Ok(_)) => self.edit_toast(&format!("'{item}' can't be renamed here")),
        }
    }

    /// Makes the selected item `count` pieces filling its space, split
    /// along `axis` (a first repeat needs it); on the chosen candidate's
    /// card, the candidate instead.
    pub(in crate::ui) fn repeat_selected(self: &Rc<Self>, count: usize, axis: Option<Axis>) {
        let chosen = self
            .session
            .borrow()
            .edit()
            .is_some_and(|edit| edit.chosen().is_some());
        if chosen {
            let axis = axis.unwrap_or(Axis::Up);
            self.send_edit_later(EditCommand::Repeat(candidate_repeat(count, axis)));
            return;
        }
        let item = match self.change_target() {
            Some(item) => item,
            None => return,
        };
        let Some(count) = RepeatCount::new(count) else {
            self.edit_toast("A repeat makes two or more; Remove takes it away");
            return;
        };
        match parse_name(&item) {
            Ok(item) => self.start_change(EditCommand::RepeatItem { item, count, axis }),
            Err(error) => self.edit_toast(&error.0),
        }
    }

    /// Does what the session offers for the selected item's other side
    /// ([`stave_app::item_offer::MirrorGesture`]): adds its mirror image,
    /// or makes a handed piece filling a space a mirrored pair.
    pub(in crate::ui) fn mirror_selected(self: &Rc<Self>) {
        let Some(item) = self.change_target() else {
            return;
        };
        let Some(gesture) = self
            .session
            .borrow()
            .item_offer(&item)
            .and_then(|offer| offer.mirror)
        else {
            self.edit_toast(&format!("{item} has no other side to add"));
            return;
        };
        match parse_name(&item) {
            Ok(item) => self.start_change(gesture.command(item)),
            Err(error) => self.edit_toast(&error.0),
        }
    }

    /// Removes the selected item: the session lists what uses it, and the
    /// card (and its dialog) say what would go before anything does.
    /// `false` when nothing is selected that can be removed.
    pub(in crate::ui) fn remove_selected(self: &Rc<Self>) -> bool {
        if !self.gestures_allowed() {
            self.edit_toast(&self.parse_block_line().unwrap_or_default());
            return true;
        }
        let Some(item) = self.change_target() else {
            return false;
        };
        match parse_name(&item) {
            Ok(item) => self.start_change(EditCommand::Remove(item)),
            Err(error) => self.edit_toast(&error.0),
        }
        true
    }

    /// The item a change of the selection acts on: the selected item, or
    /// the item the change in progress acts on.
    fn change_target(&self) -> Option<String> {
        self.selected_item().or_else(|| {
            self.session
                .borrow()
                .change()
                .and_then(|change| super::changes::change_item(change.request()))
        })
    }

    /// Starts a change of the design, reading the editor's unsaved text.
    pub(in crate::ui) fn start_change(self: &Rc<Self>, command: EditCommand) {
        if !self.gestures_allowed() {
            self.edit_toast(&self.parse_block_line().unwrap_or_default());
            return;
        }
        {
            let mut session = self.session.borrow_mut();
            session.set_edit_files(std::sync::Arc::new(self.widgets.files.edit_files()));
        }
        self.editing.state.borrow_mut().select_after = None;
        self.send_edit_later(command);
    }

    /// The name of the item a commit of the change in progress leaves to
    /// select: the renamed or mirrored item by its new name.
    pub(in crate::ui) fn change_selects_after(&self) -> Option<String> {
        let session = self.session.borrow();
        let change = session.change()?;
        match change.request() {
            ChangeRequest::Rename { to, .. } => Some(to.as_str().to_owned()),
            ChangeRequest::Mirror { item }
            | ChangeRequest::MirrorSplit { item, .. }
            | ChangeRequest::Repeat { item, .. }
            | ChangeRequest::SetParam { item, .. } => match change.detail() {
                stave_app::change::ChangeDetail::Mirror { renamed, .. } => {
                    Some(renamed.as_str().to_owned())
                }
                _ => Some(item.as_str().to_owned()),
            },
            ChangeRequest::Remove { .. }
            | ChangeRequest::Hold(_)
            | ChangeRequest::MoveShares { .. }
            | ChangeRequest::RepeatPart { .. }
            | ChangeRequest::ParamKind { .. }
            | ChangeRequest::Grip { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_piece_is_no_repeat() {
        assert_eq!(candidate_repeat(1, Axis::Up), None);
        let two = candidate_repeat(2, Axis::Across).unwrap();
        assert_eq!(two.count.get(), 2);
        assert_eq!(two.axis, Axis::Across);
    }
}
