//! Repeats (`docs/gui.md`, "Changing what is there"): on the card of a
//! candidate that fills a space, the count stepper and which way the
//! pieces split. The ghost shows every copy once the session has tried the
//! repeat. Nothing is dragged on the model: the boundaries between the
//! pieces of a split are rows of the Edit panel's Sizes card
//! (`sizes.rs`); here a script moves one (`EditCommand::MoveShares`),
//! settling where the session's boundary says.

use adw::prelude::*;
use glam::Vec3;
use relm4::{Component, ComponentController, Controller};
use stave_app::editing::{EditCommand, parse_name};
use stave_app::item_offer::{RepeatFold, candidate_repeat_fold};
use stave_app::spaces::{SplitBoundary, filled_spaces, split_boundaries};
use stave_lang::placement::Axis;
use stave_lang::units::Length;
use stave_model::ids::RegionId;
use stave_scene::Aabb;

use super::super::Window;
use super::item_tools::{
    RepeatFoldComponent, RepeatFoldInit, RepeatFoldInput, RepeatFoldOutput, RepeatFoldShown,
    RepeatFoldSlots,
};
use crate::ui::WindowInput;
use crate::ui::edit_card::CardSlots;
use crate::ui::edit_menu::MenuPlace;
use crate::ui::update::editing::EditingInput;

/// The unit vector of a design axis in the 3D view's frame (across is x,
/// up is y; the front faces +z, so deep, front to back, is −z).
pub(in crate::ui) fn axis_vector(axis: Axis) -> Vec3 {
    match axis {
        Axis::Across => Vec3::X,
        Axis::Up => Vec3::Y,
        Axis::Deep => Vec3::NEG_Z,
    }
}

/// The middles of a box's two faces across an axis: its start (bottom,
/// left, front) and its end.
pub(in crate::ui) fn axis_ends(volume: &Aabb, axis: Axis) -> (Vec3, Vec3) {
    let direction = axis_vector(axis);
    let centre = volume.centre();
    let half = (volume.max - volume.min) * 0.5;
    let reach = half.dot(direction.abs());
    (centre - direction * reach, centre + direction * reach)
}

/// A rectangle of the 3D view, in its pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(in crate::ui) struct Rect {
    /// Left edge.
    pub(in crate::ui) x: f32,
    /// Top edge.
    pub(in crate::ui) y: f32,
    /// Width.
    pub(in crate::ui) width: f32,
    /// Height.
    pub(in crate::ui) height: f32,
}

impl Rect {
    fn overlaps(&self, other: &Rect) -> bool {
        self.x < other.x + other.width
            && other.x < self.x + self.width
            && self.y < other.y + other.height
            && other.y < self.y + self.height
    }

    fn moved(&self, dx: f32, dy: f32) -> Rect {
        Rect {
            x: self.x + dx,
            y: self.y + dy,
            ..*self
        }
    }

    fn inside(&self, width: f32, height: f32) -> bool {
        self.x >= 0.0
            && self.y >= 0.0
            && self.x + self.width <= width
            && self.y + self.height <= height
    }
}

/// How far a label floating at `wanted` must move so it covers none of
/// the panels floating over the 3D view (the parts list, a card, the
/// dock): not at all when it is clear; else the smallest move that takes
/// it just past the edge of a panel it covers (left, right, above or
/// below) and leaves it clear of every panel and inside the view. `None`
/// when no such move is found.
pub(in crate::ui) fn clear_of(
    wanted: Rect,
    panels: &[Rect],
    view: (f32, f32),
) -> Option<(f32, f32)> {
    const GAP: f32 = 6.0;
    let clear =
        |rect: &Rect| rect.inside(view.0, view.1) && !panels.iter().any(|p| p.overlaps(rect));
    if clear(&wanted) {
        return Some((0.0, 0.0));
    }
    let mut moves: Vec<(f32, f32)> = panels
        .iter()
        .filter(|panel| panel.overlaps(&wanted))
        .flat_map(|panel| {
            [
                (panel.x - GAP - (wanted.x + wanted.width), 0.0),
                (panel.x + panel.width + GAP - wanted.x, 0.0),
                (0.0, panel.y - GAP - (wanted.y + wanted.height)),
                (0.0, panel.y + panel.height + GAP - wanted.y),
            ]
        })
        .collect();
    moves.sort_by(|a, b| (a.0.abs() + a.1.abs()).total_cmp(&(b.0.abs() + b.1.abs())));
    moves
        .into_iter()
        .find(|(dx, dy)| clear(&wanted.moved(*dx, *dy)))
}

/// A boundary between two neighbouring pieces of a split, as drawn: the
/// session's boundary ([`SplitBoundary`]: the region array, which pieces,
/// their sizes and where a drag settles) and where it is on the screen.
#[derive(Debug, Clone, PartialEq)]
pub(in crate::ui) struct Boundary {
    /// The boundary, from the design.
    pub(in crate::ui) split: SplitBoundary,
    /// The start of the lower piece, the boundary, and the end of the
    /// upper piece, in the 3D view's frame, on the pieces' front face.
    pub(in crate::ui) points: (Vec3, Vec3, Vec3),
}

/// Where the boundaries of a split are drawn: each piece's start and end
/// along the split, on the front of the space.
fn drawn(split: SplitBoundary, low: &Aabb, high: &Aabb) -> Boundary {
    let front = |point: Vec3, volume: &Aabb| match split.axis {
        Axis::Across | Axis::Up => Vec3::new(point.x, point.y, volume.max.z),
        Axis::Deep => point,
    };
    let (start, middle) = axis_ends(low, split.axis);
    let (_, end) = axis_ends(high, split.axis);
    let union = low.union(*high);
    Boundary {
        points: (
            front(start, &union),
            front(middle, &union),
            front(end, &union),
        ),
        split,
    }
}

/// The repeat controls: the candidate card's stepper.
pub(in crate::ui) struct Repeats {
    /// On the chosen candidate's card, folded until asked for; its
    /// outputs come to the window as [`EditingInput::CardRepeatUnfolded`]
    /// and [`EditingInput::CardRepeatChanged`].
    row: Controller<RepeatFoldComponent>,
    /// What the window last showed on it (and people last chose), read at
    /// once by the count handle and scripts.
    pub(in crate::ui) shown: RepeatFoldShown,
}

impl Repeats {
    /// Hidden controls; the stepper sits on the candidate's card, above
    /// its buttons (`slots`), and reports to `input`.
    pub(in crate::ui) fn new(input: &relm4::Sender<WindowInput>, slots: &CardSlots) -> Repeats {
        let row = RepeatFoldComponent::builder()
            .launch(RepeatFoldInit {
                least: 1,
                classes: &[],
                apart: Some(RepeatFoldSlots {
                    button: slots.repeat_button_slot.clone(),
                    row: slots.repeat_slot.clone(),
                }),
            })
            .forward(input, |output| {
                match output {
                    RepeatFoldOutput::Unfold => EditingInput::CardRepeatUnfolded,
                    RepeatFoldOutput::Changed { count, axis, shows } => {
                        EditingInput::CardRepeatChanged { count, axis, shows }
                    }
                }
                .into()
            });
        Repeats {
            row,
            shown: RepeatFoldShown::new(1),
        }
    }
}

impl Repeats {
    /// Sends the stepper a message, as [`RepeatFoldShown`]'s setters
    /// return them.
    fn send(&self, message: RepeatFoldInput) {
        // After shutdown there is nothing to show.
        let _ = self.row.sender().send(message);
    }
}

impl Window {
    /// The spaces the item of the project by this name fills, as the
    /// session's model binds them ([`filled_spaces`]): one per piece.
    fn spaces_of(&self, item: &str) -> Vec<RegionId> {
        let session = &self.session;
        session
            .shown()
            .and_then(|checked| checked.opened.model.as_ref())
            .map(|model| filled_spaces(model, item))
            .unwrap_or_default()
    }

    /// The panels floating over the 3D view, in its pixels, that a handle
    /// or a label must not cover: the view's own controls (the scene menu,
    /// the camera column and Pull apart's slider, the moving assemblies'
    /// chips, the motion slider, the line and the key at the top, the
    /// cards and the chip at the foot), where each was last drawn (their
    /// roots' own bounds; the floats place them), and the editing's menu
    /// (its cards are in the Edit panel), where it last said it placed
    /// itself ([`EditMenuOutput::Placed`]): it moves itself, so its
    /// drawn bounds lag a move made since.
    ///
    /// [`EditMenuOutput::Placed`]: crate::ui::edit_menu::EditMenuOutput::Placed
    pub(in crate::ui) fn floating_panels(&self) -> Vec<Rect> {
        let design = &self.widgets.design;
        let editing = &self.editing;
        let widgets = &self.widgets;
        let menu = editing
            .menu_place
            .filter(|_| editing.menu.widget().is_visible())
            .map(|place| menu_rect(place, design.height()));
        [
            widgets.dock.scene.upcast_ref::<gtk::Widget>(),
            widgets.dock.moves.upcast_ref(),
            widgets.camera.root.upcast_ref(),
            widgets.view_toolbar.popout.upcast_ref(),
            widgets.motion.root.upcast_ref(),
            widgets.hint_box.upcast_ref(),
            widgets.block_box.upcast_ref(),
            widgets.contact_key.upcast_ref(),
            widgets.plan_switch.root.upcast_ref(),
            widgets.step_card.root.upcast_ref(),
            widgets.problem_card.root.upcast_ref(),
            widgets.edge_pin.upcast_ref(),
        ]
        .into_iter()
        .filter(|widget| widget.is_visible())
        .filter_map(|widget| drawn_rect(widget, design))
        .chain(menu)
        .collect()
    }

    /// Where a floating label of this size, wanted with its top left at
    /// `wanted`, goes: moved clear of the panels when it would cover one.
    /// The move made, or `None` when it can't be cleared.
    pub(in crate::ui) fn clear_spot(
        &self,
        wanted: (f32, f32),
        size: (f32, f32),
    ) -> Option<(f32, f32)> {
        let view = (
            crate::convert::f32_of_int(self.widgets.design.width()),
            crate::convert::f32_of_int(self.widgets.design.height()),
        );
        let rect = Rect {
            x: wanted.0,
            y: wanted.1,
            width: size.0,
            height: size.1,
        };
        clear_of(rect, &self.floating_panels(), view)
    }

    /// Shows the candidate card's repeat controls as the rule says
    /// ([`candidate_repeat_fold`]): hidden unless the candidate fills a
    /// space; folded behind "Repeat…" until asked for or already more than
    /// one; else the count and the direction, with the repeat the session
    /// holds.
    pub(in crate::ui) fn update_repeat_row(&mut self) {
        let asked = self.editing.repeats.shown.asked();
        let shown = {
            let session = &self.session;
            session.edit().and_then(|edit| edit.chosen()).map(|chosen| {
                let repeat = chosen.repeat();
                (
                    candidate_repeat_fold(chosen.candidate(), repeat, asked),
                    repeat.map_or((1, None), |repeat| (repeat.count.get(), Some(repeat.axis))),
                )
            })
        };
        let repeats = &mut self.editing.repeats;
        match shown {
            Some((fold, (count, axis))) => {
                let axis = axis.unwrap_or_else(|| repeats.shown.axis());
                let message = repeats.shown.show_count(count, axis, true);
                repeats.send(message);
                let message = repeats.shown.show(fold);
                repeats.send(message);
            }
            None => {
                let message = repeats.shown.show(RepeatFold::Hidden);
                repeats.send(message);
            }
        }
    }

    /// "Repeat…" on the candidate's card was pressed: the count and the
    /// direction show.
    pub(in crate::ui) fn card_repeat_unfolded(&mut self) {
        self.editing.repeats.shown.unfolded();
        self.update_repeat_row();
        self.request_render();
    }

    /// People changed the count or the direction on the candidate's card.
    pub(in crate::ui) fn card_repeat_changed(&mut self, count: usize, axis: Axis, shows: u64) {
        self.editing.repeats.shown.take_change(count, axis, shows);
        self.repeat_selected(count, Some(axis));
    }

    /// Forgets that "Repeat…" was pressed on the candidate's card
    /// (another candidate, or another edit).
    pub(in crate::ui) fn forget_card_repeat(&mut self) {
        self.editing.repeats.shown.forget();
    }

    /// Sets the count to `count` pieces on the stepper, "Repeat…" pressed
    /// first when it is folded, as a person would (scripts).
    pub(in crate::ui) fn drag_count_to(&mut self, count: usize) -> Result<(), String> {
        self.unfold_repeat();
        let chosen = self.session.edit().and_then(|edit| edit.chosen()).is_some();
        let axis = if chosen {
            self.editing.repeats.shown.axis()
        } else if self.editing.tools.root.is_visible() {
            self.editing.tools.repeat_row().axis()
        } else {
            return Err("nothing to repeat: choose something that fills a space".to_owned());
        };
        self.repeat_selected(count, Some(axis));
        Ok(())
    }

    /// Presses "Repeat…" on the candidate's card, or else on the selected
    /// item's, when it is folded; `true` when one was pressed.
    pub(in crate::ui) fn unfold_repeat(&mut self) -> bool {
        if self.editing.repeats.shown.press() {
            // What a click on it does, once this step is done.
            self.send(EditingInput::CardRepeatUnfolded);
            return true;
        }
        self.press_item_repeat()
    }

    // ---- the boundaries between pieces ----

    /// The boundaries shown for the selection: of the split the selected
    /// item's space is a piece of, while nothing is being edited.
    fn shown_boundaries(&self) -> Vec<Boundary> {
        {
            let session = &self.session;
            if session.edit().is_some()
                || session.change().is_some()
                || session.selection().is_none()
            {
                return Vec::new();
            }
        }
        self.boundaries_of_selection()
    }

    fn boundaries_of_selection(&self) -> Vec<Boundary> {
        let Some(item) = self.selected_item() else {
            return Vec::new();
        };
        let Some(space) = self.spaces_of(&item).first().copied() else {
            return Vec::new();
        };
        let session = &self.session;
        let state = &self.state;
        let (Some(checked), Some((_, graph)), Some(posed)) =
            (session.shown(), session.drawable(), state.posed.as_ref())
        else {
            return Vec::new();
        };
        let (Some(model), Some(design)) = (checked.opened.model.as_ref(), checked.design()) else {
            return Vec::new();
        };
        split_boundaries(model, design, space)
            .into_iter()
            .filter_map(|split| {
                let low = graph.region_volume(posed, split.low)?;
                let high = graph.region_volume(posed, split.high)?;
                Some(drawn(split, &low, &high))
            })
            .collect()
    }

    /// Moves a boundary between two pieces so the lower is `length` long.
    pub(in crate::ui) fn move_boundary(&mut self, boundary: &Boundary, length: Length) {
        match parse_name(&boundary.split.region) {
            Ok(region) => self.start_change(EditCommand::MoveShares {
                region,
                lower: boundary.split.lower,
                boundary: length,
            }),
            Err(error) => self.edit_toast(&error.0),
        }
    }

    /// The boundaries the selection shows (scripts).
    pub(in crate::ui) fn boundaries_shown(&self) -> Vec<Boundary> {
        self.shown_boundaries()
    }
}

/// Where a panel floating over the 3D view was last drawn (its own
/// bounds in the view), while it shows.
fn drawn_rect(widget: &gtk::Widget, design: &gtk::Overlay) -> Option<Rect> {
    let bounds = widget
        .is_mapped()
        .then(|| widget.compute_bounds(design))
        .flatten()?;
    Some(Rect {
        x: bounds.x(),
        y: bounds.y(),
        width: bounds.width(),
        height: bounds.height(),
    })
}

/// Where the menu stands, as it said ([`MenuPlace`]), in a view this
/// high: no lower than the view's foot.
fn menu_rect(place: MenuPlace, view_height: i32) -> Rect {
    let room = view_height.saturating_sub(place.top).max(0);
    Rect {
        x: crate::convert::f32_of_int(place.start),
        y: crate::convert::f32_of_int(place.top),
        width: crate::convert::f32_of_int(place.width),
        height: crate::convert::f32_of_int(place.height.min(room)),
    }
}

/// A widget's natural size, for placing it before it is drawn. A hidden
/// widget measures nothing: show it first.
pub(in crate::ui) fn natural_size(widget: &gtk::Widget) -> (f32, f32) {
    let (_, width, _, _) = widget.measure(gtk::Orientation::Horizontal, -1);
    let (_, height, _, _) = widget.measure(gtk::Orientation::Vertical, width);
    (
        crate::convert::f32_of_int(width),
        crate::convert::f32_of_int(height),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: f32, y: f32, width: f32, height: f32) -> Rect {
        Rect {
            x,
            y,
            width,
            height,
        }
    }

    #[test]
    fn a_label_moves_clear_of_the_panels() {
        let view = (800.0, 600.0);
        let parts_list = rect(14.0, 64.0, 365.0, 200.0);
        let card = rect(330.0, 64.0, 350.0, 560.0);
        // Clear already: it stays.
        assert_eq!(
            clear_of(rect(420.0, 300.0, 60.0, 28.0), &[parts_list], view),
            Some((0.0, 0.0))
        );
        // On the parts list, near its right edge: just past it.
        assert_eq!(
            clear_of(rect(110.0, 180.0, 60.0, 28.0), &[parts_list], view),
            Some((0.0, 90.0))
        );
        // On the card's left edge: out to its left.
        let moved = clear_of(rect(320.0, 360.0, 60.0, 28.0), &[card], view).unwrap();
        assert_eq!(moved, (-56.0, 0.0));
        // Nowhere to go: none.
        assert_eq!(
            clear_of(
                rect(10.0, 10.0, 60.0, 28.0),
                &[rect(0.0, 0.0, 800.0, 600.0)],
                view
            ),
            None
        );
    }

    #[test]
    fn the_ends_of_a_box_along_an_axis() {
        let volume = Aabb {
            min: Vec3::new(0.0, 0.0, -4.0),
            max: Vec3::new(2.0, 6.0, 0.0),
        };
        let (bottom, top) = axis_ends(&volume, Axis::Up);
        assert_eq!(bottom, Vec3::new(1.0, 0.0, -2.0));
        assert_eq!(top, Vec3::new(1.0, 6.0, -2.0));
        let (front, back) = axis_ends(&volume, Axis::Deep);
        assert_eq!(front, Vec3::new(1.0, 3.0, 0.0));
        assert_eq!(back, Vec3::new(1.0, 3.0, -4.0));
    }
}
