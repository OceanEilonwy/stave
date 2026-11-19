//! Grips in the 3D view (`docs/gui.md`, "Grips"; `docs/editing.md`,
//! "Grips"): the sizes of a selected assembly, of the envelope or of a
//! split's boundaries drawn on the model as dimensions with handles, a
//! drag that moves the value live with its ghost and verdict, a value
//! typed in place, the sizes to pick from when a size follows several,
//! and the switch that lets the solver move a param. The view shows only
//! the gizmo (small round handles with an arrow the way a drag makes the
//! size larger) and, while a drag is open, a one-line readout at its top
//! (`overall_width 940 → 1040 mm`); what the drag changes, the sizes to
//! pick from, the stop and the verdict are in the Edit panel's change
//! card (`grip_panel.rs`), whose Apply writes it.
//!
//! ```text
//! selection / a click in a space ──ListGrips──▶ GripsChanged ──▶ handles and labels on the model
//!                                               StopsJob on a worker ──▶ GripStopsFound ──▶ the range's hard ends
//! press a handle ──BeginDrag(i)──▶ move ──DragTo(value)──▶ DragMoved / VerdictChanged ──▶ readout, Edit panel, ghost
//! let go ──▶ Add when it fits          Escape or Cancel ──▶ nothing written
//! click the number ──▶ entry ──Enter──▶ TypeGripValue ──▶ Add when it fits, else why, still editable
//! the switch (⋮) ──▶ MakeSolved / MakePlain ──▶ Edit panel ──Apply──▶ Add
//! ```
//!
//! The session decides everything: what a grip moves, where it snaps, what
//! stops it, the verdict and the text written. Here a pointer's travel
//! along the handle's direction on screen becomes a value to send, and
//! what the session holds is drawn.

use stave_app::i18n::{fill_in, tr};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use adw::prelude::*;
use glam::Vec3;
use gtk::{gdk, gio, glib};
use stave_app::change::{Change, ChangeRequest};
use stave_app::edit::{Picked, Verdict};
use stave_app::editing::{EditCommand, EditEvent, Progress, parse_value};
use stave_app::gripping::{FoundStops, GripList, ListedGrip};
use stave_app::grips::{FollowKind, GripAction, GripSelection, GripSize, PlaceInput};
use stave_app::session::{Command, Event, Selection, View};
use stave_lang::ast::Ident;
use stave_lang::placement::{Axis, Face};
use stave_lang::units::Length;
use stave_model::ids::{AssemblyId, ParamId};
use stave_model::solved::Point;
use stave_model::trace::Amount;
use stave_scene::{FaceOf, GripFace, GripOf, GripPlace, Item, SceneGraph};

use super::grip_layer::GripLayer;
use super::grip_panel::{GripPanel, PanelAction, Readout, install_style};
use super::grip_text::{
    GripLook, Held, RangeEnd, SolverSwitch, amount_like, amount_words, grip_look, grip_range,
    grip_value, held_words, label_words, length_mm, mm_amount, panel_model, solver_switches,
    suggested_range, typed_check, typed_refusal,
};
use crate::ui::update::Unprinted;
use crate::ui::update::editing::EditingInput;
use crate::ui::{EDGE, Window};

/// The handle's size, in pixels (8 inside a 2 px ring).
const HANDLE: f64 = 12.0;
/// How far a dimension's label stands off its line, in pixels.
const LABEL_OFFSET: f64 = 18.0;
/// The design's red (#e01b24) and its pale stripe (#f6b8bb), as on the
/// door's swing slider.
const BLOCKED: (f64, f64, f64) = (0.878, 0.106, 0.141);
const BLOCKED_PALE: (f64, f64, f64) = (0.965, 0.722, 0.733);

/// The grips' widgets over the 3D view, and what only they need.
pub(in crate::ui) struct GripView {
    /// The dimension lines and ranges, drawn under the handles.
    area: gtk::DrawingArea,
    /// The handles and labels, under the window's floating cards.
    layer: GripLayer,
    /// The size a drag moves, as a section of the change's card.
    panel: GripPanel,
    /// The one line at the top of the view while a drag is open.
    readout: Readout,
    /// What the area draws, worked out after each frame.
    drawing: Rc<RefCell<Vec<Drawn>>>,
    state: GripState,
}

/// What the grips keep between events.
#[derive(Default)]
struct GripState {
    /// What the grips shown are of.
    shown_for: Option<GripSelection>,
    /// Where each grip with a place sits, at rest.
    places: Vec<Placed>,
    /// The widgets on the model, one set per placed grip.
    marks: Vec<Marks>,
    /// Stops the running stops search.
    cancel: Option<Arc<AtomicBool>>,
    /// The pointer holds a handle.
    gesture: Option<Gesture>,
    /// The value to send next, while one is being tried.
    pending: Option<Amount>,
    /// A `DragTo` is waiting to be sent.
    sending: bool,
    /// The last value sent.
    last_sent: Option<Amount>,
    /// The grip whose change (a drag, or a param made solved or plain) the
    /// grips started and hold open.
    open: Option<usize>,
    /// The drag was held at an end of its range.
    held: Option<Held>,
    /// The grip whose number is being typed.
    typing: Option<usize>,
    /// A listing from the new design is on its way.
    relist_queued: bool,
    /// The grip whose options (⋮) the size section holds.
    options_for: Option<usize>,
}

/// Where a grip sits, at rest, in the render frame (millimetres).
#[derive(Debug, Clone, Copy)]
struct Placed {
    /// Its index in the listing.
    index: usize,
    /// The handle.
    handle: Vec3,
    /// The other end of its dimension.
    start: Vec3,
    /// Dragging along it makes the value larger.
    direction: Vec3,
    /// Its value now, in millimetres.
    value: f64,
    /// How it is drawn.
    look: GripLook,
}

impl Placed {
    /// Where the handle is for a value (millimetres).
    fn at(&self, value: f64) -> Vec3 {
        self.handle + self.direction * crate::convert::f32_of(value - self.value)
    }
}

/// The pointer holding a handle.
#[derive(Debug, Clone, Copy)]
struct Gesture {
    /// The grip.
    index: usize,
    /// Its value when it was pressed, in millimetres.
    from: f64,
    /// Which way on screen makes it larger, a unit vector.
    unit: (f64, f64),
    /// Pixels on screen per millimetre along it.
    scale: f64,
}

/// One grip's widgets on the model.
struct Marks {
    /// Its index in the listing.
    index: usize,
    /// The handle, or the padlock of one that can't move.
    handle: gtk::Widget,
    /// The label: its value, or what it follows.
    label: gtk::Box,
    /// The number and the entry it turns into.
    stack: Option<gtk::Stack>,
    /// The entry.
    entry: Option<gtk::Entry>,
    /// Why a typed value was refused.
    error: gtk::Label,
    /// The menu with the solver's switch.
    switches: Option<gtk::MenuButton>,
}

/// One dimension as drawn, in the view's pixels.
#[derive(Debug, Clone, Copy)]
struct Drawn {
    look: GripLook,
    start: (f64, f64),
    end: (f64, f64),
    active: bool,
    range: Option<DrawnRange>,
}

/// A range along a dimension, in the view's pixels.
#[derive(Debug, Clone, Copy)]
struct DrawnRange {
    low: (f64, f64),
    high: (f64, f64),
    low_end: RangeEnd,
    high_end: RangeEnd,
}

impl GripView {
    /// The widgets, hidden until grips are listed.
    pub(in crate::ui) fn new() -> GripView {
        let area = gtk::DrawingArea::builder()
            .can_target(false)
            .css_classes(["stave-grip-lines"])
            .visible(false)
            .build();
        let drawing: Rc<RefCell<Vec<Drawn>>> = Rc::default();
        let shown = drawing.clone();
        area.set_draw_func(move |area, context, _, _| {
            draw(area, context, &shown.borrow());
        });
        GripView {
            area,
            layer: GripLayer::new(),
            panel: GripPanel::new(),
            readout: Readout::new(),
            drawing,
            state: GripState::default(),
        }
    }
}

/// The top-level assembly an assembly is part of (itself when it is
/// one).
fn top_level(design: &stave_model::solved::SolvedDesign, id: AssemblyId) -> Option<AssemblyId> {
    let mut found = id;
    for _ in 0..=design.assemblies().len() {
        match design.assembly(found)?.parent {
            Some(parent) => found = parent,
            None => return Some(found),
        }
    }
    None
}

/// The high face along an axis.
fn high_face(axis: Axis) -> Face {
    match axis {
        Axis::Across => Face::Right,
        Axis::Up => Face::Top,
        Axis::Deep => Face::Back,
    }
}

/// Where a grip sits, from what its listing carries: the scene's
/// `grip_place`, or, for a length the scene can't tell from another of
/// the same value, the two faces its placement term names; a part's
/// length or width across the part. `handles` holds the handles placed
/// before this one: a size drawn along a box's edge takes the nearest
/// edge whose handle is not on one of them, so no two handles share a
/// corner (the envelope's width and height would both end at the corner
/// nearest the eye) and each can be grabbed.
fn place_of(
    graph: &SceneGraph,
    listed: &ListedGrip,
    index: usize,
    eye: Option<Vec3>,
    handles: &mut Vec<Vec3>,
) -> Option<Placed> {
    let placed = place_apart(graph, listed, index, eye, handles)?;
    handles.push(placed.handle);
    Some(placed)
}

fn place_apart(
    graph: &SceneGraph,
    listed: &ListedGrip,
    index: usize,
    eye: Option<Vec3>,
    handles: &[Vec3],
) -> Option<Placed> {
    let value = length_mm(grip_value(listed))?;
    let look = grip_look(listed);
    let placed = |handle: Vec3, start: Vec3, direction: Vec3| Placed {
        index,
        handle,
        start,
        direction,
        value,
        look,
    };
    let behind = |point: Vec3, direction: Vec3| point - direction * crate::convert::f32_of(value);
    let Some(place) = listed.grip.place.as_ref() else {
        // A part's length or width: along the edge of its drawn box
        // nearest the eye.
        let (GripSize::PartSize { part, .. }, Some(axis)) = (&listed.grip.size, listed.grip.axis)
        else {
            return None;
        };
        let bounds = graph.drawn_box(graph.node_of(Item::Part(*part))?)?;
        let (start, handle, up) = edge_dimension(&bounds, axis, eye, handles)?;
        return Some(placed(handle, start, up));
    };
    match place {
        PlaceInput::Length {
            assembly,
            axis,
            param,
            lengths,
            between,
        } => {
            let lengths: Vec<(&str, Length)> = lengths
                .iter()
                .map(|(name, length)| (name.as_str(), *length))
                .collect();
            let found = graph.grip_place(GripOf::Length {
                assembly: *assembly,
                axis: *axis,
                param,
                lengths: &lengths,
                nudged: None,
            });
            match found {
                Some(GripPlace::Length { start, handle, .. }) => {
                    Some(placed(handle.point, start, handle.direction))
                }
                Some(GripPlace::Envelope { .. } | GripPlace::Share { .. }) | None => {
                    let (from, to) = between.as_ref()?;
                    between_place(graph, *axis, from, to)
                        .map(|(start, handle, direction)| placed(handle, start, direction))
                }
            }
        }
        // An envelope size along the box's edge nearest the eye, its
        // handle on the face that moves.
        PlaceInput::Envelope { region, face } => {
            let bounds = graph.region(*region)?.bounds;
            let (start, handle, up) = edge_dimension(&bounds, face.axis(), eye, handles)?;
            Some(placed(handle, start, up))
        }
        PlaceInput::Share { low, high, axis } => match graph.grip_place(GripOf::Share {
            low: *low,
            high: *high,
            axis: *axis,
        })? {
            GripPlace::Share { handle, .. } => Some(placed(
                handle.point,
                behind(handle.point, handle.direction),
                handle.direction,
            )),
            GripPlace::Length { .. } | GripPlace::Envelope { .. } => None,
        },
    }
}

/// A box's size along `axis` as a dimension on the edge nearest the eye
/// whose high end is not one of `handles` (the nearest edge when every
/// one is): its low end, its high end (the handle) and the way up the
/// axis.
fn edge_dimension(
    bounds: &stave_model::solved::WorldBox,
    axis: Axis,
    eye: Option<Vec3>,
    handles: &[Vec3],
) -> Option<(Vec3, Vec3, Vec3)> {
    let [first, second] = axis.others();
    let (low, high) = (bounds.min().get(axis), bounds.max().get(axis));
    let at = |along: Length, (a, b): (Length, Length)| {
        stave_geom::render::point(
            Point::default()
                .with(axis, along)
                .with(first, a)
                .with(second, b),
        )
    };
    let mut edges = Vec::new();
    for a in [bounds.min().get(first), bounds.max().get(first)] {
        for b in [bounds.min().get(second), bounds.max().get(second)] {
            edges.push((a, b));
        }
    }
    let distance = |edge| {
        eye.map_or(0.0, |eye| {
            ((at(low, edge) + at(high, edge)) * 0.5 - eye).length()
        })
    };
    edges.sort_by(|x, y| distance(*x).total_cmp(&distance(*y)));
    let free = |edge: &(Length, Length)| {
        let handle = at(high, *edge);
        !handles.iter().any(|other| (*other - handle).length() < 1.0)
    };
    let edge = edges
        .iter()
        .copied()
        .find(free)
        .or_else(|| edges.first().copied())?;
    let up = stave_geom::render::direction(high_face(axis).direction());
    Some((at(low, edge), at(high, edge), up))
}

/// A length's dimension from the two faces its term names: from `from`
/// to `to` through the middle of the box `to` belongs to.
fn between_place(
    graph: &SceneGraph,
    axis: Axis,
    from: &GripFace,
    to: &GripFace,
) -> Option<(Vec3, Vec3, Vec3)> {
    let bounds = match to.of {
        FaceOf::Part(part) => graph.drawn_box(graph.node_of(Item::Part(part))?)?,
        FaceOf::Region(region) => graph.region(region)?.bounds,
    };
    let [first, second] = axis.others();
    let middle = |other: Axis| {
        let (low, high) = (bounds.min().get(other).nm(), bounds.max().get(other).nm());
        Length::from_nm(low.saturating_add(high.saturating_sub(low) / 2))
    };
    let point = |at: Length| {
        stave_geom::render::point(
            Point::default()
                .with(axis, at)
                .with(first, middle(first))
                .with(second, middle(second)),
        )
    };
    let up = stave_geom::render::direction(high_face(axis).direction());
    let direction = if to.at >= from.at { up } else { -up };
    Some((point(from.at), point(to.at), direction))
}

/// Draws the dimensions and ranges.
fn draw(area: &gtk::DrawingArea, context: &gtk::cairo::Context, drawing: &[Drawn]) {
    let accent = area.color();
    let accent = (
        f64::from(accent.red()),
        f64::from(accent.green()),
        f64::from(accent.blue()),
    );
    let dark = adw::StyleManager::default().is_dark();
    let (halo, ink) = if dark {
        ((0.0, 0.0, 0.0), (0.92, 0.92, 0.92))
    } else {
        ((1.0, 1.0, 1.0), (0.2, 0.2, 0.24))
    };
    context.set_line_cap(gtk::cairo::LineCap::Round);
    for drawn in drawing {
        let (sx, sy) = drawn.start;
        let (ex, ey) = drawn.end;
        let length = (ex - sx).hypot(ey - sy);
        if length < 1.0 {
            continue;
        }
        let along = ((ex - sx) / length, (ey - sy) / length);
        let across = (-along.1, along.0);
        if let Some(range) = drawn.range {
            // Beside the dimension, on the side away from its label (the
            // label stands above the line).
            let side = if across.1 > 0.0 {
                across
            } else {
                (-across.0, -across.1)
            };
            let shift = |(x, y): (f64, f64)| (x + side.0 * 16.0, y + side.1 * 16.0);
            let range = DrawnRange {
                low: shift(range.low),
                high: shift(range.high),
                ..range
            };
            draw_range(context, range, across, accent, ink);
        }
        let width = if drawn.active { 2.5 } else { 1.75 };
        let colour = match drawn.look {
            GripLook::Moves | GripLook::Follows => accent,
            GripLook::Locked => ink,
        };
        let line = |context: &gtk::cairo::Context| {
            context.move_to(sx, sy);
            context.line_to(ex, ey);
            for (x, y) in [(sx, sy), (ex, ey)] {
                context.move_to(x - across.0 * 7.0, y - across.1 * 7.0);
                context.line_to(x + across.0 * 7.0, y + across.1 * 7.0);
            }
        };
        // A halo first, so the line reads over wood and background alike.
        context.set_dash(&[], 0.0);
        line(context);
        context.set_source_rgba(halo.0, halo.1, halo.2, 0.8);
        context.set_line_width(width + 3.0);
        let _ = context.stroke();
        match drawn.look {
            GripLook::Moves => context.set_dash(&[], 0.0),
            GripLook::Follows => context.set_dash(&[7.0, 4.0], 0.0),
            GripLook::Locked => context.set_dash(&[0.5, 4.0], 0.0),
        }
        line(context);
        context.set_source_rgb(colour.0, colour.1, colour.2);
        context.set_line_width(width);
        let _ = context.stroke();
        context.set_dash(&[], 0.0);
        // Arrowheads at the start for what can be dragged.
        if drawn.look != GripLook::Locked && length > 24.0 {
            context.move_to(sx, sy);
            context.line_to(
                sx + along.0 * 8.0 + across.0 * 4.0,
                sy + along.1 * 8.0 + across.1 * 4.0,
            );
            context.line_to(
                sx + along.0 * 8.0 - across.0 * 4.0,
                sy + along.1 * 8.0 - across.1 * 4.0,
            );
            context.close_path();
            let _ = context.fill();
        }
        // A short arrow out past the handle: the way a drag makes it
        // larger.
        if drawn.look != GripLook::Locked {
            let base = (ex + along.0 * 9.0, ey + along.1 * 9.0);
            let tip = (ex + along.0 * 19.0, ey + along.1 * 19.0);
            let arrow = |context: &gtk::cairo::Context| {
                context.move_to(base.0, base.1);
                context.line_to(tip.0 - along.0 * 4.0, tip.1 - along.1 * 4.0);
            };
            arrow(context);
            context.set_source_rgba(halo.0, halo.1, halo.2, 0.8);
            context.set_line_width(4.5);
            let _ = context.stroke();
            arrow(context);
            context.set_source_rgb(accent.0, accent.1, accent.2);
            context.set_line_width(1.75);
            let _ = context.stroke();
            context.move_to(tip.0, tip.1);
            context.line_to(
                tip.0 - along.0 * 6.0 + across.0 * 3.5,
                tip.1 - along.1 * 6.0 + across.1 * 3.5,
            );
            context.line_to(
                tip.0 - along.0 * 6.0 - across.0 * 3.5,
                tip.1 - along.1 * 6.0 - across.1 * 3.5,
            );
            context.close_path();
            let _ = context.fill();
        }
    }
}

/// A range along a dimension: a pale track between its ends; a rule's
/// stop is a hard red end hatched beyond, as the door's swing slider
/// draws where the door first hits; a `within`'s end a hard grey end.
fn draw_range(
    context: &gtk::cairo::Context,
    range: DrawnRange,
    across: (f64, f64),
    accent: (f64, f64, f64),
    ink: (f64, f64, f64),
) {
    let (lx, ly) = range.low;
    let (hx, hy) = range.high;
    let length = (hx - lx).hypot(hy - ly);
    if length < 1.0 {
        return;
    }
    let along = ((hx - lx) / length, (hy - ly) / length);
    context.move_to(lx, ly);
    context.line_to(hx, hy);
    context.set_source_rgba(accent.0, accent.1, accent.2, 0.45);
    context.set_line_width(6.0);
    let _ = context.stroke();
    for ((x, y), end, outward) in [
        ((lx, ly), range.low_end, (-along.0, -along.1)),
        ((hx, hy), range.high_end, along),
    ] {
        match end {
            RangeEnd::Stop => {
                // Hatched red beyond the stop, then the hard end.
                let _ = context.save();
                context.move_to(x, y);
                context.line_to(x + outward.0 * 28.0, y + outward.1 * 28.0);
                context.set_line_width(10.0);
                context.set_line_cap(gtk::cairo::LineCap::Butt);
                context.set_source_rgb(BLOCKED_PALE.0, BLOCKED_PALE.1, BLOCKED_PALE.2);
                let _ = context.stroke();
                context.set_source_rgb(BLOCKED.0, BLOCKED.1, BLOCKED.2);
                for step in 0..5 {
                    let at = 3.0 + f64::from(step) * 5.5;
                    let (cx, cy) = (x + outward.0 * at, y + outward.1 * at);
                    context.move_to(
                        cx - across.0 * 4.5 - outward.0 * 2.0,
                        cy - across.1 * 4.5 - outward.1 * 2.0,
                    );
                    context.line_to(
                        cx + across.0 * 4.5 + outward.0 * 2.0,
                        cy + across.1 * 4.5 + outward.1 * 2.0,
                    );
                }
                context.set_line_width(1.8);
                let _ = context.stroke();
                context.move_to(x - across.0 * 12.0, y - across.1 * 12.0);
                context.line_to(x + across.0 * 12.0, y + across.1 * 12.0);
                context.set_line_width(4.0);
                let _ = context.stroke();
                let _ = context.restore();
            }
            RangeEnd::Domain => {
                context.move_to(x - across.0 * 8.0, y - across.1 * 8.0);
                context.line_to(x + across.0 * 8.0, y + across.1 * 8.0);
                context.set_source_rgb(ink.0, ink.1, ink.2);
                context.set_line_width(3.0);
                let _ = context.stroke();
            }
            RangeEnd::Open => {}
        }
    }
}

impl Window {
    /// Puts the grips' lines and panel over the 3D view and connects the
    /// panel.
    pub(in crate::ui) fn connect_grips(&mut self) {
        let grips = &self.editing.grips;
        install_style(&self.widgets.design.display());
        let design = &self.widgets.design;
        design.add_overlay(&grips.area);
        design.add_overlay(&grips.layer);
        design.add_overlay(&grips.readout.root);
        // The window's floating cards stay above the grips: put back on
        // top in the order they were laid (nothing has the keyboard yet).
        let widgets = &self.widgets;
        let editing = &self.editing;
        for floating in [
            widgets.hint_box.upcast_ref::<gtk::Widget>(),
            widgets.edge_pin.upcast_ref(),
            editing.menu.root.upcast_ref(),
            editing.card.root.upcast_ref(),
        ] {
            if floating.parent().as_ref() == Some(design.upcast_ref()) {
                design.remove_overlay(floating);
                design.add_overlay(floating);
            }
        }
        let input = self.input();
        grips
            .panel
            .connect_action(move |action| input.emit(EditingInput::GripPanel(action).into()));
    }

    /// The grips' panel asked for something.
    pub(in crate::ui) fn grip_panel_asked(&mut self, action: PanelAction) {
        match action {
            PanelAction::Pick(param) => self.pick_later(param),
            PanelAction::Type(text) => {
                let open = self.session.change().and_then(drag_index).or(self
                    .editing
                    .grips
                    .state
                    .open);
                if let Some(index) = open {
                    self.apply_typed(index, &text);
                }
            }
        }
    }

    // ---- what the grips are of ----

    /// The selection changed: an assembly (or a part of one) shows the
    /// grips of its top-level assembly; anything else hides an assembly's
    /// grips (a space's stay until another click).
    pub(in crate::ui) fn grips_follow_selection(&mut self) {
        let wanted = {
            let session = &self.session;
            let assembly = match (session.selection(), session.drawable()) {
                (Some(Selection::Assembly(id)), Some((design, _))) => top_level(design, *id),
                (Some(Selection::Part(part)), Some((design, graph))) => graph
                    .node_of(Item::Part(*part))
                    .and_then(|node| graph.node(node))
                    .and_then(|node| node.assemblies.first().copied())
                    .and_then(|id| top_level(design, id)),
                _ => None,
            };
            match (assembly, self.editing.grips.state.shown_for) {
                (Some(id), _) => Some(GripSelection::Assembly(id)),
                (None, Some(region @ (GripSelection::Envelope | GripSelection::Split(_)))) => {
                    session.selection().is_none().then_some(region)
                }
                (None, Some(GripSelection::Assembly(_)) | None) => None,
            }
        };
        self.show_grips_soon(wanted);
    }

    /// Lists the grips of `wanted` once the window has drawn what is
    /// already asked for (a listing prepares the project again).
    fn show_grips_soon(&mut self, wanted: Option<GripSelection>) {
        if self.editing.grips.state.shown_for == wanted && self.grips_listed(wanted) {
            return;
        }
        if wanted.is_none() {
            self.show_grips(None);
            return;
        }
        let input = self.input();
        glib::timeout_add_local_once(Duration::from_millis(30), move || {
            input.emit(EditingInput::ShowGrips(wanted).into());
        });
    }

    /// `true` when the session's listing is of `wanted`.
    fn grips_listed(&self, wanted: Option<GripSelection>) -> bool {
        let session = &self.session;
        match (session.grips(), wanted) {
            (Some(grips), Some(wanted)) => grips.selection() == wanted,
            (None, None) => true,
            (Some(_), None) | (None, Some(_)) => false,
        }
    }

    /// Lists the grips of `wanted` now (none: hides them). A selection
    /// with no grips (a space that is not a piece of a split) shows none,
    /// quietly.
    pub(in crate::ui) fn show_grips(&mut self, wanted: Option<GripSelection>) {
        self.stop_stops_search();
        self.editing.grips.state.shown_for = wanted;
        let Some(selection) = wanted else {
            self.clear_grip_marks();
            self.show_card();
            self.request_render();
            return;
        };
        {
            let session = &mut self.session;
            session.set_edit_files(Arc::new(self.widgets.files.edit_files()));
        }
        let events = self
            .session
            .apply(Command::Edit(EditCommand::ListGrips(selection)));
        // A space with no boundaries, or a design that doesn't place, has
        // no grips: nothing to say.
        let events: Vec<Event> = events
            .into_iter()
            .filter(|event| !matches!(event, Event::Edit(EditEvent::Rejected(_))))
            .collect();
        if self
            .session
            .grips()
            .is_none_or(|grips| grips.selection() != selection)
        {
            self.editing.grips.state.shown_for = None;
            self.clear_grip_marks();
        }
        self.handle(events);
    }

    /// An edit started: a click in the envelope or in a piece of a split
    /// shows its grips beside the menu.
    fn grips_for_edit_start(&mut self) {
        let region = {
            let session = &self.session;
            let region = match session.edit().map(|edit| edit.picked()) {
                Some(Picked::Region(region)) => Some(*region),
                Some(Picked::Face(_) | Picked::Between(_) | Picked::Group(_)) | None => None,
            };
            region
                .zip(session.drawable())
                .and_then(|(region, (_, graph))| {
                    let name = &graph.region(region)?.name;
                    if name == stave_app::naming::ENVELOPE_REGION {
                        Some(GripSelection::Envelope)
                    } else if name.contains('[') {
                        Some(GripSelection::Split(region))
                    } else {
                        None
                    }
                })
        };
        let keep = matches!(
            self.editing.grips.state.shown_for,
            Some(GripSelection::Assembly(_))
        ) && region.is_none()
            && self.session.selection().is_some();
        if !keep {
            self.show_grips_soon(region);
        }
    }

    // ---- events ----

    /// Redraws what an edit event changes for the grips. Called before the
    /// edit view's own handling of it.
    pub(in crate::ui) fn grip_event(&mut self, event: &EditEvent) {
        match event {
            EditEvent::GripsChanged => {
                let (listed, shown_for) = {
                    let session = &self.session;
                    (
                        session.grips().map(GripList::selection),
                        self.editing.grips.state.shown_for,
                    )
                };
                match (listed, shown_for) {
                    (Some(listed), Some(shown)) if listed == shown => {
                        self.build_grip_marks();
                        self.start_stops_search();
                    }
                    // A commit drops the listing: list again from the new
                    // design.
                    (None, Some(_)) => {
                        self.clear_grip_marks();
                        self.relist_grips_soon();
                    }
                    (Some(_), _) | (None, None) => {}
                }
            }
            EditEvent::GripStopsFound(_) => {
                let busy = {
                    let state = &self.editing.grips.state;
                    state.gesture.is_some() || state.typing.is_some() || state.open.is_some()
                };
                if !busy {
                    self.build_grip_marks();
                }
                self.request_render();
            }
            EditEvent::Started => self.grips_for_edit_start(),
            EditEvent::Committed => {
                let state = &mut self.editing.grips.state;
                state.open = None;
                state.typing = None;
                state.gesture = None;
                state.held = None;
                state.pending = None;
                state.last_sent = None;
            }
            EditEvent::Cancelled => {
                let still_open = self.session.change().is_some_and(|change| {
                    matches!(
                        change.request(),
                        ChangeRequest::Grip { .. } | ChangeRequest::ParamKind { .. }
                    )
                });
                let (was_open, region) = {
                    let state = &mut self.editing.grips.state;
                    let was_open = state.open.is_some();
                    if !still_open {
                        state.open = None;
                        state.gesture = None;
                        state.held = None;
                        state.pending = None;
                        state.last_sent = None;
                        if let Some(index) = state.typing.take()
                            && let Some(marks) = state.marks.iter().find(|m| m.index == index)
                        {
                            close_entry(marks);
                        }
                    }
                    (
                        was_open,
                        matches!(
                            state.shown_for,
                            Some(GripSelection::Envelope | GripSelection::Split(_))
                        ),
                    )
                };
                // The menu of a space was let go (not a drag on its
                // grips): its grips go with it.
                if !was_open && !still_open && region && self.session.edit().is_none() {
                    self.show_grips(None);
                }
            }
            EditEvent::ChangeStarted
            | EditEvent::DragMoved
            | EditEvent::ParamPicked
            | EditEvent::VerdictChanged
            | EditEvent::MenuChanged
            | EditEvent::Chosen
            | EditEvent::PlaceBound
            | EditEvent::SettingChanged
            | EditEvent::Renamed
            | EditEvent::RepeatChanged
            | EditEvent::DependantsIncluded
            | EditEvent::FixApplied
            | EditEvent::OptionsChanged
            | EditEvent::Rejected(_) => {}
        }
        self.request_render();
    }

    /// A new result (a commit, or the text changed in the editor): the
    /// grips shown are listed again from it, unless one is being dragged
    /// or typed (its change checks the text it began from when written).
    pub(in crate::ui) fn grips_result_changed(&mut self) {
        let busy = {
            let state = &self.editing.grips.state;
            state.shown_for.is_none()
                || state.gesture.is_some()
                || state.typing.is_some()
                || state.open.is_some()
        };
        if !busy {
            self.relist_grips_soon();
        }
    }

    /// Lists the grips shown again once the events in hand are handled.
    pub(in crate::ui) fn relist_grips_soon(&mut self) {
        if std::mem::replace(&mut self.editing.grips.state.relist_queued, true) {
            return;
        }
        let input = self.input();
        glib::timeout_add_local_once(Duration::from_millis(30), move || {
            input.emit(EditingInput::RelistGrips.into());
        });
    }

    /// Lists the grips shown again, as asked by [`Window::relist_grips_soon`].
    pub(in crate::ui) fn relist_grips(&mut self) {
        let shown = {
            let state = &mut self.editing.grips.state;
            state.relist_queued = false;
            state.shown_for
        };
        if shown.is_some() {
            self.show_grips(shown);
        }
    }

    // ---- the stops, off the main thread ----

    fn stop_stops_search(&mut self) {
        if let Some(cancel) = self.editing.grips.state.cancel.take() {
            cancel.store(true, Ordering::Relaxed);
        }
    }

    /// Searches the listed grips' stops on a worker thread, handing each
    /// answer to the session as it comes.
    fn start_stops_search(&mut self) {
        self.stop_stops_search();
        let Some(job) = self.session.grip_stops_job() else {
            return;
        };
        if job.searches().is_empty() {
            return;
        }
        let cancel = Arc::new(AtomicBool::new(false));
        self.editing.grips.state.cancel = Some(cancel.clone());
        let (sender, receiver) = async_channel::unbounded::<FoundStops>();
        gio::spawn_blocking(move || {
            let _ = job.run(&cancel, &mut |found| {
                let _ = sender.send_blocking(found);
            });
        });
        let input = self.input();
        glib::spawn_future_local(async move {
            while let Ok(found) = receiver.recv().await {
                if input
                    .send(EditingInput::GripStopsFound(Unprinted(Box::new(found))).into())
                    .is_err()
                {
                    return;
                }
            }
        });
    }

    // ---- the widgets on the model ----

    fn clear_grip_marks(&mut self) {
        let marks = std::mem::take(&mut self.editing.grips.state.marks);
        for marks in marks {
            self.editing.grips.layer.remove(&marks.handle);
            self.editing.grips.layer.remove(&marks.label);
        }
        let state = &mut self.editing.grips.state;
        state.places.clear();
        state.typing = None;
    }

    /// Makes a handle and a label for every listed grip the scene can
    /// place.
    fn build_grip_marks(&mut self) {
        self.clear_grip_marks();
        let eye = self.state.orbit.map(|orbit| orbit.eye());
        let (places, listed): (Vec<Placed>, Vec<ListedGrip>) = {
            let session = &self.session;
            let (Some(grips), Some((_, graph))) = (session.grips(), session.drawable()) else {
                return;
            };
            let mut handles = Vec::new();
            let places: Vec<Placed> = grips
                .grips()
                .iter()
                .enumerate()
                .filter_map(|(index, listed)| place_of(graph, listed, index, eye, &mut handles))
                .collect();
            let listed = places
                .iter()
                .filter_map(|place| grips.grips().get(place.index).cloned())
                .collect();
            (places, listed)
        };
        let mut marks = Vec::new();
        for (place, listed) in places.iter().zip(&listed) {
            let built = self.grip_marks(place, listed);
            self.editing.grips.layer.put(&built.handle, 0.0, 0.0);
            self.editing.grips.layer.put(&built.label, 0.0, 0.0);
            marks.push(built);
        }
        {
            let state = &mut self.editing.grips.state;
            state.places = places;
            state.marks = marks;
        }
        self.show_card();
    }

    /// One grip's handle (or padlock) and label.
    fn grip_marks(&mut self, place: &Placed, listed: &ListedGrip) -> Marks {
        let index = place.index;
        let words = label_words(listed);
        let handle: gtk::Widget = match place.look {
            GripLook::Locked => {
                let lock = gtk::Image::builder()
                    .icon_name("changes-prevent-symbolic")
                    .css_classes(["stave-grip-lock"])
                    .tooltip_text(fill_in(
                        &tr("{grip} can't be dragged: {why}"),
                        &[("grip", &listed.grip.label), ("why", &words)],
                    ))
                    .build();
                lock.upcast()
            }
            GripLook::Moves | GripLook::Follows => {
                let handle = gtk::Box::builder()
                    .css_classes(["stave-grip-handle"])
                    .focusable(true)
                    .accessible_role(gtk::AccessibleRole::Slider)
                    .tooltip_text(fill_in(
                        &tr("Drag {grip}: {value}. Hold Ctrl for round numbers."),
                        &[("grip", &listed.grip.label), ("value", &words)],
                    ))
                    .build();
                handle.set_cursor_from_name(Some("grab"));
                handle.update_property(&[gtk::accessible::Property::Label(&fill_in(
                    &tr("Drag {grip}"),
                    &[("grip", &listed.grip.label)],
                ))]);
                self.connect_handle(&handle, index);
                handle.upcast()
            }
        };
        handle.set_halign(gtk::Align::Start);
        handle.set_valign(gtk::Align::Start);
        handle.set_visible(false);

        let label = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .css_classes(["stave-grip-label"])
            .halign(gtk::Align::Start)
            .valign(gtk::Align::Start)
            .visible(false)
            .build();
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        label.append(&row);
        let error = gtk::Label::builder()
            .css_classes(["stave-grip-error"])
            .xalign(0.0)
            .wrap(true)
            .max_width_chars(32)
            .visible(false)
            .build();
        let (stack, entry) = match place.look {
            GripLook::Locked => {
                label.add_css_class("locked");
                row.set_spacing(6);
                row.append(&gtk::Image::from_icon_name("changes-prevent-symbolic"));
                row.append(&gtk::Label::new(Some(&amount_words(grip_value(listed)))));
                label.set_tooltip_text(Some(&words));
                (None, None)
            }
            GripLook::Moves | GripLook::Follows => {
                let number = gtk::Button::builder()
                    .label(amount_words(grip_value(listed)))
                    .css_classes(["flat", "stave-grip-number"])
                    .tooltip_text(fill_in(
                        &tr("{value}. Click to type a value"),
                        &[("value", &words)],
                    ))
                    .build();
                let entry = gtk::Entry::builder()
                    .text(amount_words(grip_value(listed)))
                    .width_chars(9)
                    .build();
                entry.update_property(&[gtk::accessible::Property::Label(&fill_in(
                    &tr("Value of {grip}"),
                    &[("grip", &listed.grip.label)],
                ))]);
                let stack = gtk::Stack::builder()
                    .hhomogeneous(false)
                    .vhomogeneous(false)
                    .build();
                stack.add_named(&number, Some("number"));
                stack.add_named(&entry, Some("entry"));
                stack.set_visible_child_name("number");
                row.append(&stack);
                let input = self.input();
                number.connect_clicked(move |_| {
                    input.emit(EditingInput::OpenGripEntry(index).into())
                });
                // Applied once the entry's own signal is done: the message
                // is queued.
                let input = self.input();
                entry.connect_activate(move |entry| {
                    let text = entry.text().to_string();
                    input.emit(EditingInput::GripTyped { index, text }.into());
                });
                let keys = gtk::EventControllerKey::new();
                let input = self.input();
                keys.connect_key_pressed(move |_, key, _, _| {
                    if key != gdk::Key::Escape {
                        return glib::Propagation::Proceed;
                    }
                    input.emit(EditingInput::CloseGripTyping(index).into());
                    glib::Propagation::Stop
                });
                entry.add_controller(keys);
                (Some(stack), Some(entry))
            }
        };
        let switches = solver_switches(listed);
        let switches = (!switches.is_empty() && place.look != GripLook::Locked)
            .then(|| self.switch_menu(index, &switches));
        if let Some(menu) = &switches {
            row.append(menu);
        }
        label.append(&error);
        Marks {
            index,
            handle,
            label,
            stack,
            entry,
            error,
            switches,
        }
    }

    /// The menu beside a grip's number: "Let the solver move this", with
    /// the range a solved param needs, or the switch to make it plain.
    fn switch_menu(&self, index: usize, switches: &[SolverSwitch]) -> gtk::MenuButton {
        let content = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(10)
            .css_classes(["stave-grip-switches"])
            .build();
        let menu = gtk::MenuButton::builder()
            .icon_name("view-more-symbolic")
            .css_classes(["flat"])
            .tooltip_text(tr("Let the solver move this"))
            .build();
        menu.update_property(&[gtk::accessible::Property::Label(&tr(
            "Let the solver move this",
        ))]);
        let popover = gtk::Popover::builder().child(&content).build();
        menu.set_popover(Some(&popover));
        let only = switches.len() == 1;
        for switch in switches {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
            let template = if only {
                tr("Let the solver move this ({param})")
            } else {
                tr("Let the solver move {param}")
            };
            let words = fill_in(&template, &[("param", &switch.param.as_str())]);
            row.append(
                &gtk::Label::builder()
                    .label(&words)
                    .xalign(0.0)
                    .hexpand(true)
                    .build(),
            );
            let toggle = gtk::Switch::builder()
                .active(switch.solved)
                .valign(gtk::Align::Center)
                .build();
            toggle.update_property(&[gtk::accessible::Property::Label(&words)]);
            row.append(&toggle);
            content.append(&row);
            // The range a solved param needs, typed before it is made one.
            let (low_text, high_text) = suggested_range(switch.value);
            let range = gtk::Box::builder()
                .orientation(gtk::Orientation::Vertical)
                .spacing(6)
                .visible(false)
                .build();
            range.append(
                &gtk::Label::builder()
                    .label(tr("The solver keeps it within:"))
                    .xalign(0.0)
                    .css_classes(["stave-grip-note"])
                    .build(),
            );
            let ends = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            let low = gtk::Entry::builder().text(&low_text).width_chars(8).build();
            let high = gtk::Entry::builder()
                .text(&high_text)
                .width_chars(8)
                .build();
            low.update_property(&[gtk::accessible::Property::Label(&tr("Lowest value"))]);
            high.update_property(&[gtk::accessible::Property::Label(&tr("Highest value"))]);
            ends.append(&low);
            ends.append(&gtk::Label::new(Some(&tr("to"))));
            ends.append(&high);
            range.append(&ends);
            let make = gtk::Button::builder()
                .label(tr("Make it solved"))
                .css_classes(["suggested-action"])
                .halign(gtk::Align::End)
                .build();
            range.append(&make);
            content.append(&range);
            let solved = switch.solved;
            let param = switch.param.clone();
            let input = self.input();
            let shown = range.clone();
            // The switch and the button live inside the menu's popover, so
            // their handlers hold the menu weakly: a strong hold would be a
            // cycle that keeps every menu alive after it is replaced.
            let weak_menu = menu.downgrade();
            toggle.connect_state_set(move |_, on| {
                shown.set_visible(on && !solved);
                if !on && solved {
                    if let Some(menu) = weak_menu.upgrade() {
                        menu.popdown();
                    }
                    input.emit(
                        EditingInput::GripMakePlain {
                            index,
                            param: param.clone(),
                        }
                        .into(),
                    );
                }
                glib::Propagation::Proceed
            });
            let param = switch.param.clone();
            let input = self.input();
            let weak_menu = menu.downgrade();
            make.connect_clicked(move |_| {
                let Some(menu) = weak_menu.upgrade() else {
                    return;
                };
                input.emit(
                    EditingInput::GripMakeSolved {
                        index,
                        param: param.clone(),
                        low: low.text().trim().to_owned(),
                        high: high.text().trim().to_owned(),
                        menu,
                    }
                    .into(),
                );
            });
        }
        menu
    }

    // ---- dragging ----

    fn connect_handle(&self, handle: &gtk::Box, index: usize) {
        let drag = gtk::GestureDrag::new();
        let input = self.input();
        drag.connect_drag_begin(move |gesture, _, _| {
            gesture.set_state(gtk::EventSequenceState::Claimed);
            input.emit(EditingInput::GripPressed(index).into());
        });
        let input = self.input();
        drag.connect_drag_update(move |gesture, dx, dy| {
            let round = gesture
                .current_event_state()
                .contains(gdk::ModifierType::CONTROL_MASK);
            input.emit(EditingInput::GripMoved { dx, dy, round }.into());
        });
        let input = self.input();
        drag.connect_drag_end(move |_, _, _| input.emit(EditingInput::GripReleased.into()));
        handle.add_controller(drag);
        // Arrow keys move it a step at a time; Enter writes it.
        let keys = gtk::EventControllerKey::new();
        let input = self.input();
        keys.connect_key_pressed(move |_, key, _, modifiers| {
            let round = modifiers.contains(gdk::ModifierType::CONTROL_MASK);
            let steps = match key {
                gdk::Key::Right | gdk::Key::Up => 1,
                gdk::Key::Left | gdk::Key::Down => -1,
                gdk::Key::Return | gdk::Key::KP_Enter => {
                    input.emit(EditingInput::SendEditLater(EditCommand::Add).into());
                    return glib::Propagation::Stop;
                }
                _ => return glib::Propagation::Proceed,
            };
            input.emit(
                EditingInput::GripStepped {
                    index,
                    steps,
                    round,
                }
                .into(),
            );
            glib::Propagation::Stop
        });
        handle.add_controller(keys);
    }

    /// The listed grip at `index`.
    fn listed_grip(&self, index: usize) -> Option<ListedGrip> {
        self.session.grips()?.grips().get(index).cloned()
    }

    /// The value a drag on `index` is at now: where it was moved, or its
    /// value.
    fn grip_target(&self, index: usize) -> Option<Amount> {
        let session = &self.session;
        let moved = session
            .change()
            .and_then(|change| drag_target(change, index));
        moved.or_else(|| Some(grip_value(session.grips()?.grips().get(index)?)))
    }

    /// Starts (or goes on with) a drag on `index`: a drag already open on
    /// the same grip goes on from where it is.
    fn begin_grip_drag(&mut self, index: usize) {
        let going_on = self
            .session
            .change()
            .is_some_and(|change| drag_index(change) == Some(index));
        if going_on {
            return;
        }
        {
            let state = &mut self.editing.grips.state;
            state.open = Some(index);
            state.held = None;
            state.last_sent = None;
            state.pending = None;
        }
        self.dispatch(Command::Edit(EditCommand::BeginDrag(index)));
        // A size that follows several plain params lists them at once,
        // each with the value it would take, to pick from first.
        let choices = self
            .listed_grip(index)
            .filter(|listed| match &listed.grip.action {
                GripAction::Follows(follows) => {
                    let plain = follows.follows.iter().filter(|value| {
                        value.kind == FollowKind::Plain
                            && value.coefficient.is_some()
                            && value.path.is_some()
                    });
                    let solved = follows
                        .follows
                        .iter()
                        .any(|value| value.kind == FollowKind::Solved);
                    plain.count() > 1 && !solved
                }
                GripAction::Moves(_) => false,
            });
        if let Some(listed) = choices {
            let value = grip_value(&listed);
            self.editing.grips.state.last_sent = Some(value);
            self.dispatch(Command::Edit(EditCommand::DragTo(value)));
        }
    }

    /// The pointer pressed a handle.
    pub(in crate::ui) fn grip_pressed(&mut self, index: usize) {
        self.begin_grip_drag(index);
        let from = self
            .grip_target(index)
            .and_then(length_mm)
            .unwrap_or_default();
        let (unit, scale) = self
            .screen_direction(index, from)
            .unwrap_or(((0.0, -1.0), 1.0));
        self.editing.grips.state.gesture = Some(Gesture {
            index,
            from,
            unit,
            scale,
        });
        self.set_dragging_look(index, true);
        self.request_render();
    }

    /// Which way on screen makes a grip larger, and how many pixels a
    /// millimetre is that way, at the value `at`.
    fn screen_direction(&self, index: usize, at: f64) -> Option<((f64, f64), f64)> {
        let place = self
            .editing
            .grips
            .state
            .places
            .iter()
            .find(|place| place.index == index)
            .copied()?;
        let state = &self.state;
        let here = place.at(at);
        let (x0, y0) = self.project(state, here)?;
        let (x1, y1) = self.project(state, here + place.direction * 100.0)?;
        let (dx, dy) = (f64::from(x1 - x0), f64::from(y1 - y0));
        let length = dx.hypot(dy);
        // Along the line of sight a drag can't be read off the screen.
        (length > 4.0).then(|| ((dx / length, dy / length), length / 100.0))
    }

    /// The pointer moved by `(dx, dy)` since it pressed the handle.
    pub(in crate::ui) fn grip_moved(&mut self, dx: f64, dy: f64, round: bool) {
        let Some(gesture) = self.editing.grips.state.gesture else {
            return;
        };
        let travel = (dx * gesture.unit.0 + dy * gesture.unit.1) / gesture.scale;
        let Some(wanted) = mm_amount(gesture.from + travel) else {
            return;
        };
        self.grip_wanted(gesture.index, wanted, round);
    }

    /// Arrow keys on a handle: a step of its resolution each way.
    pub(in crate::ui) fn grip_stepped(&mut self, index: usize, steps: i32, round: bool) {
        let Some(listed) = self.listed_grip(index) else {
            return;
        };
        self.begin_grip_drag(index);
        let Some(now) = self.grip_target(index) else {
            return;
        };
        let Some(wanted) = stave_app::grip_values::stepped(&listed, now, steps, round) else {
            return;
        };
        self.grip_wanted(index, wanted, round);
        self.flush_drag();
    }

    /// Moves a drag toward `wanted`: snapped to the grip's steps (or to
    /// round numbers), held inside its range, and sent once the last try
    /// is done.
    fn grip_wanted(&mut self, index: usize, wanted: Amount, round: bool) {
        let Some(listed) = self.listed_grip(index) else {
            return;
        };
        let (target, held) = stave_app::grip_values::drag_target(&listed, wanted, round);
        let send = {
            let state = &mut self.editing.grips.state;
            state.held = held;
            if state.last_sent == Some(target) && state.pending.is_none() {
                false
            } else {
                state.pending = Some(target);
                !std::mem::replace(&mut state.sending, true)
            }
        };
        if send {
            // Once the events in hand are handled.
            self.send(EditingInput::FlushDrag);
        }
        self.request_render();
    }

    /// Sends the value waiting, if any.
    pub(in crate::ui) fn flush_drag(&mut self) {
        let pending = {
            let state = &mut self.editing.grips.state;
            state.sending = false;
            let pending = state.pending.take();
            if pending.is_some() {
                state.last_sent = pending;
            }
            pending
        };
        if let Some(target) = pending {
            self.dispatch(Command::Edit(EditCommand::DragTo(target)));
        }
    }

    /// The pointer let the handle go: writes the drag when it fits;
    /// otherwise the panel says why and offers Cancel.
    pub(in crate::ui) fn grip_released(&mut self) {
        self.flush_drag();
        let index = self.editing.grips.state.gesture.take().map(|g| g.index);
        if let Some(index) = index {
            self.set_dragging_look(index, false);
        }
        let fits = self
            .session
            .change()
            .is_some_and(|change| drag_index(change).is_some() && fits(change.progress()));
        if fits {
            self.send_edit_later(EditCommand::Add);
        }
        self.request_render();
    }

    fn set_dragging_look(&self, index: usize, dragging: bool) {
        let state = &self.editing.grips.state;
        if let Some(marks) = state.marks.iter().find(|marks| marks.index == index) {
            if dragging {
                marks.handle.add_css_class("dragging");
            } else {
                marks.handle.remove_css_class("dragging");
            }
        }
    }

    /// A pick from the sizes a follows grip could change, sent once the
    /// panel's own signal is done.
    fn pick_later(&mut self, param: ParamId) {
        self.send(EditingInput::Dispatch(Command::Edit(
            EditCommand::PickParam(param),
        )));
    }

    /// A grip's range switch turned off: the size is written plain again.
    pub(in crate::ui) fn grip_make_plain(&mut self, index: usize, param: Ident) {
        self.editing.grips.state.open = Some(index);
        self.send_edit_later(EditCommand::MakePlain(param));
    }

    /// "Make it solved" on a grip's menu, with the range typed.
    pub(in crate::ui) fn grip_make_solved(
        &mut self,
        index: usize,
        param: Ident,
        (low, high): (&str, &str),
        menu: &gtk::MenuButton,
    ) {
        match (parse_value(low), parse_value(high)) {
            (Ok(low), Ok(high)) => {
                menu.popdown();
                self.editing.grips.state.open = Some(index);
                self.send_edit_later(EditCommand::MakeSolved { param, low, high });
            }
            (Err(error), _) | (_, Err(error)) => {
                self.edit_toast(&fill_in(
                    &tr("That range can't be read: {reason}"),
                    &[("reason", &error.0)],
                ));
            }
        }
    }

    // ---- typing ----

    /// The number turns into an entry holding the value.
    pub(in crate::ui) fn open_entry(&mut self, index: usize) {
        let state = &mut self.editing.grips.state;
        if let Some(previous) = state.typing.replace(index)
            && previous != index
            && let Some(marks) = state.marks.iter().find(|m| m.index == previous)
        {
            close_entry(marks);
        }
        let Some(marks) = state.marks.iter().find(|m| m.index == index) else {
            return;
        };
        if let (Some(stack), Some(entry)) = (&marks.stack, &marks.entry) {
            stack.set_visible_child_name("entry");
            entry.grab_focus();
            entry.select_region(0, -1);
        }
    }

    /// Escape in the entry: back to the number; a value typed and tried is
    /// dropped.
    pub(in crate::ui) fn close_typing(&mut self, index: usize) {
        let typed = {
            let state = &mut self.editing.grips.state;
            if let Some(marks) = state.marks.iter().find(|m| m.index == index) {
                close_entry(marks);
            }
            state.typing = None;
            state.open == Some(index)
        };
        if typed {
            self.send_edit_later(EditCommand::Cancel);
        }
    }

    /// Enter in the entry: the value is tried through the same check as a
    /// drag; it is written when it fits, else the entry says why and stays.
    pub(in crate::ui) fn apply_typed(&mut self, index: usize, text: &str) {
        let Some(listed) = self.listed_grip(index) else {
            return;
        };
        // A value the session would write differently (past an end of the
        // range, off the resolution) is refused before it is tried: what
        // is written is always the number typed.
        let typed = amount_like(grip_value(&listed), text.trim())
            .and_then(|value| typed_check(&listed, value).map(|()| value));
        let refusal = match typed {
            Err(words) => {
                // A value tried before and still shown in the panel goes,
                // so the panel never shows another number than the entry.
                self.drop_typed_change(index);
                Some(words)
            }
            Ok(value) => {
                {
                    let state = &mut self.editing.grips.state;
                    state.open = Some(index);
                    state.typing = Some(index);
                    state.held = None;
                }
                self.dispatch(Command::Edit(EditCommand::TypeGripValue {
                    grip: index,
                    value,
                }));
                let (progress, written) = {
                    let session = &self.session;
                    let change = session.change();
                    (
                        change.map(|change| change.progress().clone()),
                        change.and_then(|change| typed_written(&listed, change, index)),
                    )
                };
                match (progress, written) {
                    // Whatever the verdict, the session would write
                    // another number: refused, and its try dropped.
                    (_, Some(written)) if written.raw() != value.raw() => {
                        self.drop_typed_change(index);
                        Some(fill_in(
                            &tr("Can't be {value}: it would be written as {written}"),
                            &[
                                ("value", &amount_words(value)),
                                ("written", &amount_words(written)),
                            ],
                        ))
                    }
                    (Some(progress), _) => typed_refusal(&progress),
                    (None, _) => Some(tr("it can't be tried")),
                }
            }
        };
        self.show_typed_result(index, text, refusal);
    }

    /// A typed value's try that is still open on grip `index` is
    /// cancelled; the entry stays open.
    fn drop_typed_change(&mut self, index: usize) {
        let open = self.session.change().and_then(drag_index) == Some(index);
        if open {
            self.dispatch(Command::Edit(EditCommand::Cancel));
        }
    }

    /// Shows what became of a typed value: written (Add), or why not,
    /// under the entry, which stays open holding the text typed.
    fn show_typed_result(&mut self, index: usize, text: &str, refusal: Option<String>) {
        let refused = {
            let state = &mut self.editing.grips.state;
            let Some(marks) = state.marks.iter().find(|m| m.index == index) else {
                return;
            };
            match refusal {
                Some(words) => {
                    // A cancel may have closed the entry: open it again.
                    if let (Some(stack), Some(entry)) = (&marks.stack, &marks.entry) {
                        stack.set_visible_child_name("entry");
                        if entry.text() != text {
                            entry.set_text(text);
                        }
                        entry.add_css_class("error");
                        entry.grab_focus();
                    }
                    marks.error.set_label(&words);
                    marks.error.set_visible(true);
                    state.typing = Some(index);
                    true
                }
                None => {
                    marks.error.set_visible(false);
                    false
                }
            }
        };
        if refused {
            // The label grew: placed again, clear of the handles.
            self.request_render();
        } else {
            self.dispatch(Command::Edit(EditCommand::Add));
        }
    }

    // ---- placing ----

    /// Puts the handles, labels and panel where the grips show, after a
    /// frame of the 3D view is drawn; hides them where grips don't belong
    /// (focus mode, pulled apart, another view, a menu entry chosen, the
    /// removal's dialog asking over the view).
    pub(in crate::ui) fn place_grips(&mut self) {
        let hidden = {
            let session = &self.session;
            self.state.focus.is_some()
                || session.explode().get() > 0.0
                || session.view() != View::Design
                || session.showing_older()
                || session.edit().is_some_and(|edit| edit.chosen().is_some())
                || session.grips().is_none()
                || self.editing.changes.dialog.root.is_visible()
        };
        let grips = &self.editing.grips;
        if hidden {
            grips.area.set_visible(false);
            grips.readout.show(None);
            // Hidden with nothing borrowed: hiding a handle mid-drag ends
            // its drag, and the drag's end borrows the grips' state.
            let marks: Vec<(gtk::Widget, gtk::Box)> = grips
                .state
                .marks
                .iter()
                .map(|marks| (marks.handle.clone(), marks.label.clone()))
                .collect();
            for (handle, label) in marks {
                handle.set_visible(false);
                label.set_visible(false);
            }
            return;
        }
        let session = &self.session;
        let view = &self.state;
        let state = &grips.state;
        let open = session.change().and_then(drag_index).or(state.open);
        let mut drawing = Vec::new();
        let mut labels = Vec::new();
        // Handles to hide once the borrows are let go (see above).
        let mut to_hide: Vec<gtk::Widget> = Vec::new();
        let (view_width, view_height) = (
            f64::from(self.widgets.design.width()),
            f64::from(self.widgets.design.height()),
        );
        let edge = f64::from(EDGE);
        let rail = f64::from(self.right_column());
        for (place, marks) in state.places.iter().zip(&state.marks) {
            let listed = session
                .grips()
                .and_then(|grips| grips.grips().get(place.index));
            let active = open == Some(place.index);
            let shown = open.is_none_or(|open| open == place.index) && listed.is_some();
            let target = if active {
                session
                    .change()
                    .and_then(|change| drag_target(change, place.index))
                    .or(state.pending)
                    .and_then(length_mm)
            } else {
                None
            };
            let value = target.unwrap_or(place.value);
            let start = self.project(view, place.start);
            let end = self.project(view, place.at(value));
            let (Some(start), Some(end), true) = (start, end, shown) else {
                to_hide.push(marks.handle.clone());
                marks.label.set_visible(false);
                continue;
            };
            let start = (f64::from(start.0), f64::from(start.1));
            let end = (f64::from(end.0), f64::from(end.1));
            let range = if active || state.typing == Some(place.index) {
                listed
                    .and_then(grip_range)
                    .and_then(|((low, low_end), (high, high_end))| {
                        let low = self.project(view, place.at(length_mm(low)?))?;
                        let high = self.project(view, place.at(length_mm(high)?))?;
                        Some(DrawnRange {
                            low: (f64::from(low.0), f64::from(low.1)),
                            high: (f64::from(high.0), f64::from(high.1)),
                            low_end,
                            high_end,
                        })
                    })
            } else {
                None
            };
            drawing.push(Drawn {
                look: place.look,
                start,
                end,
                active,
                range,
            });
            // The handle on the moving end; the label beside the middle.
            grips
                .layer
                .move_(&marks.handle, end.0 - HANDLE / 2.0, end.1 - HANDLE / 2.0);
            marks.handle.set_visible(true);
            if let Some(listed) = listed
                && let Some(stack) = &marks.stack
                && stack.visible_child_name().as_deref() == Some("number")
                && let Some(number) = stack.child_by_name("number").and_downcast::<gtk::Button>()
            {
                number.set_label(&amount_words(grip_value(listed)));
            }
            // While a drag is open its value is in the readout and the
            // Edit panel, not beside the handle (unless it is being typed).
            if active && state.typing != Some(place.index) {
                marks.label.set_visible(false);
                continue;
            }
            // Measured once shown: a hidden widget measures nothing.
            marks.label.set_visible(true);
            labels.push((&marks.label, start, end, measured(&marks.label)));
        }
        // Each label beside its line, where it covers no handle, no label
        // placed before it and no card floating over the view (the menu
        // of a space, the edit's card: a number under one could not be
        // clicked), inside the view and clear of the rail.
        let handles: Vec<(f64, f64)> = drawing.iter().map(|drawn| drawn.end).collect();
        let cards: Vec<Rect> = [
            self.editing.menu.root.upcast_ref::<gtk::Widget>(),
            self.editing.card.root.upcast_ref(),
        ]
        .into_iter()
        .filter(|card| card.is_visible())
        .filter_map(|card| card.compute_bounds(&self.widgets.design))
        .map(|bounds| Rect {
            x: f64::from(bounds.x()),
            y: f64::from(bounds.y()),
            width: f64::from(bounds.width()),
            height: f64::from(bounds.height()),
        })
        .collect();
        let mut taken: Vec<Rect> = Vec::new();
        for (label, start, end, (width, height)) in labels {
            let right = (view_width - rail - width - edge).max(edge);
            let bottom = (view_height - height - edge).max(edge);
            let spot = |(along, side): (f64, f64)| {
                let (dx, dy) = (end.0 - start.0, end.1 - start.1);
                let length = dx.hypot(dy).max(1.0);
                let mut across = (-dy / length, dx / length);
                if across.1 > 0.0 {
                    across = (-across.0, -across.1);
                }
                across = (across.0 * side, across.1 * side);
                let at = (start.0 + dx * along, start.1 + dy * along);
                Rect {
                    x: (at.0 + across.0 * LABEL_OFFSET - width / 2.0).clamp(edge, right),
                    y: (at.1 + across.1 * LABEL_OFFSET - height / 2.0).clamp(edge, bottom),
                    width,
                    height,
                }
            };
            let off_cards = |rect: &Rect| !cards.iter().any(|card| rect.overlaps(card));
            let clear = |rect: &Rect| {
                off_cards(rect)
                    && !handles
                        .iter()
                        .any(|point| rect.near(*point, HANDLE / 2.0 + 4.0))
                    && !taken.iter().any(|other| rect.overlaps(other))
            };
            // Above the line first, then below it; then on past the
            // handle, where a line that runs behind a card still shows.
            let length = (end.0 - start.0).hypot(end.1 - start.1).max(1.0);
            let past = 1.0 + (HANDLE + width.max(height)) / length;
            let spots: Vec<Rect> = [1.0, -1.0]
                .into_iter()
                .flat_map(|side| [0.5, 0.3, 0.7, 0.2, 0.8].map(|along| (along, side)))
                .chain([(past, 0.0), (past, 1.0), (past, -1.0)])
                .map(spot)
                .collect();
            let rect = spots
                .iter()
                .find(|rect| clear(rect))
                .or_else(|| spots.iter().find(|rect| off_cards(rect)))
                .copied()
                .unwrap_or_else(|| spot((0.5, 1.0)));
            grips.layer.move_(label, rect.x, rect.y);
            taken.push(rect);
        }
        grips.area.set_visible(true);
        *grips.drawing.borrow_mut() = drawing;
        grips.area.queue_draw();
        for handle in to_hide {
            handle.set_visible(false);
        }
        self.show_grip_section();
    }

    /// Draws the size section of the change's card for a change the grips
    /// started (a drag, or a param made solved or plain), with the grip's
    /// options, and the readout at the top of the view.
    fn show_grip_section(&mut self) {
        let session = &self.session;
        let grips = &self.editing.grips;
        let (stop, open) = {
            let state = &grips.state;
            let open = session.change().and_then(drag_index).or(state.open);
            let stop = state
                .held
                .zip(open)
                .and_then(|(held, index)| held_words(session.grips()?.grips().get(index)?, held));
            (stop, open)
        };
        let model = session.change().and_then(|change| {
            panel_model(
                change,
                |index| session.grips()?.grips().get(index).cloned(),
                false,
                stop,
            )
        });
        let Some(model) = model else {
            grips.panel.root.set_visible(false);
            grips.readout.show(None);
            return;
        };
        let slot = &self.editing.changes.card.grip_slot;
        if grips.panel.root.parent().as_ref() != Some(slot.upcast_ref()) {
            if let Some(parent) = grips.panel.root.parent().and_downcast::<gtk::Box>() {
                parent.remove(&grips.panel.root);
            }
            slot.append(&grips.panel.root);
        }
        grips.panel.show(&model);
        grips.panel.root.set_visible(true);
        grips.readout.show(model.readout.as_ref());
        // The grip's options (⋮), made again only for another grip.
        let wanted = open.filter(|index| {
            session
                .grips()
                .and_then(|grips| grips.grips().get(*index))
                .is_some_and(|listed| {
                    grip_look(listed) != GripLook::Locked && !solver_switches(listed).is_empty()
                })
        });
        let had = std::mem::replace(&mut self.editing.grips.state.options_for, wanted);
        let (session, grips) = (&self.session, &self.editing.grips);
        if wanted != had {
            while let Some(child) = grips.panel.options.first_child() {
                grips.panel.options.remove(&child);
            }
            let switches = wanted.and_then(|index| {
                let listed = session.grips()?.grips().get(index)?.clone();
                Some((index, solver_switches(&listed)))
            });
            if let Some((index, switches)) = switches {
                let menu = self.switch_menu(index, &switches);
                menu.set_tooltip_text(Some(&tr(
                    "Options: let the solver move this, and its range",
                )));
                grips.panel.options.append(&menu);
            }
        }
    }

    // ---- driving it without a pointer (screenshots) ----

    /// A drag on grip `index` to `value`, as the pointer would make it,
    /// still held.
    pub(in crate::ui) fn drag_grip_to(&mut self, index: usize, value: Amount) {
        self.grip_pressed(index);
        self.grip_wanted(index, value, false);
        self.flush_drag();
    }

    /// Lets the held handle go.
    pub(in crate::ui) fn release_grip(&mut self) {
        self.flush_drag();
        let index = self.editing.grips.state.gesture.take().map(|g| g.index);
        if let Some(index) = index {
            self.set_dragging_look(index, false);
        }
        let fits = self
            .session
            .change()
            .is_some_and(|change| drag_index(change).is_some() && fits(change.progress()));
        if fits {
            self.dispatch(Command::Edit(EditCommand::Add));
        }
    }

    /// Types a value into grip `index`'s entry and presses Enter.
    pub(in crate::ui) fn type_grip(&mut self, index: usize, text: &str) {
        self.open_entry(index);
        {
            let state = &self.editing.grips.state;
            if let Some(entry) = state
                .marks
                .iter()
                .find(|m| m.index == index)
                .and_then(|m| m.entry.as_ref())
            {
                entry.set_text(text);
            }
        }
        self.apply_typed(index, text);
    }

    /// Opens grip `index`'s menu with the solver's switch.
    pub(in crate::ui) fn open_grip_menu(&self, index: usize) -> bool {
        let state = &self.editing.grips.state;
        let Some(menu) = state
            .marks
            .iter()
            .find(|m| m.index == index)
            .and_then(|m| m.switches.as_ref())
        else {
            return false;
        };
        menu.popup();
        true
    }

    /// Marks a change of a param's kind as started from grip `index`.
    pub(in crate::ui) fn grip_change_opened(&mut self, index: usize) {
        self.editing.grips.state.open = Some(index);
    }

    /// The listed grip numbered `index`, for a script.
    pub(in crate::ui) fn grip_at(&self, index: usize) -> Option<ListedGrip> {
        self.listed_grip(index)
    }
}

/// The entry goes back to the number.
fn close_entry(marks: &Marks) {
    if let Some(stack) = &marks.stack {
        stack.set_visible_child_name("number");
    }
    if let Some(entry) = &marks.entry {
        entry.remove_css_class("error");
    }
    marks.error.set_visible(false);
}

/// The grip a change drags, when it is a drag.
fn drag_index(change: &Change) -> Option<usize> {
    match change.request() {
        ChangeRequest::Grip { index } => Some(*index),
        _ => None,
    }
}

/// Where a drag on `index` was last moved to.
fn drag_target(change: &Change, index: usize) -> Option<Amount> {
    if drag_index(change) != Some(index) {
        return None;
    }
    match change.detail() {
        stave_app::change::ChangeDetail::Grip(detail) => {
            // A movable grip's own param, as written (snapped); a size
            // that follows, the size the written values reach.
            let written = detail
                .changes
                .iter()
                .find(|change| change.path == detail.label)
                .map(|change| change.to);
            match change.progress() {
                Progress::Tried(_) => written.or(detail.reached).or(detail.target),
                Progress::NeedsChoice(_) | Progress::Unwritable(_) => detail.target,
            }
        }
        _ => None,
    }
}

/// The value a typed movable grip's change would write, when it is open
/// on `index`; `None` for a size that follows params (its params are
/// written, and the size they reach is shown in the panel).
fn typed_written(listed: &ListedGrip, change: &Change, index: usize) -> Option<Amount> {
    match listed.grip.action {
        GripAction::Moves(_) => drag_target(change, index),
        GripAction::Follows(_) => None,
    }
}

/// `true` when a change fits.
fn fits(progress: &Progress) -> bool {
    matches!(progress, Progress::Tried(Verdict::Fits(_)))
}

/// A label's place on the view, to keep labels off each other and off
/// the handles.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Rect {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

impl Rect {
    /// `true` when the two cover some of the same pixels.
    fn overlaps(&self, other: &Rect) -> bool {
        self.x < other.x + other.width
            && other.x < self.x + self.width
            && self.y < other.y + other.height
            && other.y < self.y + self.height
    }

    /// `true` when `point` is inside it or within `margin` of it.
    fn near(&self, (x, y): (f64, f64), margin: f64) -> bool {
        x > self.x - margin
            && x < self.x + self.width + margin
            && y > self.y - margin
            && y < self.y + self.height + margin
    }
}

/// A widget's natural size.
fn measured(widget: &impl IsA<gtk::Widget>) -> (f64, f64) {
    let (_, width, _, _) = widget.measure(gtk::Orientation::Horizontal, -1);
    let (_, height, _, _) = widget.measure(gtk::Orientation::Vertical, width);
    (f64::from(width), f64::from(height))
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::*;
    use crate::ui::edit_view::grip_text::{PanelChoice, PanelModel};
    use crate::ui::gtk_test::on_gtk_thread;
    use stave_app::session::Session;

    /// A copy of the side table built from a box (read, never written)
    /// in a temporary folder of its own.
    fn table(name: &str) -> PathBuf {
        let folder =
            std::env::temp_dir().join(format!("stave-gtk-grips-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).unwrap();
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples/side_table_from_box/table.stave");
        std::fs::copy(source, folder.join("table.stave")).unwrap();
        folder.canonicalize().unwrap()
    }

    fn send(session: &mut Session, command: EditCommand) {
        let _ = session.apply(Command::Edit(command));
    }

    fn panel(session: &Session) -> PanelModel {
        let change = session.change().expect("a change is open");
        panel_model(
            change,
            |index| session.grips()?.grips().get(index).cloned(),
            false,
            None,
        )
        .expect("the grips' panel shows it")
    }

    #[test]
    fn the_envelopes_three_handles_sit_on_three_corners() {
        let mm = |n| Length::from_mm(n);
        let bounds = stave_model::solved::WorldBox::new(
            Point::default(),
            Point::default()
                .with(Axis::Across, mm(900))
                .with(Axis::Up, mm(1800))
                .with(Axis::Deep, mm(600)),
        );
        // Front, right and above, as the camera usually stands.
        let corner = stave_geom::render::point(
            Point::default()
                .with(Axis::Across, mm(900))
                .with(Axis::Up, mm(1800))
                .with(Axis::Deep, mm(0)),
        );
        let eye = corner + (corner - stave_geom::render::point(bounds.min())) * 2.0;
        let mut handles: Vec<Vec3> = Vec::new();
        for axis in [Axis::Across, Axis::Up, Axis::Deep] {
            let (start, handle, _) = edge_dimension(&bounds, axis, Some(eye), &handles).unwrap();
            assert!((handle - start).length() > 500.0);
            handles.push(handle);
        }
        for (i, a) in handles.iter().enumerate() {
            for b in &handles[i + 1..] {
                assert!((*a - *b).length() > 500.0, "two handles share a corner");
            }
        }
    }

    #[test]
    fn labels_keep_off_each_other_and_the_handles() {
        let label = Rect {
            x: 10.0,
            y: 10.0,
            width: 80.0,
            height: 24.0,
        };
        let beside = Rect { x: 95.0, ..label };
        let over = Rect { x: 60.0, ..label };
        assert!(!label.overlaps(&beside));
        assert!(label.overlaps(&over));
        assert!(label.near((95.0, 20.0), 11.0), "a handle just off its edge");
        assert!(!label.near((120.0, 20.0), 11.0));
    }

    #[test]
    fn the_panel_says_what_a_drag_changes_and_lists_the_sizes_to_pick() {
        let folder = table("panel");
        let mut session = Session::new(folder.clone());
        let opened = stave_app::open(&folder).unwrap();
        let model = opened.model.as_ref().unwrap();
        let apron = model
            .assemblies
            .iter()
            .find(|assembly| assembly.name == "apron_front")
            .unwrap()
            .id;
        send(
            &mut session,
            EditCommand::ListGrips(GripSelection::Assembly(apron)),
        );
        let grips = session.grips().unwrap();
        // The setback moves; the apron's length follows two plain params.
        assert_eq!(grip_look(&grips.grips()[1]), GripLook::Moves);
        assert_eq!(grip_look(&grips.grips()[3]), GripLook::Follows);
        assert_eq!(
            label_words(&grips.grips()[3]),
            "360mm · follows overall_width, legs.inset"
        );

        // A movable grip: the value it is moved to, what it was, the verdict.
        send(&mut session, EditCommand::BeginDrag(1));
        let waiting = panel(&session);
        assert_eq!(waiting.verdict.words, "Drag the handle, or type a value");
        assert!(!waiting.can_apply);
        send(&mut session, EditCommand::DragTo(mm_amount(6.0).unwrap()));
        let moved = panel(&session);
        assert_eq!(moved.title, "apron_front.setback");
        assert_eq!(moved.value.as_deref(), Some("6mm"));
        assert_eq!(moved.was.as_deref(), Some("5mm"));
        assert_eq!(moved.verdict.words, "✓ It fits");
        assert!(moved.can_apply);
        // The readout at the top of the view says it in one line.
        assert_eq!(
            moved.readout.as_ref().map(|(_, value)| value.as_str()),
            Some("5 → 6 mm")
        );

        // A size that follows two plain params: both, each with its value,
        // to pick from before anything is tried.
        send(&mut session, EditCommand::BeginDrag(3));
        send(&mut session, EditCommand::DragTo(mm_amount(380.0).unwrap()));
        let choosing = panel(&session);
        let words: Vec<&str> = choosing.choices.iter().map(|c| c.words.as_str()).collect();
        assert_eq!(
            words,
            ["overall_width 500mm to 520mm", "legs.inset 25mm to 15mm"]
        );
        assert_eq!(choosing.verdict.words, "Pick which size changes");
        // Each choice is a radio row: the name, and the value on the right.
        assert_eq!(choosing.choices[0].name, "overall_width");
        assert_eq!(choosing.choices[0].value, "500 → 520 mm");
        let about = choosing.about.as_ref().unwrap();
        assert_eq!(
            (about.label.as_str(), about.words.as_str()),
            ("Follows", "overall_width, legs.inset")
        );
        assert!(!choosing.can_apply);
        let width = choosing.choices[0].param;
        send(&mut session, EditCommand::PickParam(width));
        let picked = panel(&session);
        assert!(picked.choices[0].picked);
        assert_eq!(picked.changes, ["Changes overall_width 500mm to 520mm"]);
        assert!(picked.can_apply);
        assert_eq!(
            picked.readout,
            Some(("overall_width".to_owned(), "500 → 520 mm".to_owned()))
        );
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn a_typed_value_past_the_range_is_refused_before_the_stops_are_found() {
        let folder = table("typed");
        let mut session = Session::new(folder.clone());
        send(
            &mut session,
            EditCommand::ListGrips(GripSelection::Envelope),
        );
        let grips = session.grips().unwrap();
        let index = grips
            .grips()
            .iter()
            .position(|listed| listed.grip.label == "overall_width")
            .unwrap();
        let listed = grips.grips()[index].clone();
        assert!(matches!(
            listed.stops,
            stave_app::gripping::GripStops::NotSearched
        ));
        // The window refuses it, naming the range, before trying it.
        assert_eq!(
            typed_check(&listed, mm_amount(9000.0).unwrap()).unwrap_err(),
            "Can't be 9000mm: this grip moves it between 400mm and 600mm"
        );
        // Tried anyway, the session would write the end of the range: the
        // window sees another number than the one typed, and refuses it.
        send(
            &mut session,
            EditCommand::TypeGripValue {
                grip: index,
                value: mm_amount(9000.0).unwrap(),
            },
        );
        let written = typed_written(&listed, session.change().unwrap(), index).unwrap();
        assert_eq!(amount_words(written), "600mm");
        // A value inside the range is written as typed.
        send(&mut session, EditCommand::Cancel);
        let typed = mm_amount(550.0).unwrap();
        assert_eq!(typed_check(&listed, typed), Ok(()));
        send(
            &mut session,
            EditCommand::TypeGripValue {
                grip: index,
                value: typed,
            },
        );
        let written = typed_written(&listed, session.change().unwrap(), index).unwrap();
        assert_eq!(written.raw(), typed.raw());
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn the_size_section_lists_the_sizes_as_radios_and_the_readout_is_one_line() {
        on_gtk_thread(|| {
            let panel = GripPanel::new();
            let readout = Readout::new();
            let choice = |name: &str, value: &str, picked: bool| PanelChoice {
                param: ParamId::from_index(usize::from(picked)).unwrap(),
                words: format!("{name} {value}"),
                name: name.to_owned(),
                value: value.to_owned(),
                possible: true,
                picked,
            };
            let model = PanelModel {
                title: "case.base.length".to_owned(),
                value: Some("1000mm".to_owned()),
                was: Some("900mm".to_owned()),
                about: Some(super::super::grip_text::PanelAbout {
                    label: "Follows".to_owned(),
                    words: "overall_width".to_owned(),
                }),
                changes: Vec::new(),
                choices: vec![
                    choice("overall_width", "940 → 1040 mm", true),
                    choice("case.overhang", "20 → -30 mm", false),
                ],
                stop: Some("stops at 1200mm".to_owned()),
                verdict: super::super::grip_text::PanelVerdict {
                    class: "fits",
                    words: "✓ It fits".to_owned(),
                },
                can_apply: true,
                readout: Some(("overall_width".to_owned(), "940 → 1040 mm".to_owned())),
            };
            panel.show(&model);
            assert_eq!(
                panel.words(),
                [
                    "case.base.length",
                    "1000mm",
                    "was 900mm",
                    "overall_width",
                    "stops at 1200mm"
                ]
            );
            assert_eq!(
                panel.radios(),
                [
                    ("overall_width".to_owned(), true),
                    ("case.overhang".to_owned(), false)
                ]
            );
            readout.show(model.readout.as_ref());
            assert_eq!(
                readout.words().as_deref(),
                Some("overall_width 940 → 1040 mm")
            );
            readout.show(None);
            assert_eq!(readout.words(), None);
        });
    }

    #[test]
    fn a_grips_options_menu_is_freed_once_it_is_let_go() {
        on_gtk_thread(|| {
            use relm4::{Component, ComponentController};

            let application = adw::Application::new(
                Some("org.stave.GripsTest.MenuFreed"),
                gio::ApplicationFlags::NON_UNIQUE,
            );
            if let Err(error) = application.register(gio::Cancellable::NONE) {
                panic!("the test's application registers: {error}");
            }
            let folder = table("menu-freed");
            let options = crate::ui::Options {
                layout: Some(std::env::temp_dir().join("stave-grips-test-no-layout")),
                ..crate::ui::Options::default()
            };
            let builder = Window::builder();
            let root = builder.widget().clone();
            let widgets =
                crate::ui::build_widgets(&root, &application, &folder, &gio::Menu::new(), &options)
                    .unwrap();
            let controller = builder
                .launch(crate::ui::WindowInit {
                    widgets,
                    folder,
                    options,
                })
                .detach();
            crate::ui::gtk_test::settle();
            let width = Ident::new("width", stave_lang::ast::Span::default()).unwrap();
            let switches = [SolverSwitch {
                param: width,
                value: super::super::grip_text::mm_amount(100.0).unwrap(),
                solved: false,
            }];
            let menu = controller.model().switch_menu(0, &switches);
            let holder = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            holder.append(&menu);
            let weak = menu.downgrade();
            holder.remove(&menu);
            drop(menu);
            crate::ui::gtk_test::settle();
            // Its switch and button sit in its own popover: holding the
            // menu from their handlers would keep it alive for good.
            assert!(weak.upgrade().is_none(), "the menu is freed");
            controller.widget().close();
            drop(controller);
            crate::ui::gtk_test::settle();
        });
    }
}
