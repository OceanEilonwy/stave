//! Changes of a selected item, on its card (`docs/gui.md`, "Changing
//! what is there"): its name, retyped in place (a rename everywhere it is
//! used); how many there are, with a stepper (a repeat filling its space);
//! "And the other side" when the session offers a mirror image; holding a
//! distance from it; removing it. Each control and each verb of the
//! finder calls the same method of the window, which sends the session's
//! command: one path for the mouse and the keyboard.
//!
//! The count stepper and the item's tools are relm4 components
//! ([`RepeatRowComponent`], [`ItemToolsComponent`]) following
//! `docs/relm4-migration.md`; [`RepeatRow`] and [`ItemTools`] keep the
//! older API in front of them while the window (`impl Window`) is old
//! code. "Repeat…" ([`RepeatFoldRow`]) stays plain widgets: scripts press
//! it and read whether it opened in one step.
//!
//! ```text
//! ▾ Change                          drawer   (a section of the Inspector)
//! Name        [drawer          ]
//! How many    [- 2 +]  stacked [up ▾]
//! And the other side · adds door_right   (o )
//! [Hold a distance…]  [Remove…]
//! ```

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use relm4::{Component, ComponentParts, ComponentSender};
use stave_app::change::ChangeRequest;
use stave_app::editing::{EditCommand, parse_name};
use stave_app::hold::Measured;
use stave_app::item_offer::{ItemOffer, MirrorGesture, RepeatFold};
pub(in crate::ui) use stave_app::item_offer::{piece_count, selected_item};
use stave_app::naming::RepeatCount;
use stave_app::session::Selection;
use stave_lang::placement::Axis;
use stave_model::solved::SolvedDesign;

use super::super::Window;
use super::super::relm4_shim::Bridge;
use super::super::sections;

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

/// The words for the axes, in the order the list shows them.
const AXIS_WORDS: [&str; 3] = [AXES[0].1, AXES[1].1, AXES[2].1];

/// A count stepper with the axis to split along, on a candidate's card and
/// on a selected item's, as a relm4 component: its model is the count and
/// axis shown, and only what people do sends [`RepeatRowOutput::Changed`];
/// [`RepeatRowInput::Show`] fills it in with its handlers blocked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::ui) struct RepeatRowComponent {
    count: usize,
    axis: Axis,
    /// Whether the axis list shows (only while the count can choose it).
    axis_shown: bool,
    /// Whether it says "×" in place of "How many".
    short: bool,
}

impl RepeatRowComponent {
    /// The count, as the stepper holds it.
    fn count_value(&self) -> f64 {
        f64::from(u32::try_from(self.count).unwrap_or(1))
    }

    /// The axis's place in the list.
    fn axis_index(&self) -> u32 {
        AXES.iter()
            .position(|(each, _)| *each == self.axis)
            .and_then(|index| u32::try_from(index).ok())
            .unwrap_or(0)
    }
}

/// The axis at a place in the list (stacked up when out of it).
fn axis_at(index: u32) -> Axis {
    usize::try_from(index)
        .ok()
        .and_then(|index| AXES.get(index))
        .map_or(Axis::Up, |(axis, _)| *axis)
}

/// What a count stepper is told.
#[derive(Debug)]
pub(in crate::ui) enum RepeatRowInput {
    /// Show a count and an axis without reporting them; the axis list
    /// shows only while the count can still choose it.
    Show {
        /// The count.
        count: usize,
        /// The axis.
        axis: Axis,
        /// Whether the axis list shows.
        axis_shown: bool,
    },
    /// Say "×" in place of "How many".
    Shorten,
    /// People set the count.
    CountChanged(usize),
    /// People chose the axis at this place in the list.
    AxisChosen(u32),
}

/// What a count stepper tells its owner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::ui) enum RepeatRowOutput {
    /// People changed the count or the axis.
    Changed {
        /// The count.
        count: usize,
        /// The axis.
        axis: Axis,
    },
}

#[relm4::component(pub(in crate::ui))]
impl Component for RepeatRowComponent {
    /// The smallest count it offers.
    type Init = usize;
    type Input = RepeatRowInput;
    type Output = RepeatRowOutput;
    type CommandOutput = ();

    view! {
        gtk::Box {
            set_spacing: 8,
            add_css_class: "stave-card-row",
            add_css_class: "stave-repeat-row",

            gtk::Label {
                set_xalign: 0.0,
                add_css_class: "stave-key",
                #[watch]
                set_label: if model.short { "×" } else { "How many" },
                #[watch]
                set_tooltip_text: model
                    .short
                    .then_some("How many pieces fill the space, each the same size"),
            },

            #[name = "count"]
            gtk::SpinButton {
                set_adjustment: &gtk::Adjustment::new(
                    1.0,
                    f64::from(u32::try_from(least).unwrap_or(1)),
                    f64::from(u32::try_from(MOST).unwrap_or(12)),
                    1.0,
                    1.0,
                    0.0,
                ),
                set_digits: 0,
                set_width_chars: 2,
                set_numeric: true,
                set_tooltip_text: Some("How many pieces fill the space, each the same size"),
                update_property: &[gtk::accessible::Property::Label("How many")],
                #[watch]
                #[block_signal(count_changed)]
                set_value: model.count_value(),
                connect_value_changed[sender] => move |count| {
                    let count = usize::try_from(count.value_as_int()).unwrap_or(1);
                    sender.input_sender().emit(RepeatRowInput::CountChanged(count));
                } @count_changed,
            },

            #[name = "axis"]
            gtk::DropDown::from_strings(&AXIS_WORDS) -> gtk::DropDown {
                set_tooltip_text: Some("Which way the pieces are split"),
                update_property: &[gtk::accessible::Property::Label("Which way")],
                #[watch]
                set_visible: model.axis_shown,
                #[watch]
                #[block_signal(axis_chosen)]
                set_selected: model.axis_index(),
                connect_selected_notify[sender] => move |axis| {
                    sender.input_sender().emit(RepeatRowInput::AxisChosen(axis.selected()));
                } @axis_chosen,
            },
        }
    }

    fn init(least: usize, root: Self::Root, sender: ComponentSender<Self>) -> ComponentParts<Self> {
        let model = RepeatRowComponent {
            count: 1,
            axis: Axis::Up,
            axis_shown: true,
            short: false,
        };
        let widgets = view_output!();
        ComponentParts { model, widgets }
    }

    fn update(
        &mut self,
        message: RepeatRowInput,
        sender: ComponentSender<Self>,
        root: &Self::Root,
    ) {
        let changed = match message {
            RepeatRowInput::Show {
                count,
                axis,
                axis_shown,
            } => {
                self.count = count;
                self.axis = axis;
                self.axis_shown = axis_shown;
                false
            }
            RepeatRowInput::Shorten => {
                self.short = true;
                root.add_css_class("stave-repeat-short");
                false
            }
            RepeatRowInput::CountChanged(count) => {
                let changed = count != self.count;
                self.count = count;
                changed
            }
            RepeatRowInput::AxisChosen(index) => {
                let axis = axis_at(index);
                let changed = axis != self.axis;
                self.axis = axis;
                changed
            }
        };
        if changed {
            // Nobody listening is not a mistake.
            let _ = sender.output(RepeatRowOutput::Changed {
                count: self.count,
                axis: self.axis,
            });
        }
    }
}

/// A count stepper, behind the callback API the window still uses: it owns
/// the component and passes its setters on as [`RepeatRowInput`] messages.
pub(in crate::ui) struct RepeatRow {
    /// The row; its owner shows and hides it.
    pub(in crate::ui) root: gtk::Box,
    bridge: Bridge<RepeatRowComponent>,
}

impl RepeatRow {
    /// A row starting at one piece, stacked up; the smallest count it
    /// offers is `least`.
    pub(in crate::ui) fn new(least: usize) -> RepeatRow {
        let bridge = Bridge::<RepeatRowComponent>::launch(least);
        RepeatRow {
            root: bridge.widget().clone(),
            bridge,
        }
    }

    /// Says "×" in place of "How many" (in full in the stepper's tooltip
    /// and accessible label), so the row fits the Edit panel's width in
    /// the default layout, where the docked area would otherwise widen to
    /// hold it.
    pub(in crate::ui) fn shorten(&self) {
        self.bridge.send(RepeatRowInput::Shorten);
    }

    /// Shows a count and an axis without reporting them; the axis list
    /// shows only while the count can still choose it.
    pub(in crate::ui) fn show(&self, count: usize, axis: Axis, axis_shown: bool) {
        self.bridge.send(RepeatRowInput::Show {
            count,
            axis,
            axis_shown,
        });
    }

    /// The count shown, as of the last message the row handled.
    pub(in crate::ui) fn count(&self) -> usize {
        self.bridge.model().count
    }

    /// The axis chosen, as of the last message the row handled.
    pub(in crate::ui) fn axis(&self) -> Axis {
        self.bridge.model().axis
    }

    /// Calls `callback` with the count and axis when the user changes
    /// either.
    pub(in crate::ui) fn connect_changed(&self, callback: impl Fn(usize, Axis) + 'static) {
        self.bridge.connect(move |output| match output {
            RepeatRowOutput::Changed { count, axis } => callback(count, axis),
        });
    }

    /// The stepper and the axis list, to use them as people do, for a test.
    #[cfg(test)]
    fn controls(&self) -> Option<(gtk::SpinButton, gtk::DropDown)> {
        let mut spin = None;
        let mut list = None;
        let mut child = self.root.first_child();
        while let Some(widget) = child {
            if let Some(found) = widget.downcast_ref::<gtk::SpinButton>() {
                spin = Some(found.clone());
            }
            if let Some(found) = widget.downcast_ref::<gtk::DropDown>() {
                list = Some(found.clone());
            }
            child = widget.next_sibling();
        }
        spin.zip(list)
    }
}

/// The repeat controls folded behind a small "Repeat…" button
/// (`stave_app::item_offer::RepeatFold`, one rule for every card): the
/// button while folded, the count and the direction once open.
pub(in crate::ui) struct RepeatFoldRow {
    /// The button and the row.
    pub(in crate::ui) root: gtk::Box,
    unfold: gtk::Button,
    /// The count and the direction.
    pub(in crate::ui) row: RepeatRow,
    /// Whether the user pressed "Repeat…" for what is shown now.
    asked: Rc<Cell<bool>>,
}

impl RepeatFoldRow {
    /// Folded, starting at one piece.
    pub(in crate::ui) fn new(least: usize) -> RepeatFoldRow {
        let root = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .css_classes(["stave-repeat-fold"])
            .build();
        let unfold = gtk::Button::builder()
            .label("Repeat…")
            .halign(gtk::Align::Start)
            .css_classes(["flat", "stave-repeat-unfold"])
            .tooltip_text("Make more than one, each the same size, filling the space")
            .build();
        unfold.update_property(&[gtk::accessible::Property::Label("Repeat")]);
        let row = RepeatRow::new(least);
        root.append(&unfold);
        root.append(&row.root);
        let asked = Rc::new(Cell::new(false));
        {
            let asked = asked.clone();
            unfold.connect_clicked(move |_| asked.set(true));
        }
        RepeatFoldRow {
            root,
            unfold,
            row,
            asked,
        }
    }

    /// Shows what the rule says: hidden, the button, or the count and the
    /// direction.
    pub(in crate::ui) fn show(&self, fold: RepeatFold) {
        let (any, open) = match fold {
            RepeatFold::Hidden => (false, false),
            RepeatFold::Folded => (true, false),
            RepeatFold::Open => (true, true),
        };
        self.root.set_visible(any);
        self.unfold.set_visible(any && !open);
        self.row.root.set_visible(open);
    }

    /// `true` once the user pressed "Repeat…".
    pub(in crate::ui) fn asked(&self) -> bool {
        self.asked.get()
    }

    /// Puts "Repeat…" in one place and the count and direction in another
    /// (the candidate's card: "Repeat…" on the line of its buttons, the
    /// count above them, so folded it takes no line of its own).
    pub(in crate::ui) fn place_apart(&self, button_slot: &gtk::Box, row_slot: &gtk::Box) {
        self.root.remove(&self.unfold);
        self.root.remove(&self.row.root);
        button_slot.append(&self.unfold);
        row_slot.append(&self.row.root);
        self.row.shorten();
    }

    /// Forgets that "Repeat…" was pressed (something else is shown).
    pub(in crate::ui) fn forget(&self) {
        self.asked.set(false);
    }

    /// `true` while the count and the direction show.
    pub(in crate::ui) fn is_open(&self) -> bool {
        self.root.is_visible() && self.row.root.is_visible()
    }

    /// Presses "Repeat…", as a click does (scripts); `false` when it
    /// doesn't show.
    pub(in crate::ui) fn press(&self) -> bool {
        if !self.unfold.is_visible() || !self.root.is_visible() {
            return false;
        }
        self.unfold.emit_clicked();
        true
    }

    /// Calls `callback` after "Repeat…" is pressed.
    pub(in crate::ui) fn connect_unfold(&self, callback: impl Fn() + 'static) {
        self.unfold.connect_clicked(move |_| callback());
    }

    /// The button (for tests).
    #[cfg(test)]
    pub(in crate::ui) fn unfold_button(&self) -> gtk::Button {
        self.unfold.clone()
    }
}

/// What a selected item's tools show, worked out by the window from the
/// session (`Window::update_item_tools`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(in crate::ui) struct ItemToolsShown {
    /// The item, by name.
    pub(in crate::ui) item: String,
    /// What "And the other side" does, in words; `None` when the session
    /// offers no other side.
    pub(in crate::ui) mirror: Option<String>,
    /// Why no distance can be held from it, if none can.
    pub(in crate::ui) unmeasurable: Option<&'static str>,
}

/// The changes a selected item's card offers, as a relm4 component: its
/// model is what it shows ([`ItemToolsShown`]); its name field, Mirror
/// switch and buttons only send [`ItemToolsInput`] messages. Showing it
/// fills in the name and turns Mirror off with their handlers blocked, so
/// only people's renames and switches are reported. The repeat controls on
/// it ([`RepeatFoldRow`]) are its owner's, placed among its rows at launch.
#[derive(Debug)]
pub(in crate::ui) struct ItemToolsComponent {
    shown: ItemToolsShown,
    /// Set for the update a [`ItemToolsInput::Show`] makes: the name and
    /// Mirror are filled in again.
    refill: bool,
    /// The section's note, where the item's name shows (outside the rows).
    title: gtk::Label,
}

/// What a selected item's tools are made with.
#[derive(Debug)]
pub(in crate::ui) struct ItemToolsInit {
    /// The section's note, for the item's name.
    pub(in crate::ui) title: gtk::Label,
    /// The repeat controls' row, put after the name.
    pub(in crate::ui) repeat: gtk::Box,
}

/// What a selected item's tools are told.
#[derive(Debug)]
pub(in crate::ui) enum ItemToolsInput {
    /// Show these.
    Show(ItemToolsShown),
    /// Put the keyboard in the name, ready to retype it.
    StartEditing,
    /// People finished retyping the name: this is in it.
    Renamed(String),
    /// People switched "And the other side" on.
    MirrorOn,
    /// People pressed "Hold a distance…".
    Hold,
    /// People pressed "Remove…".
    Remove,
}

/// What a selected item's tools tell their owner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::ui) enum ItemToolsOutput {
    /// Rename the item to this.
    Rename(String),
    /// Add its other side.
    Mirror,
    /// Hold a distance from it.
    Hold,
    /// Remove it.
    Remove,
}

#[relm4::component(pub(in crate::ui))]
impl Component for ItemToolsComponent {
    type Init = ItemToolsInit;
    type Input = ItemToolsInput;
    type Output = ItemToolsOutput;
    type CommandOutput = ();

    view! {
        gtk::Box {
            set_orientation: gtk::Orientation::Vertical,
            add_css_class: "stave-card",
            add_css_class: "stave-edit-rows",

            gtk::Box {
                set_spacing: 6,
                add_css_class: "stave-card-row",
                add_css_class: "stave-edit-row",
                append: &sections::row_label("Name"),

                #[name = "name"]
                gtk::EditableLabel {
                    set_hexpand: true,
                    add_css_class: "stave-edit-name",
                    add_css_class: "stave-name-field",
                    set_tooltip_text: Some("Click to retype the name: it changes everywhere it is used"),
                    update_property: &[gtk::accessible::Property::Label("Name")],
                    #[track(model.refill)]
                    #[block_signal(name_editing)]
                    set_text: &model.shown.item,
                    // When the retyping ends, what is in it.
                    connect_editing_notify[sender] => move |label| {
                        if !label.is_editing() {
                            let text = label.text().trim().to_owned();
                            sender.input_sender().emit(ItemToolsInput::Renamed(text));
                        }
                    } @name_editing,
                },
            },

            append: &repeat,

            gtk::Box {
                set_spacing: 10,
                add_css_class: "stave-card-row",
                add_css_class: "stave-edit-row",
                #[watch]
                set_visible: model.shown.mirror.is_some(),

                gtk::Label {
                    set_xalign: 0.0,
                    set_hexpand: true,
                    set_wrap: true,
                    #[watch]
                    set_label: model.shown.mirror.as_deref().unwrap_or_default(),
                },
                gtk::Switch {
                    set_valign: gtk::Align::Center,
                    set_tooltip_text: Some("Add its mirror image on the other side"),
                    update_property: &[gtk::accessible::Property::Label("And the other side")],
                    #[track(model.refill)]
                    #[block_signal(mirror_switched)]
                    set_active: false,
                    connect_active_notify[sender] => move |switch| {
                        if switch.is_active() {
                            sender.input_sender().emit(ItemToolsInput::MirrorOn);
                        }
                    } @mirror_switched,
                },
            },

            gtk::Box {
                set_spacing: 8,
                add_css_class: "stave-card-row",
                add_css_class: "stave-edit-row",
                add_css_class: "stave-item-buttons",

                gtk::Button {
                    set_label: "Hold a distance…",
                    add_css_class: "flat",
                    set_hexpand: true,
                    #[watch]
                    set_sensitive: model.shown.unmeasurable.is_none(),
                    #[watch]
                    set_tooltip_text: Some(model.shown.unmeasurable.unwrap_or(
                        "Pick a second part or space and keep the distance to it",
                    )),
                    connect_clicked[sender] => move |_| {
                        sender.input_sender().emit(ItemToolsInput::Hold);
                    },
                },
                gtk::Button {
                    set_label: "Remove…",
                    add_css_class: "flat",
                    add_css_class: "error",
                    set_hexpand: true,
                    set_tooltip_text: Some("Take it out of the design (Delete)"),
                    connect_clicked[sender] => move |_| {
                        sender.input_sender().emit(ItemToolsInput::Remove);
                    },
                },
            },
        }
    }

    fn init(
        init: ItemToolsInit,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let ItemToolsInit { title, repeat } = init;
        let model = ItemToolsComponent {
            shown: ItemToolsShown::default(),
            refill: false,
            title,
        };
        let widgets = view_output!();
        ComponentParts { model, widgets }
    }

    fn update_with_view(
        &mut self,
        widgets: &mut Self::Widgets,
        message: ItemToolsInput,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        self.refill = false;
        let output = match message {
            ItemToolsInput::Show(shown) => {
                self.title.set_label(&shown.item);
                self.title.set_visible(true);
                self.shown = shown;
                self.refill = true;
                None
            }
            ItemToolsInput::StartEditing => {
                widgets.name.start_editing();
                None
            }
            ItemToolsInput::Renamed(text) => Some(ItemToolsOutput::Rename(text)),
            ItemToolsInput::MirrorOn => Some(ItemToolsOutput::Mirror),
            ItemToolsInput::Hold => Some(ItemToolsOutput::Hold),
            ItemToolsInput::Remove => Some(ItemToolsOutput::Remove),
        };
        self.update_view(widgets, sender.clone());
        if let Some(output) = output {
            // Nobody listening is not a mistake.
            let _ = sender.output(output);
        }
    }
}

/// What the tools were last worked out for, so the offer (which reads the
/// project) is worked out once per item and result.
#[derive(Debug, Clone, PartialEq)]
struct Shown {
    item: String,
    /// The design it was worked out from, by address: a new result works
    /// the offer out again.
    design: usize,
    /// What the session offers for it.
    offer: Option<ItemOffer>,
}

/// The changes a selected item's card offers, behind the API the window
/// still uses: the section it sits in (its owner shows and hides it), the
/// component that draws it, and the repeat controls placed on it.
pub(in crate::ui) struct ItemTools {
    /// The panel, at the bottom of the part's card.
    pub(in crate::ui) root: gtk::Box,
    repeat: RepeatFoldRow,
    bridge: Bridge<ItemToolsComponent>,
    /// The owner's cache of the offer for the item shown: read and
    /// replaced at once by `Window::update_item_tools`, never borrowed
    /// across a call to GTK.
    shown: RefCell<Option<Shown>>,
}

impl ItemTools {
    /// The panel, hidden.
    pub(in crate::ui) fn new() -> ItemTools {
        // A section of the Inspector, "Change" with the item as its note.
        let (root, body, title) = sections::section_noted("Change", true);
        root.add_css_class("stave-item-tools");
        root.add_css_class("stave-edit-card");
        root.set_visible(false);
        let repeat = RepeatFoldRow::new(1);
        repeat.root.add_css_class("stave-card-row");
        repeat.root.add_css_class("stave-edit-row");
        let bridge = Bridge::<ItemToolsComponent>::launch(ItemToolsInit {
            title,
            repeat: repeat.root.clone(),
        });
        body.append(bridge.widget());
        ItemTools {
            root,
            repeat,
            bridge,
            shown: RefCell::new(None),
        }
    }

    /// Its count stepper.
    pub(in crate::ui) fn repeat_row(&self) -> &RepeatRow {
        &self.repeat.row
    }

    /// Its repeat controls, folded or open.
    pub(in crate::ui) fn repeat_fold(&self) -> &RepeatFoldRow {
        &self.repeat
    }

    /// Calls `callback` with each change people ask for on it.
    pub(in crate::ui) fn connect(&self, callback: impl Fn(ItemToolsOutput) + 'static) {
        self.bridge.connect(callback);
    }

    /// Shows these when the main loop next runs.
    fn show(&self, shown: ItemToolsShown) {
        self.bridge.send(ItemToolsInput::Show(shown));
    }

    /// Puts the keyboard in the name when the main loop next runs.
    fn start_editing(&self) {
        self.bridge.send(ItemToolsInput::StartEditing);
    }
}

impl Window {
    /// Puts the item's changes in the Details panel, under the
    /// selection's account, and connects them.
    pub(in crate::ui) fn connect_item_tools(self: &Rc<Self>) {
        let tools = &self.editing.tools;
        self.widgets.details.append(&tools.root);
        let window = Rc::downgrade(self);
        tools.connect(move |output| {
            let Some(window) = window.upgrade() else {
                return;
            };
            match output {
                ItemToolsOutput::Rename(text) => window.rename_selected(&text),
                ItemToolsOutput::Mirror => window.mirror_selected(),
                ItemToolsOutput::Hold => window.hold_from_selected(),
                ItemToolsOutput::Remove => {
                    window.remove_selected();
                }
            }
        });
        let window = Rc::downgrade(self);
        tools.repeat.connect_unfold(move || {
            if let Some(window) = window.upgrade() {
                window.update_item_tools();
            }
        });
        let window = Rc::downgrade(self);
        tools.repeat.row.connect_changed(move |count, axis| {
            if let Some(window) = window.upgrade() {
                window.repeat_selected(count, Some(axis));
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
        let (stale, another_item) = tools.shown.borrow().as_ref().map_or((true, true), |shown| {
            (
                shown.item != item || shown.design != address,
                shown.item != item,
            )
        });
        if stale {
            // Another item: "Repeat…" is asked for again.
            if another_item {
                tools.repeat.forget();
            }
            // The offer reads the project, so it is worked out once per
            // item and result.
            let offer = self.session.borrow().item_offer(&item);
            tools.shown.replace(Some(Shown {
                item: item.clone(),
                design: address,
                offer,
            }));
        }
        let offer = tools
            .shown
            .borrow()
            .as_ref()
            .and_then(|shown| shown.offer.clone());
        let Some(offer) = offer else {
            tools.root.set_visible(false);
            return;
        };
        let count = offer.pieces;
        tools
            .repeat
            .row
            .show(count, tools.repeat.row.axis(), count < 2);
        tools.repeat.show(offer.repeat_fold(tools.repeat.asked()));
        let mirror = offer.mirror.as_ref().map(|gesture| match gesture {
            MirrorGesture::Image(image) => {
                format!("And the other side · adds {}", image.image.as_str())
            }
            MirrorGesture::Split(_) => {
                "And the other side · splits its space across into a mirrored pair".to_owned()
            }
        });
        tools.show(ItemToolsShown {
            item,
            mirror,
            unmeasurable,
        });
        tools.root.set_visible(true);
    }

    /// Puts the keyboard in the item's name, ready to retype it.
    pub(in crate::ui) fn focus_item_name(&self) {
        let tools = &self.editing.tools;
        if tools.root.is_visible() {
            tools.start_editing();
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
    use std::cell::RefCell;

    use super::*;

    #[test]
    fn one_piece_is_no_repeat() {
        assert_eq!(candidate_repeat(1, Axis::Up), None);
        let two = candidate_repeat(2, Axis::Across).unwrap();
        assert_eq!(two.count.get(), 2);
        assert_eq!(two.axis, Axis::Across);
    }

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

    #[test]
    fn a_dropped_repeat_row_is_freed() {
        on_gtk_thread(repeat_row_is_freed);
    }

    /// Dropping the row drops its controller: the widget, the model and
    /// the callback are freed once the main loop runs.
    fn repeat_row_is_freed() {
        let row = RepeatRow::new(1);
        let token = Rc::new(());
        let held = Rc::clone(&token);
        row.connect_changed(move |_, _| {
            let _ = &held;
        });
        let callback = Rc::downgrade(&token);
        drop(token);
        settle();
        let root = row.root.downgrade();
        let input = row.bridge.sender().clone();
        drop(row);
        settle();
        assert!(root.upgrade().is_none(), "the row is freed");
        assert!(
            input.send(RepeatRowInput::Shorten).is_err(),
            "the component's runtime, and with it the model, is gone"
        );
        assert!(callback.upgrade().is_none(), "the callback is freed");
    }

    #[test]
    fn showing_a_count_reports_nothing() {
        on_gtk_thread(showing_a_count_reports_nothing_body);
    }

    /// The old row set a `filling` flag around its own setters; now its
    /// handlers are blocked while the view follows the model.
    fn showing_a_count_reports_nothing_body() {
        let row = RepeatRow::new(1);
        let heard: Rc<RefCell<Vec<(usize, Axis)>>> = Rc::default();
        let log = Rc::clone(&heard);
        row.connect_changed(move |count, axis| log.borrow_mut().push((count, axis)));
        row.show(3, Axis::Deep, false);
        row.shorten();
        settle();
        assert_eq!((row.count(), row.axis()), (3, Axis::Deep));
        let Some((spin, list)) = row.controls() else {
            panic!("the row has a stepper and an axis list");
        };
        assert_eq!(spin.value_as_int(), 3);
        assert_eq!(list.selected(), 2);
        assert!(!list.is_visible(), "the axis list follows the model");
        assert!(heard.borrow().is_empty(), "showing it reports nothing");
    }

    #[test]
    fn a_count_set_while_a_count_is_handled_is_queued() {
        on_gtk_thread(counts_are_queued);
    }

    /// The old row called back from inside the stepper's own signal. Now
    /// people's change only queues a message; it reaches the callback after
    /// the row's update has returned, and a change made from inside the
    /// callback waits its turn.
    fn counts_are_queued() {
        let row = Rc::new(RepeatRow::new(1));
        let Some((spin, list)) = row.controls() else {
            panic!("the row has a stepper and an axis list");
        };
        let heard: Rc<RefCell<Vec<(usize, Axis)>>> = Rc::default();
        let log = Rc::clone(&heard);
        let weak = Rc::downgrade(&row);
        row.connect_changed(move |count, axis| {
            let Some(row) = weak.upgrade() else {
                return;
            };
            let first = log.borrow().is_empty();
            log.borrow_mut().push((count, axis));
            // The model is free to read: the update has returned.
            assert_eq!(row.count(), count);
            if first {
                list.set_selected(1);
                assert_eq!(log.borrow().len(), 1, "nothing ran under the callback");
            }
        });
        spin.set_value(4.0);
        assert!(heard.borrow().is_empty(), "the change is only queued");
        settle();
        assert_eq!(*heard.borrow(), vec![(4, Axis::Up), (4, Axis::Across)]);
    }

    #[test]
    fn dropped_item_tools_are_freed() {
        on_gtk_thread(item_tools_are_freed);
    }

    fn item_tools_are_freed() {
        let tools = ItemTools::new();
        let token = Rc::new(());
        let held = Rc::clone(&token);
        tools.connect(move |_| {
            let _ = &held;
        });
        let callback = Rc::downgrade(&token);
        drop(token);
        tools.show(ItemToolsShown {
            item: "drawer".to_owned(),
            mirror: None,
            unmeasurable: None,
        });
        settle();
        let rows = tools.bridge.widget().downgrade();
        let input = tools.bridge.sender().clone();
        drop(tools);
        settle();
        assert!(rows.upgrade().is_none(), "the rows are freed");
        assert!(
            input.send(ItemToolsInput::Hold).is_err(),
            "the component's runtime, and with it the model, is gone"
        );
        assert!(callback.upgrade().is_none(), "the callback is freed");
    }

    #[test]
    fn showing_item_tools_reports_nothing_and_a_switch_made_in_a_callback_is_queued() {
        on_gtk_thread(item_tools_are_queued);
    }

    /// The old tools kept a `filling` flag so filling them in reported
    /// nothing. Now showing them blocks their handlers, and people's input
    /// only queues a message: the callback runs after the update, and
    /// Mirror switched on from inside it waits its turn.
    fn item_tools_are_queued() {
        let tools = Rc::new(ItemTools::new());
        let heard: Rc<RefCell<Vec<ItemToolsOutput>>> = Rc::default();
        let log = Rc::clone(&heard);
        let rows: gtk::Widget = tools.bridge.widget().clone().upcast();
        let Some(mirror) = descendants::<gtk::Switch>(&rows).first().cloned() else {
            panic!("the tools have a Mirror switch");
        };
        let shown = ItemToolsShown {
            item: "door".to_owned(),
            mirror: Some("And the other side · adds door_right".to_owned()),
            unmeasurable: Some("pick one of its parts to measure from"),
        };
        mirror.set_active(true);
        settle();
        heard.borrow_mut().clear();
        let weak = Rc::downgrade(&tools);
        let again = shown.clone();
        let inner = mirror.clone();
        tools.connect(move |output| {
            let Some(tools) = weak.upgrade() else {
                return;
            };
            let first = log.borrow().is_empty();
            log.borrow_mut().push(output);
            if first {
                tools.show(again.clone());
                inner.set_active(true);
                assert_eq!(log.borrow().len(), 1, "nothing ran under the callback");
            }
        });
        tools.show(shown.clone());
        settle();
        assert!(!mirror.is_active(), "showing turns Mirror off");
        let names = descendants::<gtk::EditableLabel>(&rows);
        assert_eq!(
            names.first().map(|name| name.text().to_string()).as_deref(),
            Some("door")
        );
        let buttons = descendants::<gtk::Button>(&rows);
        let hold = buttons
            .iter()
            .find(|button| button.label().as_deref() == Some("Hold a distance…"));
        assert!(
            hold.is_some_and(|hold| !hold.is_sensitive()),
            "nothing to hold from"
        );
        assert!(heard.borrow().is_empty(), "showing reports nothing");
        let Some(remove) = buttons
            .iter()
            .find(|button| button.label().as_deref() == Some("Remove…"))
        else {
            panic!("the tools have Remove");
        };
        remove.emit_clicked();
        assert!(heard.borrow().is_empty(), "the press is only queued");
        settle();
        assert_eq!(
            *heard.borrow(),
            vec![ItemToolsOutput::Remove, ItemToolsOutput::Mirror],
            "the switch made in the callback came after it"
        );
    }
}
