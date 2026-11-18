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
use stave_app::editing::{EditCommand, is_component, parse_name};
use stave_lang::placement::Axis;
use stave_lang::units::{Length, NM_PER_MM};
use stave_scene::{Aabb, NodeId, SceneGraph};

use super::super::Window;
use super::item_tools::{MOST, RepeatRow, item_base};

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

/// Where a boundary dragged to `raw` (the lower piece's size) settles:
/// on half of the two pieces (an even division), on the whole split's
/// even size, or on a round size (a whole 10 mm) when it is near one, else
/// on a whole millimetre; never less than a millimetre either side.
pub(in crate::ui) fn snap_boundary(raw: Length, total: Length, even: Option<Length>) -> Length {
    let total_nm = total.nm();
    let millimetre = NM_PER_MM;
    let near = (total_nm / 30).max(millimetre);
    let raw_nm = raw
        .nm()
        .clamp(millimetre, (total_nm - millimetre).max(millimetre));
    let half = total_nm / 2;
    let mut targets = vec![half];
    if let Some(even) = even {
        targets.push(even.nm());
    }
    for target in targets {
        if (raw_nm - target).abs() <= near {
            return Length::from_nm(target);
        }
    }
    let round = 10 * millimetre;
    let nearest_round = (raw_nm + round / 2) / round * round;
    if (raw_nm - nearest_round).abs() <= near / 6 && nearest_round > 0 && nearest_round < total_nm {
        return Length::from_nm(nearest_round);
    }
    Length::from_nm((raw_nm + millimetre / 2) / millimetre * millimetre)
}

/// The space a box fills, if it fills one: the smallest space holding its
/// middle that it covers at least half of along every axis (a drawer in
/// its bay, a case in the envelope; not a rail between two legs). Only
/// for showing the count handle: whether it can be repeated is the
/// session's to say.
pub(in crate::ui) fn space_filled_by(
    graph: &SceneGraph,
    posed: &stave_scene::Posed,
    volume: &Aabb,
) -> Option<Aabb> {
    let centre = volume.centre();
    let size = volume.max - volume.min;
    graph
        .regions()
        .iter()
        .filter_map(|region| graph.region_volume(posed, region.id))
        .filter(|space| {
            let room = space.max - space.min;
            (0..3).all(|axis| {
                centre[axis] >= space.min[axis]
                    && centre[axis] <= space.max[axis]
                    && room[axis] > 0.0
                    && size[axis] >= room[axis] * 0.5
            })
        })
        .min_by(|a, b| {
            let volume = |space: &Aabb| {
                let room = space.max - space.min;
                room.x * room.y * room.z
            };
            volume(a).total_cmp(&volume(b))
        })
}

/// A boundary between two neighbouring pieces of a split, as drawn.
#[derive(Debug, Clone, PartialEq)]
pub(in crate::ui) struct Boundary {
    /// The split's region array: `drawer_bay`.
    pub(in crate::ui) region: String,
    /// The lower piece, from 0 in label order.
    pub(in crate::ui) lower: usize,
    /// The axis it splits along.
    pub(in crate::ui) axis: Axis,
    /// The two pieces' sizes along it.
    pub(in crate::ui) sizes: (Length, Length),
    /// Every piece's size, for the even division.
    pub(in crate::ui) whole: Length,
    /// How many pieces the split has.
    pub(in crate::ui) pieces: usize,
    /// The far face of the lower piece, the boundary, and the far face of
    /// the upper, in the 3D view's frame, at the split's left (or bottom)
    /// front edge.
    pub(in crate::ui) points: (Vec3, Vec3, Vec3),
}

/// The boundaries of the project's splits (regions written as an array,
/// `drawer_bay[bottom]`, `drawer_bay[top]`, side by side along one axis),
/// for the splits whose space holds `inside`.
pub(in crate::ui) fn boundaries(graph: &SceneGraph, inside: Vec3) -> Vec<Boundary> {
    let mut groups: Vec<(String, Vec<usize>)> = Vec::new();
    for (index, region) in graph.regions().iter().enumerate() {
        if region.assembly.is_some() {
            continue;
        }
        let Some(open) = region.name.find('[') else {
            continue;
        };
        let base = &region.name[..open];
        if region.name[open..].contains('.') {
            continue;
        }
        match groups.iter_mut().find(|(name, _)| name == base) {
            Some((_, members)) => members.push(index),
            None => groups.push((base.to_owned(), vec![index])),
        }
    }
    let mut found = Vec::new();
    for (base, members) in groups {
        let regions: Vec<_> = members
            .iter()
            .filter_map(|index| graph.regions().get(*index))
            .collect();
        if regions.len() < 2 {
            continue;
        }
        let Some(union) = regions
            .iter()
            .map(|region| region.volume)
            .reduce(Aabb::union)
        else {
            continue;
        };
        let holds = (0..3).all(|axis| {
            inside[axis] >= union.min[axis] - 0.5 && inside[axis] <= union.max[axis] + 0.5
        });
        if !holds {
            continue;
        }
        let Some(axis) = stave_lang::placement::Axis::ALL.into_iter().find(|axis| {
            regions.windows(2).all(|pair| match pair {
                [a, b] => {
                    a.bounds.max().get(*axis) == b.bounds.min().get(*axis)
                        || a.bounds.min().get(*axis) == b.bounds.max().get(*axis)
                }
                _ => false,
            })
        }) else {
            continue;
        };
        let whole = regions
            .iter()
            .try_fold(Length::ZERO, |sum, region| {
                sum.checked_add(region.bounds.size(axis))
            })
            .unwrap_or(Length::ZERO);
        for (lower, pair) in regions.windows(2).enumerate() {
            let [a, b] = pair else {
                continue;
            };
            let direction = (b.volume.centre() - a.volume.centre()).normalize_or_zero();
            let far = |volume: &Aabb, sign: f32| {
                let half = (volume.max - volume.min) * 0.5;
                volume.centre() + direction * sign * half.dot(direction.abs())
            };
            let start = at_edge(far(&a.volume, -1.0), &union, axis);
            let middle = at_edge(far(&a.volume, 1.0), &union, axis);
            let end = at_edge(far(&b.volume, 1.0), &union, axis);
            found.push(Boundary {
                region: base.clone(),
                lower,
                axis,
                sizes: (a.bounds.size(axis), b.bounds.size(axis)),
                whole,
                pieces: regions.len(),
                points: (start, middle, end),
            });
        }
    }
    found
}

/// A handle floating over the 3D view, dragged with the pointer.
pub(in crate::ui) struct Handle {
    /// The pill.
    pub(in crate::ui) root: gtk::Box,
    label: gtk::Label,
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
        Handle { root, label }
    }

    fn place(&self, (x, y): (f32, f32)) {
        self.root
            .set_margin_start(crate::convert::int_of_f32((x - 30.0).max(0.0).round()));
        self.root
            .set_margin_top(crate::convert::int_of_f32((y - 14.0).max(0.0).round()));
        self.root.set_visible(true);
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
    /// On the chosen candidate's card.
    pub(in crate::ui) row: RepeatRow,
    count: Handle,
    boundaries: RefCell<Vec<(Handle, Boundary)>>,
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
            row: RepeatRow::new(1),
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
    /// `true` when the item of the project by this name fills a space of
    /// the design (see [`space_filled_by`]).
    pub(in crate::ui) fn item_fills_a_space(&self, item: &str) -> bool {
        let session = self.session.borrow();
        let state = self.state.borrow();
        let (Some((_, graph)), Some(posed)) = (session.drawable(), state.posed.as_ref()) else {
            return false;
        };
        let nodes: Vec<NodeId> = graph
            .node_ids()
            .filter(|id| {
                graph
                    .node(*id)
                    .is_some_and(|node| item_base(&node.name) == item)
            })
            .collect();
        graph
            .bounds_of(posed, &nodes)
            .and_then(|volume| space_filled_by(graph, posed, &volume))
            .is_some()
    }

    /// Puts the repeat controls on the card and over the 3D view.
    pub(in crate::ui) fn connect_repeats(self: &Rc<Self>) {
        let repeats = &self.editing.repeats;
        // The stepper sits on the candidate's card, above its buttons.
        let card = &self.editing.card.root;
        let last = card.last_child();
        card.insert_child_after(
            &repeats.row.root,
            last.and_then(|w| w.prev_sibling()).as_ref(),
        );
        repeats.row.root.add_css_class("stave-repeat-on-card");
        let window = Rc::downgrade(self);
        repeats.row.connect_changed(move |count, axis| {
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

    /// Shows the candidate card's stepper for a component that fills a
    /// space, with the repeat the session holds.
    pub(in crate::ui) fn update_repeat_row(&self) {
        let shown = {
            let session = self.session.borrow();
            session
                .edit()
                .and_then(|edit| edit.chosen())
                .and_then(|chosen| {
                    let candidate = chosen.candidate();
                    let fills = candidate
                        .place
                        .iter()
                        .any(|param| matches!(param.slot, PlaceSlot::Region(Some(_))));
                    (is_component(candidate) && fills).then(|| {
                        chosen
                            .repeat()
                            .map_or((1, None), |repeat| (repeat.count.get(), Some(repeat.axis)))
                    })
                })
        };
        let row = &self.editing.repeats.row;
        match shown {
            Some((count, axis)) => {
                row.show(count, axis.unwrap_or_else(|| row.axis()), true);
                row.root.set_visible(true);
            }
            None => row.root.set_visible(false),
        }
    }

    /// The space the count handle stands on, the axis and the count: the
    /// candidate's space while its card shows the stepper; the selected
    /// item's box while its card offers a first repeat.
    fn count_handle_on(&self) -> Option<(Aabb, Axis, usize)> {
        let session = self.session.borrow();
        let state = self.state.borrow();
        let (_, graph) = session.drawable()?;
        let posed = state.posed.as_ref()?;
        if let Some(chosen) = session.edit().and_then(|edit| edit.chosen()) {
            if !self.editing.repeats.row.root.is_visible() {
                return None;
            }
            let region = chosen
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
                .unwrap_or_else(|| repeats.row.count());
            return Some((volume, repeats.row.axis(), count));
        }
        if session.edit().is_some() || session.change().is_some() {
            return None;
        }
        let tools = &self.editing.tools;
        if !tools.root.is_visible() {
            return None;
        }
        let (design, _) = session.drawable()?;
        let item = super::item_tools::selected_item(design, session.selection()?)?;
        if super::item_tools::piece_count(design, &item) > 1 {
            return None;
        }
        let nodes: Vec<NodeId> = graph
            .node_ids()
            .filter(|id| {
                graph
                    .node(*id)
                    .is_some_and(|node| item_base(&node.name) == item)
            })
            .collect();
        let volume = graph.bounds_of(posed, &nodes)?;
        space_filled_by(graph, posed, &volume)?;
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
                        repeats.count.place(point);
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
        let root = &repeats.count.root;
        let origin = (
            crate::convert::f32_of_int(root.margin_start()) + crate::convert::f32_of(x),
            crate::convert::f32_of_int(root.margin_top()) + crate::convert::f32_of(y),
        );
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
        let Some((_, axis, _)) = self.count_handle_on() else {
            return Err("no count handle shows: choose something that fills a space".to_owned());
        };
        self.editing.repeats.dragged_count.set(Some(count));
        self.place_repeat_handles();
        self.editing.repeats.dragged_count.set(None);
        self.repeat_selected(count, Some(axis));
        Ok(())
    }

    // ---- the boundaries between pieces ----

    /// The boundaries shown for the selection: of the splits holding the
    /// selected item, while nothing is being edited.
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
        let nodes = self.selected_nodes();
        let session = self.session.borrow();
        let state = self.state.borrow();
        let (Some((_, graph)), Some(posed)) = (session.drawable(), state.posed.as_ref()) else {
            return Vec::new();
        };
        graph
            .bounds_of(posed, &nodes)
            .map(|bounds| boundaries(graph, bounds.centre()))
            .unwrap_or_default()
    }

    fn place_boundary_handles(self: &Rc<Self>, focus: bool) {
        let shown = if focus || !self.gestures_allowed() {
            Vec::new()
        } else {
            self.shown_boundaries()
        };
        let repeats = &self.editing.repeats;
        let dragging = repeats.dragged_boundary.get();
        {
            let mut handles = repeats.boundaries.borrow_mut();
            while handles.len() < shown.len() {
                let handle =
                    Handle::new("Drag to move the boundary: only these two pieces change size");
                handle.root.add_css_class("boundary");
                self.widgets.design.add_overlay(&handle.root);
                let index = handles.len();
                self.connect_boundary_drag(&handle.root, index);
                handles.push((
                    handle,
                    Boundary {
                        region: String::new(),
                        lower: 0,
                        axis: Axis::Up,
                        sizes: (Length::ZERO, Length::ZERO),
                        whole: Length::ZERO,
                        pieces: 0,
                        points: (Vec3::ZERO, Vec3::ZERO, Vec3::ZERO),
                    },
                ));
            }
            for (index, (handle, boundary)) in handles.iter_mut().enumerate() {
                let Some(found) = shown.get(index) else {
                    handle.root.set_visible(false);
                    continue;
                };
                *boundary = found.clone();
                let lower = match dragging {
                    Some((dragged, length)) if dragged == index => length,
                    _ => found.sizes.0,
                };
                let total = found
                    .sizes
                    .0
                    .checked_add(found.sizes.1)
                    .unwrap_or(found.sizes.0);
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
                    Some(point) => handle.place(point),
                    None => handle.root.set_visible(false),
                }
            }
        }
    }

    fn connect_boundary_drag(self: &Rc<Self>, root: &gtk::Box, index: usize) {
        let drag = gtk::GestureDrag::new();
        let window = Rc::downgrade(self);
        let widget = root.clone();
        drag.connect_drag_begin(move |_, x, y| {
            let Some(window) = window.upgrade() else {
                return;
            };
            let boundary = window
                .editing
                .repeats
                .boundaries
                .borrow()
                .get(index)
                .map(|(_, boundary)| boundary.clone());
            let Some(boundary) = boundary else {
                return;
            };
            let state = window.state.borrow();
            let (start, _, end) = boundary.points;
            let (Some(start), Some(end)) =
                (window.project(&state, start), window.project(&state, end))
            else {
                return;
            };
            let origin = (
                crate::convert::f32_of_int(widget.margin_start()) + crate::convert::f32_of(x),
                crate::convert::f32_of_int(widget.margin_top()) + crate::convert::f32_of(y),
            );
            window
                .editing
                .repeats
                .drag
                .set(Some(DragFrom { start, end, origin }));
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
            let boundary = repeats
                .boundaries
                .borrow()
                .get(index)
                .map(|(_, boundary)| boundary.clone());
            let Some(boundary) = boundary else {
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
            let boundary = repeats
                .boundaries
                .borrow()
                .get(dragged)
                .map(|(_, boundary)| boundary.clone());
            if let Some(boundary) = boundary {
                window.move_boundary(&boundary, length);
            }
        });
        root.add_controller(drag);
    }

    /// Moves a boundary between two pieces so the lower is `length` long.
    pub(in crate::ui) fn move_boundary(self: &Rc<Self>, boundary: &Boundary, length: Length) {
        match parse_name(&boundary.region) {
            Ok(region) => self.start_change(EditCommand::MoveShares {
                region,
                lower: boundary.lower,
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

/// The lower piece's size for a boundary dragged to `fraction` of the
/// two pieces, snapped.
pub(in crate::ui) fn boundary_at(boundary: &Boundary, fraction: f32) -> Length {
    let total = boundary
        .sizes
        .0
        .checked_add(boundary.sizes.1)
        .unwrap_or(boundary.sizes.0);
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
    let even = (boundary.pieces > 0).then(|| {
        Length::from_nm(boundary.whole.nm() / i64::try_from(boundary.pieces).unwrap_or(1))
    });
    snap_boundary(raw, total, even)
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

    #[test]
    fn a_boundary_snaps_to_even_and_round_sizes() {
        let mm = Length::from_mm;
        // Near half of the two: half.
        assert_eq!(snap_boundary(mm(247), mm(500), None), mm(250));
        // Near the whole split's even size.
        assert_eq!(snap_boundary(mm(168), mm(500), Some(mm(166))), mm(166));
        // Near a round size: it.
        assert_eq!(
            snap_boundary(Length::from_nm(179_600_000), mm(500), None),
            mm(180)
        );
        // Elsewhere, a whole millimetre.
        assert_eq!(
            snap_boundary(Length::from_nm(134_400_000), mm(500), None),
            mm(134)
        );
        // Never past either end.
        assert_eq!(snap_boundary(mm(-20), mm(500), None), mm(1));
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
