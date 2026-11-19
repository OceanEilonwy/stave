//! Repeat by dragging (`docs/gui.md`, "Changing what is there"): on the
//! card of a candidate that fills a space, the count stepper and which way
//! the pieces split; in the 3D view, a handle at the top (or side) of the
//! first piece, which, dragged toward the start of the space, makes more
//! pieces (each the size the handle marks) and, dragged back, fewer; its
//! label says how many. The ghost shows every copy once the session has
//! tried the repeat. Between two neighbouring pieces of a split, a
//! boundary handle moves the boundary (`EditCommand::MoveShares`),
//! snapping to an even division and to round sizes.
//!
//! ```text
//!   ┌──────────┐ ← ⇕ × 1   drag down ─▶ × 2, × 3 …
//!   │  space   │
//!   └──────────┘
//!   piece a ══ ⇕ 180 mm ══ piece b   (a boundary handle)
//! ```

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use glam::Vec3;
use stave_app::edit::PlaceSlot;
use stave_app::editing::{EditCommand, parse_name};
use stave_app::item_offer::{RepeatFold, candidate_repeat_fold};
use stave_app::spaces::{SplitBoundary, filled_spaces, split_boundaries};
use stave_lang::placement::Axis;
use stave_lang::units::Length;
use stave_model::ids::RegionId;
use stave_scene::Aabb;

use super::super::Window;
use super::item_tools::{MOST, RepeatFoldRow};

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

/// A point moved to a box's left edge (or, for a split across, its
/// bottom edge), at its front: where a handle on it reads clear of the
/// card, which stands to the right.
pub(in crate::ui) fn at_edge(point: Vec3, volume: &Aabb, axis: Axis) -> Vec3 {
    let mut moved = point;
    match axis {
        Axis::Across => moved.y = volume.min.y,
        Axis::Up | Axis::Deep => moved.x = volume.min.x,
    }
    moved.z = volume.max.z;
    moved
}

/// How far along a segment of the screen a point is: 0 at its start, 1 at
/// its end (not clamped).
pub(in crate::ui) fn fraction_along(point: (f32, f32), start: (f32, f32), end: (f32, f32)) -> f32 {
    let (dx, dy) = (end.0 - start.0, end.1 - start.1);
    let length = dx * dx + dy * dy;
    if length <= f32::EPSILON {
        return 0.0;
    }
    ((point.0 - start.0) * dx + (point.1 - start.1) * dy) / length
}

/// The count a count handle dragged to `fraction` of the space asks for:
/// the first piece reaches the handle, and the copies fill the rest.
pub(in crate::ui) fn count_at(fraction: f32) -> usize {
    // The count whose first piece ends nearest the handle.
    (1..=MOST)
        .min_by(|a, b| {
            let off = |count: usize| (1.0 / crate::convert::f32_of_index(count) - fraction).abs();
            off(*a).total_cmp(&off(*b))
        })
        .unwrap_or(1)
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

/// A handle floating over the 3D view, dragged with the pointer.
pub(in crate::ui) struct Handle {
    /// The pill.
    pub(in crate::ui) root: gtk::Box,
    label: gtk::Label,
    /// How far it was moved off its point to keep clear of the panels;
    /// a drag subtracts it, so the handle still measures from its point.
    nudge: Cell<(f32, f32)>,
}

impl Handle {
    fn new(tooltip: &str) -> Handle {
        let label = gtk::Label::new(None);
        let root = gtk::Box::builder()
            .spacing(4)
            .halign(gtk::Align::Start)
            .valign(gtk::Align::Start)
            .css_classes(["stave-repeat-handle"])
            .tooltip_text(tooltip)
            .visible(false)
            .build();
        root.append(&gtk::Image::from_icon_name("view-more-horizontal-symbolic"));
        root.append(&label);
        root.set_cursor_from_name(Some("grab"));
        Handle {
            root,
            label,
            nudge: Cell::new((0.0, 0.0)),
        }
    }

    /// Where a drag that began `at` (in the handle) began, measured from
    /// the handle's point rather than where it was moved to.
    fn origin(&self, (x, y): (f64, f64)) -> (f32, f32) {
        let (dx, dy) = self.nudge.get();
        (
            crate::convert::f32_of_int(self.root.margin_start()) + crate::convert::f32_of(x) - dx,
            crate::convert::f32_of_int(self.root.margin_top()) + crate::convert::f32_of(y) - dy,
        )
    }
}

/// What a drag on a handle started from.
#[derive(Debug, Clone, Copy)]
struct DragFrom {
    /// The screen points the handle's fraction is measured between.
    start: (f32, f32),
    end: (f32, f32),
    /// Where the drag began, in the 3D view.
    origin: (f32, f32),
}

/// The repeat controls: the candidate card's stepper, the count handle and
/// the boundary handles.
pub(in crate::ui) struct Repeats {
    /// On the chosen candidate's card, folded until asked for.
    pub(in crate::ui) row: RepeatFoldRow,
    count: Handle,
    boundaries: RefCell<Vec<(Handle, Option<Boundary>)>>,
    /// What the count handle is on: the space's box and the axis, while
    /// it shows.
    count_on: Cell<Option<(Aabb, Axis, usize)>>,
    drag: Cell<Option<DragFrom>>,
    /// The count a drag on the count handle has reached.
    dragged_count: Cell<Option<usize>>,
    /// A boundary drag: which handle, and the boundary it has reached.
    dragged_boundary: Cell<Option<(usize, Length)>>,
}

impl Repeats {
    /// Hidden controls.
    pub(in crate::ui) fn new() -> Repeats {
        Repeats {
            row: RepeatFoldRow::new(1),
            count: Handle::new(
                "Drag toward the start of the space for more pieces, back for fewer",
            ),
            boundaries: RefCell::new(Vec::new()),
            count_on: Cell::new(None),
            drag: Cell::new(None),
            dragged_count: Cell::new(None),
            dragged_boundary: Cell::new(None),
        }
    }
}

impl Window {
    /// The spaces the item of the project by this name fills, as the
    /// session's model binds them ([`filled_spaces`]): one per piece.
    fn spaces_of(&self, item: &str) -> Vec<RegionId> {
        let session = self.session.borrow();
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
    /// cards and the chip at the foot) and the editing's menu (its cards
    /// are in the Edit panel).
    pub(in crate::ui) fn floating_panels(&self) -> Vec<Rect> {
        let design = &self.widgets.design;
        let editing = &self.editing;
        let widgets = &self.widgets;
        [
            widgets.dock.scene.upcast_ref::<gtk::Widget>(),
            widgets.dock.moves.upcast_ref(),
            widgets.camera.root.upcast_ref(),
            widgets.view_toolbar.popout.upcast_ref(),
            widgets.motion.root.upcast_ref(),
            widgets.hint_box.upcast_ref(),
            widgets.contact_key.upcast_ref(),
            widgets.plan_switch.root.upcast_ref(),
            widgets.step_card.root.upcast_ref(),
            widgets.problem_card.root.upcast_ref(),
            widgets.edge_pin.upcast_ref(),
            editing.menu.root.upcast_ref(),
        ]
        .into_iter()
        .filter(|widget| widget.is_visible())
        .flat_map(|widget| panel_rects(widget, design))
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

    /// Places a handle at a point of the screen, moved clear of the
    /// panels.
    fn place_handle(&self, handle: &Handle, (x, y): (f32, f32)) {
        // A hidden widget measures nothing.
        handle.root.set_visible(true);
        let size = natural_size(handle.root.upcast_ref());
        let wanted = ((x - 30.0).max(0.0), (y - 14.0).max(0.0));
        let nudge = self.clear_spot(wanted, size).unwrap_or((0.0, 0.0));
        handle.nudge.set(nudge);
        handle.root.set_margin_start(crate::convert::int_of_f32(
            (wanted.0 + nudge.0).max(0.0).round(),
        ));
        handle.root.set_margin_top(crate::convert::int_of_f32(
            (wanted.1 + nudge.1).max(0.0).round(),
        ));
        handle.root.set_visible(true);
    }

    /// Puts the repeat controls on the card and over the 3D view.
    pub(in crate::ui) fn connect_repeats(self: &Rc<Self>) {
        let repeats = &self.editing.repeats;
        // The repeat controls sit on the candidate's card, above its
        // buttons, folded until asked for.
        repeats.row.place_apart(
            &self.editing.card.repeat_button_slot,
            &self.editing.card.repeat_slot,
        );
        let window = Rc::downgrade(self);
        repeats.row.connect_unfold(move || {
            if let Some(window) = window.upgrade() {
                window.update_repeat_row();
                window.request_render();
            }
        });
        let window = Rc::downgrade(self);
        repeats.row.row.connect_changed(move |count, axis| {
            if let Some(window) = window.upgrade() {
                window.repeat_selected(count, Some(axis));
            }
        });
        self.widgets.design.add_overlay(&repeats.count.root);
        let drag = gtk::GestureDrag::new();
        let window = Rc::downgrade(self);
        drag.connect_drag_begin(move |_, x, y| {
            if let Some(window) = window.upgrade() {
                window.count_drag_begin(x, y);
            }
        });
        let window = Rc::downgrade(self);
        drag.connect_drag_update(move |_, x, y| {
            if let Some(window) = window.upgrade() {
                window.count_drag_update(x, y);
            }
        });
        let window = Rc::downgrade(self);
        drag.connect_drag_end(move |_, _, _| {
            if let Some(window) = window.upgrade() {
                window.count_drag_end();
            }
        });
        repeats.count.root.add_controller(drag);
    }

    /// Shows the candidate card's repeat controls as the rule says
    /// ([`candidate_repeat_fold`]): hidden unless the candidate fills a
    /// space; folded behind "Repeat…" until asked for or already more than
    /// one; else the count and the direction, with the repeat the session
    /// holds.
    pub(in crate::ui) fn update_repeat_row(&self) {
        let row = &self.editing.repeats.row;
        let shown = {
            let session = self.session.borrow();
            session.edit().and_then(|edit| edit.chosen()).map(|chosen| {
                let repeat = chosen.repeat();
                (
                    candidate_repeat_fold(chosen.candidate(), repeat, row.asked()),
                    repeat.map_or((1, None), |repeat| (repeat.count.get(), Some(repeat.axis))),
                )
            })
        };
        match shown {
            Some((fold, (count, axis))) => {
                row.row
                    .show(count, axis.unwrap_or_else(|| row.row.axis()), true);
                row.show(fold);
            }
            None => row.show(RepeatFold::Hidden),
        }
    }

    /// The space the count handle stands on, the axis and the count: the
    /// candidate's space while its card shows the stepper; the space the
    /// selected item fills while its card offers a first repeat.
    fn count_handle_on(&self) -> Option<(Aabb, Axis, usize)> {
        let item = {
            let session = self.session.borrow();
            let state = self.state.borrow();
            let (_, graph) = session.drawable()?;
            let posed = state.posed.as_ref()?;
            if let Some(chosen) = session.edit().and_then(|edit| edit.chosen()) {
                // Only while the repeat controls are open: folded, they
                // take no room on the model either.
                if !self.editing.repeats.row.is_open() {
                    return None;
                }
                let region =
                    chosen
                        .candidate()
                        .place
                        .iter()
                        .find_map(|param| match param.slot {
                            PlaceSlot::Region(Some(region)) => Some(region),
                            PlaceSlot::Region(None)
                            | PlaceSlot::Anchor(_)
                            | PlaceSlot::Part(_)
                            | PlaceSlot::Parts(_) => None,
                        })?;
                let volume = graph.region_volume(posed, region)?;
                let repeats = &self.editing.repeats;
                let count = repeats
                    .dragged_count
                    .get()
                    .unwrap_or_else(|| repeats.row.row.count());
                return Some((volume, repeats.row.row.axis(), count));
            }
            if session.edit().is_some() || session.change().is_some() {
                return None;
            }
            if !self.editing.tools.root.is_visible() || !self.editing.tools.repeat_fold().is_open()
            {
                return None;
            }
            let (design, _) = session.drawable()?;
            let item = super::item_tools::selected_item(design, session.selection()?)?;
            if super::item_tools::piece_count(design, &item) > 1 {
                return None;
            }
            item
        };
        // The one space it fills, as the model binds it.
        let [space] = self.spaces_of(&item)[..] else {
            return None;
        };
        let session = self.session.borrow();
        let state = self.state.borrow();
        let (_, graph) = session.drawable()?;
        let volume = graph.region_volume(state.posed.as_ref()?, space)?;
        let count = self.editing.repeats.dragged_count.get().unwrap_or(1);
        Some((volume, self.editing.tools.repeat_row().axis(), count))
    }

    /// Places the repeat handles after a frame is drawn.
    pub(in crate::ui) fn place_repeat_handles(self: &Rc<Self>) {
        let repeats = &self.editing.repeats;
        let focus = self.state.borrow().focus.is_some();
        let on = (!focus && self.gestures_allowed())
            .then(|| self.count_handle_on())
            .flatten();
        repeats.count_on.set(on);
        match on {
            Some((volume, axis, count)) => {
                let (start, end) = axis_ends(&volume, axis);
                let (start, end) = (at_edge(start, &volume, axis), at_edge(end, &volume, axis));
                let at = start + (end - start) / crate::convert::f32_of_index(count.max(1));
                let point = {
                    let state = self.state.borrow();
                    self.project(&state, at)
                };
                match point {
                    Some(point) => {
                        repeats.count.label.set_label(&format!("× {count}"));
                        self.place_handle(&repeats.count, point);
                    }
                    None => repeats.count.root.set_visible(false),
                }
            }
            None => repeats.count.root.set_visible(false),
        }
        self.place_boundary_handles(focus);
    }

    fn count_drag_begin(&self, x: f64, y: f64) {
        let repeats = &self.editing.repeats;
        let Some((volume, axis, _)) = repeats.count_on.get() else {
            return;
        };
        let (start, end) = axis_ends(&volume, axis);
        let (start, end) = (at_edge(start, &volume, axis), at_edge(end, &volume, axis));
        let state = self.state.borrow();
        let (Some(start), Some(end)) = (self.project(&state, start), self.project(&state, end))
        else {
            return;
        };
        let origin = repeats.count.origin((x, y));
        repeats.drag.set(Some(DragFrom { start, end, origin }));
    }

    fn count_drag_update(self: &Rc<Self>, x: f64, y: f64) {
        let repeats = &self.editing.repeats;
        let Some(from) = repeats.drag.get() else {
            return;
        };
        let point = (
            from.origin.0 + crate::convert::f32_of(x),
            from.origin.1 + crate::convert::f32_of(y),
        );
        let count = count_at(fraction_along(point, from.start, from.end));
        if repeats.dragged_count.get() != Some(count) {
            repeats.dragged_count.set(Some(count));
            repeats.count.label.set_label(&format!("× {count}"));
            self.request_render();
        }
    }

    fn count_drag_end(self: &Rc<Self>) {
        let repeats = &self.editing.repeats;
        repeats.drag.set(None);
        let Some(count) = repeats.dragged_count.take() else {
            return;
        };
        let axis = repeats.count_on.get().map(|(_, axis, _)| axis);
        self.repeat_selected(count, axis);
    }

    /// Drags the count handle to `count` pieces, as a drag that ends there
    /// does (scripts).
    pub(in crate::ui) fn drag_count_to(self: &Rc<Self>, count: usize) -> Result<(), String> {
        // The handle shows once the repeat controls are open: "Repeat…"
        // is pressed first, as a person would.
        self.unfold_repeat();
        let Some((_, axis, _)) = self.count_handle_on() else {
            return Err("no count handle shows: choose something that fills a space".to_owned());
        };
        self.editing.repeats.dragged_count.set(Some(count));
        self.place_repeat_handles();
        self.editing.repeats.dragged_count.set(None);
        self.repeat_selected(count, Some(axis));
        Ok(())
    }

    /// Presses "Repeat…" on the candidate's card, or else on the selected
    /// item's, when it is folded; `true` when one was pressed.
    pub(in crate::ui) fn unfold_repeat(self: &Rc<Self>) -> bool {
        if self.editing.repeats.row.press() {
            return true;
        }
        self.editing.tools.repeat_fold().press()
    }

    // ---- the boundaries between pieces ----

    /// The boundaries shown for the selection: of the split the selected
    /// item's space is a piece of, while nothing is being edited.
    fn shown_boundaries(&self) -> Vec<Boundary> {
        {
            let session = self.session.borrow();
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
        let session = self.session.borrow();
        let state = self.state.borrow();
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

    fn place_boundary_handles(self: &Rc<Self>, focus: bool) {
        let shown = if focus || !self.gestures_allowed() {
            Vec::new()
        } else {
            self.shown_boundaries()
        };
        let repeats = &self.editing.repeats;
        let dragging = repeats.dragged_boundary.get();
        let mut handles = repeats.boundaries.borrow_mut();
        while handles.len() < shown.len() {
            let handle =
                Handle::new("Drag to move the boundary: only these two pieces change size");
            handle.root.add_css_class("boundary");
            self.widgets.design.add_overlay(&handle.root);
            let index = handles.len();
            self.connect_boundary_drag(&handle.root, index);
            handles.push((handle, None));
        }
        for (index, (handle, boundary)) in handles.iter_mut().enumerate() {
            let Some(found) = shown.get(index) else {
                handle.root.set_visible(false);
                *boundary = None;
                continue;
            };
            *boundary = Some(found.clone());
            let lower = match dragging {
                Some((dragged, length)) if dragged == index => length,
                _ => found.split.sizes.0,
            };
            let total = found.split.pair();
            let fraction = if total.nm() > 0 {
                lower.to_mm_f64() / total.to_mm_f64()
            } else {
                0.5
            };
            let (start, _, end) = found.points;
            let at = start + (end - start) * crate::convert::f32_of(fraction);
            let point = {
                let state = self.state.borrow();
                self.project(&state, at)
            };
            handle
                .label
                .set_label(&super::hold::length_text(lower).replace("mm", " mm"));
            match point {
                Some(point) => self.place_handle(handle, point),
                None => handle.root.set_visible(false),
            }
        }
    }

    /// The boundary the handle at `index` is on.
    fn boundary_of_handle(&self, index: usize) -> Option<Boundary> {
        self.editing
            .repeats
            .boundaries
            .borrow()
            .get(index)
            .and_then(|(_, boundary)| boundary.clone())
    }

    fn connect_boundary_drag(self: &Rc<Self>, root: &gtk::Box, index: usize) {
        let drag = gtk::GestureDrag::new();
        let window = Rc::downgrade(self);
        drag.connect_drag_begin(move |_, x, y| {
            let Some(window) = window.upgrade() else {
                return;
            };
            let Some(boundary) = window.boundary_of_handle(index) else {
                return;
            };
            let state = window.state.borrow();
            let (start, _, end) = boundary.points;
            let (Some(start), Some(end)) =
                (window.project(&state, start), window.project(&state, end))
            else {
                return;
            };
            let repeats = &window.editing.repeats;
            let origin = repeats
                .boundaries
                .borrow()
                .get(index)
                .map(|(handle, _)| handle.origin((x, y)));
            if let Some(origin) = origin {
                repeats.drag.set(Some(DragFrom { start, end, origin }));
            }
        });
        let window = Rc::downgrade(self);
        drag.connect_drag_update(move |_, x, y| {
            let Some(window) = window.upgrade() else {
                return;
            };
            let repeats = &window.editing.repeats;
            let Some(from) = repeats.drag.get() else {
                return;
            };
            let Some(boundary) = window.boundary_of_handle(index) else {
                return;
            };
            let point = (
                from.origin.0 + crate::convert::f32_of(x),
                from.origin.1 + crate::convert::f32_of(y),
            );
            let length = boundary_at(&boundary, fraction_along(point, from.start, from.end));
            repeats.dragged_boundary.set(Some((index, length)));
            window.request_render();
        });
        let window = Rc::downgrade(self);
        drag.connect_drag_end(move |_, _, _| {
            let Some(window) = window.upgrade() else {
                return;
            };
            let repeats = &window.editing.repeats;
            repeats.drag.set(None);
            let Some((dragged, length)) = repeats.dragged_boundary.take() else {
                return;
            };
            if let Some(boundary) = window.boundary_of_handle(dragged) {
                window.move_boundary(&boundary, length);
            }
        });
        root.add_controller(drag);
    }

    /// Moves a boundary between two pieces so the lower is `length` long.
    pub(in crate::ui) fn move_boundary(self: &Rc<Self>, boundary: &Boundary, length: Length) {
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

/// Where a panel floating over the 3D view is: where it was last drawn,
/// and, for a panel placed by its margins from the top left (a card beside
/// the selection), also where its margins now put it at its natural size,
/// since its allocation lags a move made in the same frame.
fn panel_rects(widget: &gtk::Widget, design: &gtk::Overlay) -> Vec<Rect> {
    let mut rects = Vec::new();
    if widget.halign() == gtk::Align::Start && widget.valign() == gtk::Align::Start {
        let (width, height) = natural_size(widget);
        let room = crate::convert::f32_of_int(design.height() - widget.margin_top()).max(0.0);
        rects.push(Rect {
            x: crate::convert::f32_of_int(widget.margin_start()),
            y: crate::convert::f32_of_int(widget.margin_top()),
            width,
            height: height.min(room),
        });
    }
    if widget.is_mapped()
        && let Some(bounds) = widget.compute_bounds(design)
    {
        rects.push(Rect {
            x: bounds.x(),
            y: bounds.y(),
            width: bounds.width(),
            height: bounds.height(),
        });
    }
    rects
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

/// The lower piece's size for a boundary dragged to `fraction` of the
/// two pieces, settled where the session's boundary says
/// ([`SplitBoundary::settle`]).
pub(in crate::ui) fn boundary_at(boundary: &Boundary, fraction: f32) -> Length {
    let total = boundary.split.pair();
    // In millionths of the two pieces, so the size is worked out exactly.
    let millionths = i64::from(crate::convert::int_of_f32(
        (fraction.clamp(0.0, 1.0) * 1_000_000.0).round(),
    ));
    let raw = Length::from_nm(
        total
            .nm()
            .checked_mul(millionths)
            .map_or(total.nm() / 2, |nm| nm / 1_000_000),
    );
    boundary.split.settle(raw)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_drag_toward_the_start_makes_more_pieces() {
        assert_eq!(count_at(1.0), 1);
        assert_eq!(count_at(0.55), 2);
        assert_eq!(count_at(0.34), 3);
        assert_eq!(count_at(0.26), 4);
        assert_eq!(count_at(0.0), MOST);
        assert_eq!(count_at(-0.4), MOST);
        assert_eq!(count_at(3.0), 1);
    }

    #[test]
    fn a_point_is_measured_along_the_screen_segment() {
        assert!((fraction_along((5.0, 0.0), (0.0, 0.0), (10.0, 0.0)) - 0.5).abs() < 1e-6);
        assert!((fraction_along((0.0, 15.0), (0.0, 0.0), (0.0, 10.0)) - 1.5).abs() < 1e-6);
        assert_eq!(fraction_along((1.0, 1.0), (2.0, 2.0), (2.0, 2.0)), 0.0);
    }

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
