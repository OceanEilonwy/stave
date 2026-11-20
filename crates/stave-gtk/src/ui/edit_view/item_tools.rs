//! Changes of a selected item, on its card (`docs/gui.md`, "Changing
//! what is there"): its name, retyped in place (a rename everywhere it is
//! used); how many there are, with a stepper (a repeat filling its space);
//! "And the other side" when the session offers a mirror image; holding a
//! distance from it; removing it. Each control and each verb of the
//! finder calls the same method of the window, which sends the session's
//! command: one path for the mouse and the keyboard.
//!
//! The count stepper, the repeat controls folded behind "Repeat…" and the
//! item's tools are relm4 components ([`RepeatRowComponent`],
//! [`RepeatFoldComponent`], [`ItemToolsComponent`]) following
//! `docs/relm4-migration.md`. The window owns the tools' and their repeat
//! controls' controllers ([`ItemTools`]), launched with their outputs
//! forwarded to its input, and keeps its own copy of what the repeat
//! controls show ([`RepeatFoldShown`]), which scripts read and press in
//! one step. The candidate card's repeat controls (`repeat_handles.rs`)
//! are a [`RepeatFoldComponent`] the window holds the same way.
//!
//! ```text
//! ▾ Change                          drawer   (a section of the Inspector)
//! Name        [drawer          ]
//! How many    [- 2 +]  stacked [up ▾]
//! And the other side · adds door_right   (o )
//! [Hold a distance…]  [Remove…]
//! ```

use stave_app::i18n::{fill_in, tr, tr_noop};

use crate::ui::update::editing::EditingInput;
use adw::prelude::*;
use relm4::{Component, ComponentController, ComponentParts, ComponentSender};
use stave_app::change::ChangeRequest;
use stave_app::editing::{EditCommand, parse_name};
use stave_app::hold::Measured;
use stave_app::item_offer::{ItemOffer, MirrorGesture, RepeatFold};
pub(in crate::ui) use stave_app::item_offer::{piece_count, selected_item};
use stave_app::naming::RepeatCount;
use stave_app::session::Selection;
use stave_lang::placement::Axis;
use stave_model::solved::SolvedDesign;

use super::super::sections;
use super::super::{Window, WindowInput};

/// The axes a repeat can split along, in the order the list shows them,
/// with the word for how the pieces stand.
pub(in crate::ui) const AXES: [(Axis, &str); 3] = [
    (Axis::Up, tr_noop("stacked up")),
    (Axis::Across, tr_noop("side by side")),
    (Axis::Deep, tr_noop("front to back")),
];

/// The most pieces the stepper and the drag handle go to.
pub(in crate::ui) const MOST: usize = 12;

/// The thing a hold measures from, for a selection: a part by its whole
/// name; `None` for an assembly (a distance is measured between parts or
/// spaces), with the reason.
pub(in crate::ui) fn measured_of(
    design: &SolvedDesign,
    selection: &Selection,
) -> Result<Measured, String> {
    match selection {
        Selection::Part(part) => design
            .part(*part)
            .map(|part| Measured::Item(part.name.clone()))
            .ok_or_else(|| tr("that part is gone")),
        Selection::Assembly(_) => Err(tr("pick one of its parts to measure from")),
        Selection::Hardware(_)
        | Selection::Joint { .. }
        | Selection::Member(_)
        | Selection::Location(_)
        | Selection::Problem(_) => Err(tr("a distance is measured from a part or a space")),
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

/// The words for the axes, translated, in the order the list shows them.
fn axis_words() -> gtk::StringList {
    AXES.iter().map(|(_, words)| tr(words)).collect()
}

/// A count stepper with the axis to split along, on a candidate's card and
/// on a selected item's, as a relm4 component: its model is the count and
/// axis shown, and only what people do sends [`RepeatRowOutput::Changed`];
/// [`RepeatRowInput::Show`] fills it in with its handlers blocked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::ui) struct RepeatRowComponent {
    /// The count shown, within the stepper's range.
    count: usize,
    axis: Axis,
    /// The smallest count the stepper offers.
    least: usize,
    /// How many [`RepeatRowInput::Show`] messages it has handled, so its
    /// owner can tell a change people made before the last one it sent.
    shows: u64,
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

/// `count` as the stepper clamps it, to `least..=MOST` (GTK takes the
/// upper bound first, then the lower, and so does this).
fn clamp_count(count: usize, least: usize) -> usize {
    count.min(MOST).max(least)
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
        /// How many [`RepeatRowInput::Show`] messages the row had handled
        /// when people made the change: fewer than its owner sent means a
        /// later `Show` replaced it.
        shows: u64,
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
                set_label: &if model.short { "×".to_owned() } else { tr("How many") },
                #[watch]
                set_tooltip_text: model
                    .short
                    .then(|| tr("How many pieces fill the space, each the same size"))
                    .as_deref(),
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
                set_tooltip_text: Some(&tr("How many pieces fill the space, each the same size")),
                update_property: &[gtk::accessible::Property::Label(&tr("How many"))],
                #[watch]
                #[block_signal(count_changed)]
                set_value: model.count_value(),
                connect_value_changed[sender] => move |count| {
                    let count = usize::try_from(count.value_as_int()).unwrap_or(1);
                    sender.input_sender().emit(RepeatRowInput::CountChanged(count));
                } @count_changed,
            },

            #[name = "axis"]
            gtk::DropDown::new(Some(axis_words()), gtk::Expression::NONE) -> gtk::DropDown {
                set_tooltip_text: Some(&tr("Which way the pieces are split")),
                update_property: &[gtk::accessible::Property::Label(&tr("Which way"))],
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
            count: clamp_count(1, least),
            axis: Axis::Up,
            least,
            shows: 0,
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
                self.count = clamp_count(count, self.least);
                self.axis = axis;
                self.axis_shown = axis_shown;
                self.shows = self.shows.wrapping_add(1);
                false
            }
            RepeatRowInput::Shorten => {
                self.short = true;
                root.add_css_class("stave-repeat-short");
                false
            }
            RepeatRowInput::CountChanged(count) => {
                let count = clamp_count(count, self.least);
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
                shows: self.shows,
            });
        }
    }
}

/// The repeat controls folded behind a small "Repeat…" button, as a relm4
/// component its owner holds through a `Controller`: the button while
/// folded, the count and the direction (a [`RepeatRowComponent`] it owns)
/// once open. It keeps only what it shows; whether people asked for it,
/// and the count and axis a script reads between steps, are the owner's,
/// in a [`RepeatFoldShown`] it keeps beside the controller and updates from
/// this component's outputs.
#[derive(Debug)]
pub(in crate::ui) struct RepeatFoldComponent {
    /// The count and the direction.
    row: relm4::Controller<RepeatRowComponent>,
}

/// Where a [`RepeatFoldComponent`]'s parts go when they are placed apart
/// (the candidate's card: "Repeat…" on the line of its buttons, the count
/// above them, so folded it takes no line of its own).
#[derive(Debug)]
pub(in crate::ui) struct RepeatFoldSlots {
    /// Where "Repeat…" goes.
    pub(in crate::ui) button: gtk::Box,
    /// Where the count and the direction go; placed apart, the row says
    /// "×" in place of "How many".
    pub(in crate::ui) row: gtk::Box,
}

/// What a [`RepeatFoldComponent`] is made with.
#[derive(Debug)]
pub(in crate::ui) struct RepeatFoldInit {
    /// The smallest count the stepper offers.
    pub(in crate::ui) least: usize,
    /// Style classes its root takes beside its own, to sit among its
    /// owner's rows.
    pub(in crate::ui) classes: &'static [&'static str],
    /// Where its parts go, if not in its own root.
    pub(in crate::ui) apart: Option<RepeatFoldSlots>,
}

/// A [`RepeatFoldComponent`]'s widgets.
#[derive(Debug)]
pub(in crate::ui) struct RepeatFoldWidgets {
    unfold: gtk::Button,
    row: gtk::Box,
}

#[cfg(test)]
impl RepeatFoldWidgets {
    /// "Repeat…".
    pub(in crate::ui) fn unfold_button(&self) -> &gtk::Button {
        &self.unfold
    }

    /// The count and the direction.
    pub(in crate::ui) fn row(&self) -> &gtk::Box {
        &self.row
    }
}

/// What a [`RepeatFoldComponent`] is told.
#[derive(Debug)]
pub(in crate::ui) enum RepeatFoldInput {
    /// Show what the rule says: hidden, the button, or the count and the
    /// direction. Reports nothing.
    Show(RepeatFold),
    /// Show a count and an axis on the row without reporting them; the
    /// axis list shows only while the count can still choose it.
    ShowCount {
        /// The count.
        count: usize,
        /// The axis.
        axis: Axis,
        /// Whether the axis list shows.
        axis_shown: bool,
    },
    /// People pressed "Repeat…".
    Unfold,
    /// People changed the row's count or axis.
    RowChanged {
        /// The count.
        count: usize,
        /// The axis.
        axis: Axis,
        /// How many `ShowCount` messages the row had handled.
        shows: u64,
    },
}

/// What a [`RepeatFoldComponent`] tells its owner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::ui) enum RepeatFoldOutput {
    /// People pressed "Repeat…".
    Unfold,
    /// People changed the count or the axis.
    Changed {
        /// The count.
        count: usize,
        /// The axis.
        axis: Axis,
        /// How many `ShowCount` messages the row had handled when people
        /// made the change: fewer than its owner sent means a later one
        /// replaced it ([`RepeatFoldShown::take_change`]).
        shows: u64,
    },
}

impl Component for RepeatFoldComponent {
    type Init = RepeatFoldInit;
    type Input = RepeatFoldInput;
    type Output = RepeatFoldOutput;
    type CommandOutput = ();
    type Root = gtk::Box;
    type Widgets = RepeatFoldWidgets;

    fn init_root() -> gtk::Box {
        gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .css_classes(["stave-repeat-fold"])
            .visible(false)
            .build()
    }

    fn init(
        init: RepeatFoldInit,
        root: gtk::Box,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let unfold = gtk::Button::builder()
            .label(tr("Repeat…"))
            .halign(gtk::Align::Start)
            .css_classes(["flat", "stave-repeat-unfold"])
            .tooltip_text(tr(
                "Make more than one, each the same size, filling the space",
            ))
            .visible(false)
            .build();
        unfold.update_property(&[gtk::accessible::Property::Label(&tr("Repeat"))]);
        let input = sender.input_sender().clone();
        unfold.connect_clicked(move |_| input.emit(RepeatFoldInput::Unfold));
        let row = RepeatRowComponent::builder().launch(init.least).forward(
            sender.input_sender(),
            |output| match output {
                RepeatRowOutput::Changed { count, axis, shows } => {
                    RepeatFoldInput::RowChanged { count, axis, shows }
                }
            },
        );
        for class in init.classes {
            root.add_css_class(class);
        }
        let row_root = row.widget().clone();
        row_root.set_visible(false);
        match &init.apart {
            Some(slots) => {
                slots.button.append(&unfold);
                slots.row.append(&row_root);
                let _ = row.sender().send(RepeatRowInput::Shorten);
            }
            None => {
                root.append(&unfold);
                root.append(&row_root);
            }
        }
        ComponentParts {
            model: RepeatFoldComponent { row },
            widgets: RepeatFoldWidgets {
                unfold,
                row: row_root,
            },
        }
    }

    fn update_with_view(
        &mut self,
        widgets: &mut RepeatFoldWidgets,
        message: RepeatFoldInput,
        sender: ComponentSender<Self>,
        root: &gtk::Box,
    ) {
        let output = match message {
            RepeatFoldInput::Show(fold) => {
                let (any, open) = match fold {
                    RepeatFold::Hidden => (false, false),
                    RepeatFold::Folded => (true, false),
                    RepeatFold::Open => (true, true),
                };
                root.set_visible(any);
                widgets.unfold.set_visible(any && !open);
                widgets.row.set_visible(open);
                None
            }
            RepeatFoldInput::ShowCount {
                count,
                axis,
                axis_shown,
            } => {
                let _ = self.row.sender().send(RepeatRowInput::Show {
                    count,
                    axis,
                    axis_shown,
                });
                None
            }
            RepeatFoldInput::Unfold => Some(RepeatFoldOutput::Unfold),
            RepeatFoldInput::RowChanged { count, axis, shows } => {
                Some(RepeatFoldOutput::Changed { count, axis, shows })
            }
        };
        if let Some(output) = output {
            // Nobody listening is not a mistake.
            let _ = sender.output(output);
        }
    }
}

/// The owner's copy of what its [`RepeatFoldComponent`] shows, kept in the
/// owner's model beside the controller: plain values, written only by the
/// owner, so scripts and the count handle read them at once while the
/// component draws them when the main loop next runs. Each setter returns
/// the message to send to the component.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::ui) struct RepeatFoldShown {
    /// What the rule last said.
    fold: RepeatFold,
    /// Whether people (or a script) pressed "Repeat…" for what is shown.
    asked: bool,
    /// The count (within the stepper's range) and the axis.
    count: usize,
    axis: Axis,
    /// The smallest count the stepper offers.
    least: usize,
    /// How many `ShowCount` messages were sent.
    shows: u64,
}

impl RepeatFoldShown {
    /// Hidden, at the least count, stacked up: what a new component shows.
    pub(in crate::ui) fn new(least: usize) -> RepeatFoldShown {
        RepeatFoldShown {
            fold: RepeatFold::Hidden,
            asked: false,
            count: clamp_count(1, least),
            axis: Axis::Up,
            least,
            shows: 0,
        }
    }

    /// Shows what the rule says.
    pub(in crate::ui) fn show(&mut self, fold: RepeatFold) -> RepeatFoldInput {
        self.fold = fold;
        RepeatFoldInput::Show(fold)
    }

    /// Shows a count and an axis on the row, clamped as the stepper
    /// clamps the count.
    pub(in crate::ui) fn show_count(
        &mut self,
        count: usize,
        axis: Axis,
        axis_shown: bool,
    ) -> RepeatFoldInput {
        self.count = clamp_count(count, self.least);
        self.axis = axis;
        self.shows = self.shows.wrapping_add(1);
        RepeatFoldInput::ShowCount {
            count,
            axis,
            axis_shown,
        }
    }

    /// Takes a change people made, unless a `show_count` sent after it has
    /// replaced it (the row handled fewer than were sent).
    pub(in crate::ui) fn take_change(&mut self, count: usize, axis: Axis, shows: u64) {
        if shows == self.shows {
            self.count = clamp_count(count, self.least);
            self.axis = axis;
        }
    }

    /// People pressed "Repeat…" ([`RepeatFoldOutput::Unfold`]).
    pub(in crate::ui) fn unfolded(&mut self) {
        self.asked = true;
    }

    /// Presses "Repeat…" as a script does: `true` when it shows, after
    /// which it counts as asked for. The owner then does what it does for
    /// [`RepeatFoldOutput::Unfold`].
    pub(in crate::ui) fn press(&mut self) -> bool {
        if self.fold != RepeatFold::Folded {
            return false;
        }
        self.asked = true;
        true
    }

    /// Forgets that "Repeat…" was pressed (something else is shown).
    pub(in crate::ui) fn forget(&mut self) {
        self.asked = false;
    }

    /// `true` once "Repeat…" was pressed for what is shown.
    pub(in crate::ui) fn asked(&self) -> bool {
        self.asked
    }

    /// `true` while the count and the direction show.
    pub(in crate::ui) fn is_open(&self) -> bool {
        self.fold == RepeatFold::Open
    }

    /// The count shown.
    pub(in crate::ui) fn count(&self) -> usize {
        self.count
    }

    /// The axis shown.
    pub(in crate::ui) fn axis(&self) -> Axis {
        self.axis
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
    pub(in crate::ui) unmeasurable: Option<String>,
}

/// The changes a selected item's card offers, as a relm4 component: its
/// model is what it shows ([`ItemToolsShown`]); its name field, Mirror
/// switch and buttons only send [`ItemToolsInput`] messages. Showing it
/// fills in the name and turns Mirror off with their handlers blocked, so
/// only people's renames and switches are reported. The repeat controls on
/// it ([`RepeatFoldComponent`]) are its owner's, placed among its rows at launch.
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
                append: &sections::row_label(&tr("Name")),

                #[name = "name"]
                gtk::EditableLabel {
                    set_hexpand: true,
                    add_css_class: "stave-edit-name",
                    add_css_class: "stave-name-field",
                    set_tooltip_text: Some(&tr("Click to retype the name: it changes everywhere it is used")),
                    update_property: &[gtk::accessible::Property::Label(&tr("Name"))],
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
                    set_tooltip_text: Some(&tr("Add its mirror image on the other side")),
                    update_property: &[gtk::accessible::Property::Label(&tr("And the other side"))],
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
                    set_label: &tr("Hold a distance…"),
                    // After the label, which adds GTK's `text-button`:
                    // these replace it, so the row keeps its width.
                    set_css_classes: &["flat"],
                    set_hexpand: true,
                    #[watch]
                    set_sensitive: model.shown.unmeasurable.is_none(),
                    #[watch]
                    set_tooltip_text: Some(&model.shown.unmeasurable.clone().unwrap_or_else(|| {
                        tr("Pick a second part or space and keep the distance to it")
                    })),
                    connect_clicked[sender] => move |_| {
                        sender.input_sender().emit(ItemToolsInput::Hold);
                    },
                },
                gtk::Button {
                    set_label: &tr("Remove…"),
                    set_css_classes: &["flat", "error"],
                    set_hexpand: true,
                    set_tooltip_text: Some(&tr("Take it out of the design (Delete)")),
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

/// The changes a selected item's card offers, as the window holds them:
/// the section they sit in (the window shows and hides it), the
/// controllers of the component that draws them and of the repeat controls
/// placed on it (launched with their outputs forwarded to the window's
/// input), and the window's own copies of what they show.
pub(in crate::ui) struct ItemTools {
    /// The panel, at the bottom of the part's card.
    pub(in crate::ui) root: gtk::Box,
    controller: relm4::Controller<ItemToolsComponent>,
    repeat: relm4::Controller<RepeatFoldComponent>,
    /// What the repeat controls show, readable at once.
    repeat_shown: RepeatFoldShown,
    /// The owner's cache of the offer for the item shown, kept by
    /// `Window::update_item_tools`.
    shown: Option<Shown>,
}

impl ItemTools {
    /// The panel, hidden; what people ask for on it goes to `input`.
    pub(in crate::ui) fn new(input: &relm4::Sender<WindowInput>) -> ItemTools {
        // A section of the Inspector, "Change" with the item as its note.
        let (root, body, title) = sections::section_noted(&tr("Change"), true);
        root.add_css_class("stave-item-tools");
        root.add_css_class("stave-edit-card");
        root.set_visible(false);
        let repeat = RepeatFoldComponent::builder()
            .launch(RepeatFoldInit {
                least: 1,
                classes: &["stave-card-row", "stave-edit-row"],
                apart: None,
            })
            .forward(input, |output| EditingInput::ItemRepeat(output).into());
        let controller = ItemToolsComponent::builder()
            .launch(ItemToolsInit {
                title,
                repeat: repeat.widget().clone(),
            })
            .forward(input, |output| EditingInput::ItemTools(output).into());
        body.append(controller.widget());
        ItemTools {
            root,
            controller,
            repeat,
            repeat_shown: RepeatFoldShown::new(1),
            shown: None,
        }
    }

    /// What its count stepper shows.
    pub(in crate::ui) fn repeat_row(&self) -> &RepeatFoldShown {
        &self.repeat_shown
    }

    /// What its repeat controls show, folded or open.
    pub(in crate::ui) fn repeat_fold(&self) -> &RepeatFoldShown {
        &self.repeat_shown
    }

    /// Queues a message for the component that draws it. It is gone only
    /// once the window is, when nobody needs to know.
    fn send(&self, message: ItemToolsInput) {
        let _ = self.controller.sender().send(message);
    }

    /// Queues a message for its repeat controls.
    fn send_repeat(&self, message: RepeatFoldInput) {
        let _ = self.repeat.sender().send(message);
    }
}

impl Window {
    /// Puts the item's changes in the Details panel, under the
    /// selection's account. (Their outputs were forwarded to the window
    /// when they were launched, `ItemTools::new`.)
    pub(in crate::ui) fn connect_item_tools(&self) {
        self.widgets.details.append(&self.editing.tools.root);
    }

    /// The item's repeat controls said something: "Repeat…" was pressed,
    /// or people changed the count or the axis.
    pub(in crate::ui) fn item_repeat_heard(&mut self, output: RepeatFoldOutput) {
        match output {
            RepeatFoldOutput::Unfold => {
                self.editing.tools.repeat_shown.unfolded();
                self.update_item_tools();
            }
            RepeatFoldOutput::Changed { count, axis, shows } => {
                self.editing
                    .tools
                    .repeat_shown
                    .take_change(count, axis, shows);
                self.repeat_selected(count, Some(axis));
            }
        }
    }

    /// Presses "Repeat…" on the selected item's card, as a click does
    /// (scripts): `false` when it doesn't show.
    pub(in crate::ui) fn press_item_repeat(&mut self) -> bool {
        if !self.editing.tools.repeat_shown.press() {
            return false;
        }
        self.send(EditingInput::ItemRepeat(RepeatFoldOutput::Unfold));
        true
    }

    /// The item's tools asked for something.
    pub(in crate::ui) fn item_tools_asked(&mut self, output: ItemToolsOutput) {
        match output {
            ItemToolsOutput::Rename(text) => self.rename_selected(&text),
            ItemToolsOutput::Mirror => self.mirror_selected(),
            ItemToolsOutput::Hold => self.hold_from_selected(),
            ItemToolsOutput::Remove => {
                self.remove_selected();
            }
        }
    }

    /// The item of the project the selection belongs to, by name.
    pub(in crate::ui) fn selected_item(&self) -> Option<String> {
        let session = &self.session;
        let (design, _) = session.drawable()?;
        selected_item(design, session.selection()?)
    }

    /// Shows the item's changes on its card when the selection is a part,
    /// a piece of hardware or an assembly and nothing is being edited.
    pub(in crate::ui) fn update_item_tools(&mut self) {
        let found = {
            let session = &self.session;
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
            self.editing.tools.root.set_visible(false);
            return;
        };
        let (stale, another_item) =
            self.editing
                .tools
                .shown
                .as_ref()
                .map_or((true, true), |shown| {
                    (
                        shown.item != item || shown.design != address,
                        shown.item != item,
                    )
                });
        if stale {
            // Another item: "Repeat…" is asked for again.
            if another_item {
                self.editing.tools.repeat_shown.forget();
            }
            // The offer reads the project, so it is worked out once per
            // item and result.
            let offer = self.session.item_offer(&item);
            self.editing.tools.shown = Some(Shown {
                item: item.clone(),
                design: address,
                offer,
            });
        }
        let tools = &mut self.editing.tools;
        let offer = tools.shown.as_ref().and_then(|shown| shown.offer.clone());
        let Some(offer) = offer else {
            tools.root.set_visible(false);
            return;
        };
        let count = offer.pieces;
        let axis = tools.repeat_shown.axis();
        let message = tools.repeat_shown.show_count(count, axis, count < 2);
        tools.send_repeat(message);
        let message = tools
            .repeat_shown
            .show(offer.repeat_fold(tools.repeat_shown.asked()));
        tools.send_repeat(message);
        let mirror = offer.mirror.as_ref().map(|gesture| match gesture {
            MirrorGesture::Image(image) => fill_in(
                &tr("And the other side · adds {image}"),
                &[("image", &image.image.as_str())],
            ),
            MirrorGesture::Split(_) => {
                tr("And the other side · splits its space across into a mirrored pair")
            }
        });
        tools.send(ItemToolsInput::Show(ItemToolsShown {
            item,
            mirror,
            unmeasurable,
        }));
        tools.root.set_visible(true);
    }

    /// Puts the keyboard in the item's name, ready to retype it.
    pub(in crate::ui) fn focus_item_name(&self) {
        let tools = &self.editing.tools;
        if tools.root.is_visible() {
            tools.send(ItemToolsInput::StartEditing);
        } else {
            self.edit_toast(&tr("Select a part or an assembly first, then rename it"));
        }
    }

    // ---- the changes, one path for the card's controls and the finder ----

    /// Renames the selected item everywhere it is used: the change is
    /// tried and shown on its card.
    pub(in crate::ui) fn rename_selected(&mut self, to: &str) {
        let Some(item) = self.selected_item() else {
            return;
        };
        if to.is_empty() || to == item {
            return;
        }
        match (parse_name(&item), parse_name(to)) {
            (Ok(item), Ok(to)) => self.start_change(EditCommand::RenameItem { item, to }),
            (_, Err(error)) => self.edit_toast(&fill_in(
                &tr("'{text}' can't be a name: {reason}"),
                &[("text", &to), ("reason", &error.0)],
            )),
            (Err(_), Ok(_)) => self.edit_toast(&fill_in(
                &tr("'{item}' can't be renamed here"),
                &[("item", &item)],
            )),
        }
    }

    /// Makes the selected item `count` pieces filling its space, split
    /// along `axis` (a first repeat needs it); on the chosen candidate's
    /// card, the candidate instead.
    pub(in crate::ui) fn repeat_selected(&mut self, count: usize, axis: Option<Axis>) {
        let chosen = self
            .session
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
            self.edit_toast(&tr("A repeat makes two or more; Remove takes it away"));
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
    pub(in crate::ui) fn mirror_selected(&mut self) {
        let Some(item) = self.change_target() else {
            return;
        };
        let Some(gesture) = self
            .session
            .item_offer(&item)
            .and_then(|offer| offer.mirror)
        else {
            self.edit_toast(&fill_in(
                &tr("{item} has no other side to add"),
                &[("item", &item)],
            ));
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
    pub(in crate::ui) fn remove_selected(&mut self) -> bool {
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

    /// Whether Delete has something to remove: a selected item, or the
    /// item the change in progress acts on.
    pub(in crate::ui) fn has_removable(&self) -> bool {
        self.change_target().is_some()
    }

    /// The item a change of the selection acts on: the selected item, or
    /// the item the change in progress acts on.
    fn change_target(&self) -> Option<String> {
        self.selected_item().or_else(|| {
            self.session
                .change()
                .and_then(|change| super::changes::change_item(change.request()))
        })
    }

    /// Starts a change of the design, reading the editor's unsaved text.
    pub(in crate::ui) fn start_change(&mut self, command: EditCommand) {
        if !self.gestures_allowed() {
            self.edit_toast(&self.parse_block_line().unwrap_or_default());
            return;
        }
        {
            let session = &mut self.session;
            session.set_edit_files(std::sync::Arc::new(self.widgets.files.edit_files()));
        }
        self.editing.state.select_after = None;
        self.send_edit_later(command);
    }

    /// The name of the item a commit of the change in progress leaves to
    /// select: the renamed or mirrored item by its new name.
    pub(in crate::ui) fn change_selects_after(&self) -> Option<String> {
        let session = &self.session;
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
            | ChangeRequest::Grip { .. }
            | ChangeRequest::Patch { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

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

    /// What a count stepper reported: (count, axis, shows).
    type RowHeard = Rc<RefCell<Vec<(usize, Axis, u64)>>>;

    /// A count stepper launched as a fold launches it, what it reports
    /// recorded.
    fn launch_row(least: usize) -> (relm4::Controller<RepeatRowComponent>, RowHeard) {
        let heard: RowHeard = Rc::default();
        let log = Rc::clone(&heard);
        let row = RepeatRowComponent::builder()
            .launch(least)
            .connect_receiver(move |_, RepeatRowOutput::Changed { count, axis, shows }| {
                log.borrow_mut().push((count, axis, shows));
            });
        (row, heard)
    }

    /// The stepper and the axis list of a count stepper's row.
    fn row_controls(row: &gtk::Box) -> (gtk::SpinButton, gtk::DropDown) {
        let row: gtk::Widget = row.clone().upcast();
        let spin = descendants::<gtk::SpinButton>(&row).first().cloned();
        let list = descendants::<gtk::DropDown>(&row).first().cloned();
        match spin.zip(list) {
            Some(controls) => controls,
            None => panic!("the row has a stepper and an axis list"),
        }
    }

    fn show_row(
        row: &relm4::Controller<RepeatRowComponent>,
        count: usize,
        axis: Axis,
        axis_shown: bool,
    ) {
        let _ = row.sender().send(RepeatRowInput::Show {
            count,
            axis,
            axis_shown,
        });
    }

    #[test]
    fn a_dropped_repeat_row_is_freed() {
        on_gtk_thread(repeat_row_is_freed);
    }

    /// Dropping the controller shuts the row down: the widget, the model
    /// and the receiver's captures are freed once the main loop runs.
    fn repeat_row_is_freed() {
        let token = Rc::new(());
        let held = Rc::clone(&token);
        let row = RepeatRowComponent::builder()
            .launch(1)
            .connect_receiver(move |_, _| {
                let _ = &held;
            });
        let callback = Rc::downgrade(&token);
        drop(token);
        settle();
        let root = row.widget().downgrade();
        let input = row.sender().clone();
        drop(row);
        settle();
        assert!(root.upgrade().is_none(), "the row is freed");
        assert!(
            input.send(RepeatRowInput::Shorten).is_err(),
            "the component's runtime, and with it the model, is gone"
        );
        assert!(callback.upgrade().is_none(), "the receiver is freed");
    }

    #[test]
    fn showing_a_count_reports_nothing() {
        on_gtk_thread(showing_a_count_reports_nothing_body);
    }

    /// The old row set a `filling` flag around its own setters; now its
    /// handlers are blocked while the view follows the model.
    fn showing_a_count_reports_nothing_body() {
        let (row, heard) = launch_row(1);
        show_row(&row, 3, Axis::Deep, false);
        let _ = row.sender().send(RepeatRowInput::Shorten);
        settle();
        let (spin, list) = row_controls(row.widget());
        assert_eq!(spin.value_as_int(), 3);
        assert_eq!(list.selected(), 2);
        assert!(!list.is_visible(), "the axis list follows the model");
        assert!(row.widget().has_css_class("stave-repeat-short"));
        assert!(heard.borrow().is_empty(), "showing it reports nothing");
    }

    #[test]
    fn a_shown_count_is_clamped_as_the_stepper_clamps_it() {
        on_gtk_thread(shown_count_is_clamped);
    }

    /// What the owner's copy says at once ([`RepeatFoldShown`], tested
    /// without the main loop below) is what the stepper holds once it has
    /// run: clamped to the least and the most.
    fn shown_count_is_clamped() {
        let (row, _) = launch_row(2);
        let mut shown = RepeatFoldShown::new(2);
        let (spin, list) = row_controls(row.widget());
        settle();
        assert_eq!(spin.value_as_int(), 2, "starts at least");
        for (count, axis) in [(3, Axis::Deep), (40, Axis::Deep), (0, Axis::Across)] {
            let _ = shown.show_count(count, axis, true);
            show_row(&row, count, axis, true);
            settle();
            assert_eq!(
                (
                    usize::try_from(spin.value_as_int()).unwrap_or(0),
                    axis_at(list.selected())
                ),
                (shown.count(), shown.axis()),
                "the stepper agrees with the copy"
            );
        }
        assert_eq!((shown.count(), shown.axis()), (2, Axis::Across));
    }

    #[test]
    fn a_change_people_made_before_a_show_does_not_outlive_it() {
        on_gtk_thread(show_replaces_an_earlier_change);
    }

    /// People change the count, and the window shows another before the
    /// main loop runs: the change is still reported (the old row called
    /// back before the show), with the shows the row had handled, so the
    /// owner's copy keeps the show; the stepper holds the show too.
    fn show_replaces_an_earlier_change() {
        let (row, heard) = launch_row(1);
        let mut shown = RepeatFoldShown::new(1);
        let (spin, _) = row_controls(row.widget());
        spin.set_value(5.0);
        let _ = shown.show_count(3, Axis::Deep, true);
        show_row(&row, 3, Axis::Deep, true);
        settle();
        assert_eq!(*heard.borrow(), vec![(5, Axis::Up, 0)]);
        shown.take_change(5, Axis::Up, 0);
        assert_eq!((shown.count(), shown.axis()), (3, Axis::Deep));
        assert_eq!(spin.value_as_int(), 3);
        // A change after the show is taken again.
        spin.set_value(6.0);
        settle();
        assert_eq!(heard.borrow().last(), Some(&(6, Axis::Deep, 1)));
        shown.take_change(6, Axis::Deep, 1);
        assert_eq!((shown.count(), shown.axis()), (6, Axis::Deep));
    }

    #[test]
    fn a_count_set_while_a_count_is_handled_is_queued() {
        on_gtk_thread(counts_are_queued);
    }

    /// The old row called back from inside the stepper's own signal. Now
    /// people's change only queues a message; it reaches the receiver
    /// after the row's update has returned, and a change made from inside
    /// the receiver waits its turn.
    fn counts_are_queued() {
        let heard: Rc<RefCell<Vec<(usize, Axis)>>> = Rc::default();
        let log = Rc::clone(&heard);
        let list_slot: Rc<RefCell<Option<gtk::DropDown>>> = Rc::default();
        let inner = Rc::clone(&list_slot);
        let row = RepeatRowComponent::builder().launch(1).connect_receiver(
            move |_, RepeatRowOutput::Changed { count, axis, .. }| {
                let first = log.borrow().is_empty();
                log.borrow_mut().push((count, axis));
                if first {
                    if let Some(list) = inner.borrow().as_ref() {
                        list.set_selected(1);
                    }
                    assert_eq!(log.borrow().len(), 1, "nothing ran under the receiver");
                }
            },
        );
        let (spin, list) = row_controls(row.widget());
        *list_slot.borrow_mut() = Some(list);
        spin.set_value(4.0);
        assert!(heard.borrow().is_empty(), "the change is only queued");
        settle();
        assert_eq!(*heard.borrow(), vec![(4, Axis::Up), (4, Axis::Across)]);
    }

    /// A fold launched as an owner launches it, its outputs recorded.
    fn launch_fold(
        apart: Option<RepeatFoldSlots>,
    ) -> (
        relm4::Controller<RepeatFoldComponent>,
        Rc<RefCell<Vec<RepeatFoldOutput>>>,
    ) {
        let heard: Rc<RefCell<Vec<RepeatFoldOutput>>> = Rc::default();
        let log = Rc::clone(&heard);
        let fold = RepeatFoldComponent::builder()
            .launch(RepeatFoldInit {
                least: 1,
                classes: &[],
                apart,
            })
            .connect_receiver(move |_, output| log.borrow_mut().push(output));
        (fold, heard)
    }

    /// The stepper and the axis list in a fold's row.
    fn fold_controls(
        fold: &relm4::Controller<RepeatFoldComponent>,
    ) -> (gtk::SpinButton, gtk::DropDown) {
        row_controls(fold.widgets().row())
    }

    #[test]
    fn a_dropped_repeat_fold_is_freed() {
        on_gtk_thread(repeat_fold_is_freed);
    }

    /// Dropping the controller shuts the fold down, and with it the row it
    /// owns: both widgets, the model and the receiver's captures go.
    fn repeat_fold_is_freed() {
        let token = Rc::new(());
        let held = Rc::clone(&token);
        let fold = RepeatFoldComponent::builder()
            .launch(RepeatFoldInit {
                least: 1,
                classes: &[],
                apart: None,
            })
            .connect_receiver(move |_, _| {
                let _ = &held;
            });
        let callback = Rc::downgrade(&token);
        drop(token);
        let _ = fold.sender().send(RepeatFoldInput::Show(RepeatFold::Open));
        settle();
        let root = fold.widget().downgrade();
        let row = fold.widgets().row().downgrade();
        let input = fold.sender().clone();
        drop(fold);
        settle();
        assert!(root.upgrade().is_none(), "the fold is freed");
        assert!(row.upgrade().is_none(), "its row is freed");
        assert!(
            input.send(RepeatFoldInput::Unfold).is_err(),
            "the component's runtime, and with it the model, is gone"
        );
        assert!(callback.upgrade().is_none(), "the receiver is freed");
    }

    #[test]
    fn a_repeat_fold_reports_only_what_people_do() {
        on_gtk_thread(repeat_fold_reports_people);
    }

    /// Showing the fold and its count reports nothing; "Repeat…" and the
    /// stepper report, the stepper with how many counts it had been shown.
    fn repeat_fold_reports_people() {
        let (fold, heard) = launch_fold(None);
        let mut shown = RepeatFoldShown::new(1);
        let send = |message| {
            let _ = fold.sender().send(message);
        };
        send(shown.show(RepeatFold::Folded));
        settle();
        let unfold = fold.widgets().unfold_button().clone();
        assert!(fold.widget().is_visible());
        assert!(unfold.is_visible() && !fold.widgets().row().is_visible());
        unfold.emit_clicked();
        assert!(heard.borrow().is_empty(), "the press is only queued");
        settle();
        assert_eq!(*heard.borrow(), vec![RepeatFoldOutput::Unfold]);
        shown.unfolded();
        assert!(shown.asked());
        send(shown.show(RepeatFold::Open));
        send(shown.show_count(3, Axis::Deep, false));
        settle();
        assert!(!unfold.is_visible() && fold.widgets().row().is_visible());
        let (spin, list) = fold_controls(&fold);
        assert_eq!(spin.value_as_int(), 3);
        assert_eq!(list.selected(), 2);
        assert!(!list.is_visible());
        assert_eq!(heard.borrow().len(), 1, "showing reported nothing");
        spin.set_value(5.0);
        settle();
        assert_eq!(
            heard.borrow().last(),
            Some(&RepeatFoldOutput::Changed {
                count: 5,
                axis: Axis::Deep,
                shows: 1
            })
        );
        shown.take_change(5, Axis::Deep, 1);
        assert_eq!((shown.count(), shown.axis()), (5, Axis::Deep));
        send(shown.show(RepeatFold::Hidden));
        settle();
        assert!(!fold.widget().is_visible());
    }

    #[test]
    fn a_repeat_fold_placed_apart_puts_its_parts_in_the_slots() {
        on_gtk_thread(repeat_fold_apart);
    }

    fn repeat_fold_apart() {
        let button = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        let row = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let (fold, _) = launch_fold(Some(RepeatFoldSlots {
            button: button.clone(),
            row: row.clone(),
        }));
        settle();
        assert!(fold.widget().first_child().is_none(), "its root is empty");
        assert_eq!(
            button.first_child().as_ref(),
            Some(fold.widgets().unfold_button().upcast_ref())
        );
        assert_eq!(
            row.first_child().as_ref(),
            Some(fold.widgets().row().upcast_ref())
        );
        let labels = descendants::<gtk::Label>(row.upcast_ref());
        assert!(
            labels.iter().any(|label| label.label() == "×"),
            "placed apart, the row is short"
        );
    }

    #[test]
    fn a_repeat_folds_classes_come_with_it() {
        on_gtk_thread(repeat_fold_classes);
    }

    /// The item's tools used to style the fold's root themselves; now the
    /// classes come in its init and it puts them on its own root.
    fn repeat_fold_classes() {
        let fold = RepeatFoldComponent::builder()
            .launch(RepeatFoldInit {
                least: 1,
                classes: &["stave-card-row", "stave-edit-row"],
                apart: None,
            })
            .detach();
        let root = fold.widget();
        assert!(root.has_css_class("stave-repeat-fold"), "its own class");
        assert!(root.has_css_class("stave-card-row") && root.has_css_class("stave-edit-row"));
    }

    #[test]
    fn a_press_made_while_a_fold_output_is_handled_is_queued() {
        on_gtk_thread(fold_presses_are_queued);
    }

    /// A change made from inside the receiver (the owner redrawing and a
    /// second press) waits its turn.
    fn fold_presses_are_queued() {
        let heard: Rc<RefCell<Vec<RepeatFoldOutput>>> = Rc::default();
        let log = Rc::clone(&heard);
        let button = Rc::new(RefCell::new(None::<gtk::Button>));
        let inner = Rc::clone(&button);
        let fold = RepeatFoldComponent::builder()
            .launch(RepeatFoldInit {
                least: 1,
                classes: &[],
                apart: None,
            })
            .connect_receiver(move |input, output| {
                let first = log.borrow().is_empty();
                log.borrow_mut().push(output);
                if first {
                    let _ = input.send(RepeatFoldInput::Show(RepeatFold::Folded));
                    if let Some(button) = inner.borrow().as_ref() {
                        button.emit_clicked();
                    }
                    assert_eq!(log.borrow().len(), 1, "nothing ran under the receiver");
                }
            });
        *button.borrow_mut() = Some(fold.widgets().unfold_button().clone());
        let _ = fold
            .sender()
            .send(RepeatFoldInput::Show(RepeatFold::Folded));
        settle();
        fold.widgets().unfold_button().emit_clicked();
        settle();
        assert_eq!(
            *heard.borrow(),
            vec![RepeatFoldOutput::Unfold, RepeatFoldOutput::Unfold]
        );
    }

    #[test]
    fn the_owners_copy_of_a_fold_follows_its_rules() {
        let mut shown = RepeatFoldShown::new(2);
        assert_eq!(
            (shown.count(), shown.axis()),
            (2, Axis::Up),
            "starts at least"
        );
        assert!(!shown.press(), "hidden: nothing to press");
        let _ = shown.show(RepeatFold::Folded);
        assert!(!shown.asked() && !shown.is_open());
        assert!(shown.press());
        assert!(shown.asked());
        let _ = shown.show(RepeatFold::Open);
        assert!(shown.is_open());
        assert!(!shown.press(), "open: nothing to press");
        shown.forget();
        assert!(!shown.asked());
        let _ = shown.show_count(40, Axis::Deep, true);
        assert_eq!(shown.count(), MOST, "clamped to the most");
        let _ = shown.show_count(0, Axis::Across, true);
        assert_eq!(
            (shown.count(), shown.axis()),
            (2, Axis::Across),
            "clamped up"
        );
        // A change people made before the last show is replaced by it.
        shown.take_change(5, Axis::Up, 1);
        assert_eq!((shown.count(), shown.axis()), (2, Axis::Across));
        shown.take_change(5, Axis::Up, 2);
        assert_eq!((shown.count(), shown.axis()), (5, Axis::Up));
    }

    #[test]
    fn dropped_item_tools_are_freed() {
        on_gtk_thread(item_tools_are_freed);
    }

    /// Dropping the controller shuts the tools down: the rows, the model
    /// and the receiver's captures go.
    fn item_tools_are_freed() {
        let token = Rc::new(());
        let held = Rc::clone(&token);
        let tools = ItemToolsComponent::builder()
            .launch(ItemToolsInit {
                title: gtk::Label::new(None),
                repeat: gtk::Box::new(gtk::Orientation::Vertical, 0),
            })
            .connect_receiver(move |_, _| {
                let _ = &held;
            });
        let callback = Rc::downgrade(&token);
        drop(token);
        let _ = tools.sender().send(ItemToolsInput::Show(ItemToolsShown {
            item: "drawer".to_owned(),
            mirror: None,
            unmeasurable: None,
        }));
        settle();
        let rows = tools.widget().downgrade();
        let input = tools.sender().clone();
        drop(tools);
        settle();
        assert!(rows.upgrade().is_none(), "the rows are freed");
        assert!(
            input.send(ItemToolsInput::Hold).is_err(),
            "the component's runtime, and with it the model, is gone"
        );
        assert!(callback.upgrade().is_none(), "the receiver is freed");
    }

    #[test]
    fn the_windows_item_tools_are_freed_with_it_and_reach_the_window() {
        on_gtk_thread(window_item_tools);
    }

    /// Launched as the window launches them (`ItemTools::new`), the tools'
    /// buttons and the repeat controls' "Repeat…" arrive as the window's
    /// editing input; dropping them frees the section, the rows and the
    /// repeat controls.
    fn window_item_tools() {
        let (input, receiver) = relm4::channel::<WindowInput>();
        let mut tools = ItemTools::new(&input);
        tools.send(ItemToolsInput::Show(ItemToolsShown {
            item: "drawer".to_owned(),
            mirror: None,
            unmeasurable: None,
        }));
        let message = tools.repeat_shown.show(RepeatFold::Folded);
        tools.send_repeat(message);
        // As the window shows it (`is_visible` asks the parents too).
        tools.root.set_visible(true);
        settle();
        let root: gtk::Widget = tools.root.clone().upcast();
        let buttons = descendants::<gtk::Button>(&root);
        let Some(remove) = buttons
            .iter()
            .find(|button| button.label().as_deref() == Some("Remove…"))
        else {
            panic!("the tools have Remove");
        };
        remove.emit_clicked();
        settle();
        assert!(matches!(
            receiver.recv_sync(),
            Some(WindowInput::Editing(EditingInput::ItemTools(
                ItemToolsOutput::Remove
            )))
        ));
        let Some(unfold) = buttons
            .iter()
            .find(|button| button.label().as_deref() == Some("Repeat…"))
        else {
            panic!("the tools have Repeat…");
        };
        assert!(unfold.is_visible(), "folded: Repeat… shows");
        unfold.emit_clicked();
        settle();
        assert!(matches!(
            receiver.recv_sync(),
            Some(WindowInput::Editing(EditingInput::ItemRepeat(
                RepeatFoldOutput::Unfold
            )))
        ));
        let section = tools.root.downgrade();
        let rows = tools.controller.widget().downgrade();
        let repeat = tools.repeat.widget().downgrade();
        let tools_input = tools.controller.sender().clone();
        let repeat_input = tools.repeat.sender().clone();
        drop((buttons, root));
        drop(tools);
        settle();
        assert!(section.upgrade().is_none(), "the section is freed");
        assert!(rows.upgrade().is_none(), "the rows are freed");
        assert!(tools_input.send(ItemToolsInput::Hold).is_err());
        assert!(repeat_input.send(RepeatFoldInput::Unfold).is_err());
        assert!(repeat.upgrade().is_none(), "the repeat controls are freed");
    }

    #[test]
    fn showing_item_tools_reports_nothing_and_a_switch_made_in_a_callback_is_queued() {
        on_gtk_thread(item_tools_are_queued);
    }

    /// The old tools kept a `filling` flag so filling them in reported
    /// nothing. Now showing them blocks their handlers, and people's input
    /// only queues a message: the receiver runs after the update, and
    /// Mirror switched on from inside it waits its turn.
    fn item_tools_are_queued() {
        let shown = ItemToolsShown {
            item: "door".to_owned(),
            mirror: Some("And the other side · adds door_right".to_owned()),
            unmeasurable: Some("pick one of its parts to measure from".to_owned()),
        };
        let heard: Rc<RefCell<Vec<ItemToolsOutput>>> = Rc::default();
        let log = Rc::clone(&heard);
        let mirror_slot: Rc<RefCell<Option<gtk::Switch>>> = Rc::default();
        let inner = Rc::clone(&mirror_slot);
        let again = shown.clone();
        let tools = ItemToolsComponent::builder()
            .launch(ItemToolsInit {
                title: gtk::Label::new(None),
                repeat: gtk::Box::new(gtk::Orientation::Vertical, 0),
            })
            .connect_receiver(move |input, output| {
                let first = log.borrow().is_empty();
                log.borrow_mut().push(output);
                if first {
                    let _ = input.send(ItemToolsInput::Show(again.clone()));
                    if let Some(mirror) = inner.borrow().as_ref() {
                        mirror.set_active(true);
                    }
                    assert_eq!(log.borrow().len(), 1, "nothing ran under the receiver");
                }
            });
        let rows: gtk::Widget = tools.widget().clone().upcast();
        let Some(mirror) = descendants::<gtk::Switch>(&rows).first().cloned() else {
            panic!("the tools have a Mirror switch");
        };
        // Switched on before anything is shown: reported, then forgotten.
        mirror.set_active(true);
        settle();
        heard.borrow_mut().clear();
        *mirror_slot.borrow_mut() = Some(mirror.clone());
        let _ = tools.sender().send(ItemToolsInput::Show(shown));
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
            "the switch made in the receiver came after it"
        );
    }
}
