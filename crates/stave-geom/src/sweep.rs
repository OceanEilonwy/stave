//! Motion sweeps: how far a drawer slides, or a door turns, before one
//! body first runs into another (grammar, "Scenes": every motion is
//! checked through its whole travel, not only at its ends; "One collision
//! model": every body, part or hardware member, is swept the same way).
//!
//! A [`Path`] is an assembly's mechanism ready to sweep
//! ([`crate::mechanism`]), and a [`Mover`] says how it moves one body: not
//! at all (where a scene places it), with the assembly as one piece, or as
//! a member the mechanism places. [`first_contact`] picks the method from
//! the motion's shape ([`Shape`]), never from a kind:
//!
//! - **Slides** are one-dimensional and exact. The moving solid travels
//!   along a world direction; a fixed box is in its way when the two
//!   overlap by more than the contact tolerance across that direction. The
//!   first contact is then a subtraction in whole nanometres: the gap
//!   ahead.
//! - **A turn about one fixed line** reduces to two dimensions. Both
//!   solids are cut into convex pieces in the plane across the line
//!   ([`crate::plane`]), shrunk by half the contact tolerance so that
//!   touching is not hitting. Contact can only begin where a corner of one
//!   polygon crosses an edge of the other: each moving corner turns on a
//!   circle about the pivot, met by each fixed edge, and each fixed corner
//!   meets each moving edge on its own circle. Those angles are found in
//!   closed form; between two of them the polygons either overlap all the
//!   way or not at all, so testing one angle in each span finds the first
//!   contact.
//! - **Any other motion** (a cup hinge's linkage, and every member a
//!   mechanism places) moves bodies differently, so there is no closed
//!   form. The same pieces are swept by **conservative advancement** in
//!   the mechanism's plane: at each step the distance between two pieces
//!   is measured, and the motion goes on by that distance over the fastest
//!   their corners can close it (each member's turn and offset speed,
//!   found by the solver over the whole range), so no contact can be
//!   stepped over; near contact the step never falls below a quarter of
//!   the contact tolerance's worth of movement. The first overlap is then
//!   bisected to far below a micro-degree (or a nanometre).
//!
//! **Sweeps see cuts**: a part's boxes are taken less the round holes
//! whose axis lies in the plane, as their narrowest chord over the width
//! the other piece shares with them ([`crate::plane`]), so a cup's arm and
//! links can move in its bore. In a turn about one fixed line, a round
//! hole whose axis lies on the line is taken out as itself (a piece that
//! stays inside it all the way round is clear of the wood it is drilled
//! in), and a round shape turning about its own axis sweeps only itself,
//! so it is left out of the sweep: a pivot bolt in its own hole. Other
//! holes and angled cuts are left as wood, any other rod is the box
//! around it, and a fixed solid turned about another axis is the outline
//! of its corners: each finds more wood than there is, never less.
//! Angles are floats here (`f64` nanometres and
//! radians, converted in [`render`](crate::render)); results are rounded
//! down to the micro-degree.

use std::f64::consts::TAU;

use glam::{DMat4, DVec2, DVec3};
use stave_lang::placement::{Axis, Direction};
use stave_lang::units::{Angle, Length};
use stave_model::mechanism::TurnLine;
use stave_model::mechanism::{DriveIndex, DriveValue, Mechanism, MemberIndex};
use stave_model::solved::{Point, PoseValue, WorldBox};

use crate::collide::{
    CONTACT_TOLERANCE, Placement, WorldPrism, WorldRod, WorldSolid, overlaps_placed, placed_bounds,
    translate_box,
};
use crate::mechanism::MechanismPath;
use crate::planar::MechanismError;
use crate::plane::{
    Bores, Motion2, Pair, Piece, Plane, Room, corners_of, intersect, piece_depth, piece_pairs,
    polygon_distance,
};
use crate::render;
use stave_model::mechanism::MotionShape;

/// The boxes of a solid's wood: its blocks and the boxes around its rods
/// and prisms.
pub(crate) fn wood_boxes(solid: &WorldSolid) -> impl Iterator<Item = WorldBox> + '_ {
    solid
        .boxes()
        .iter()
        .copied()
        .chain(solid.rods().iter().map(WorldRod::bounds))
        .chain(solid.prisms().iter().filter_map(WorldPrism::bounds))
        .chain(solid.slant_bounds())
}

/// A fixed solid's boxes at its placement: exact when it only moved, the
/// box around each turned box otherwise.
pub(crate) fn placed_boxes(solid: &WorldSolid, placement: &Placement) -> Vec<WorldBox> {
    wood_boxes(solid)
        .map(|b| match placement {
            Placement::Exact(offset) => translate_box(&b, *offset),
            Placement::Turned(_) => placed_bounds(&b, placement),
        })
        .collect()
}

/// Overlap of two ranges, in nanometres (negative when apart).
pub(crate) fn shared(a: &WorldBox, b: &WorldBox, axis: Axis) -> i64 {
    let low = a.min().get(axis).max(b.min().get(axis)).nm();
    let high = a.max().get(axis).min(b.max().get(axis)).nm();
    high.saturating_sub(low)
}

/// Where a sliding solid first hits a fixed one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct SlideHit {
    /// How far it travels before it touches.
    pub distance: Length,
    /// How much smaller the moving piece must be, on the axis `across`,
    /// to pass the fixed one: the smaller of the two overlaps across the
    /// direction of travel that the moving piece only partly takes up.
    /// `None` when it lies wholly in the way on both axes.
    pub shrink: Option<(Length, Axis)>,
}

/// How far `moving` can travel `toward` before it first touches `fixed`
/// (placed at `fixed_at`) and would go on into it; `None` when it travels
/// the whole `travel` clear. Pieces that already overlap at the start are
/// left out: the static checks report them.
pub fn slide_contact(
    moving: &WorldSolid,
    fixed: &WorldSolid,
    fixed_at: &Placement,
    toward: Direction,
    travel: Length,
) -> Option<SlideHit> {
    let axis = toward.axis();
    let [first, second] = axis.others();
    let tolerance = CONTACT_TOLERANCE.nm();
    let fixed_boxes = placed_boxes(fixed, fixed_at);
    let mut best: Option<SlideHit> = None;
    for m in wood_boxes(moving) {
        for f in &fixed_boxes {
            let (a, b) = (shared(&m, f, first), shared(&m, f, second));
            if a <= tolerance || b <= tolerance {
                continue;
            }
            // To pass, the moving piece must end short of the fixed one on
            // one side or the other along an axis across its travel.
            let partial = |axis: Axis| {
                let over_high = m
                    .max()
                    .get(axis)
                    .nm()
                    .saturating_sub(f.min().get(axis).nm());
                let over_low = f
                    .max()
                    .get(axis)
                    .nm()
                    .saturating_sub(m.min().get(axis).nm());
                let amount = over_high.min(over_low);
                (amount < m.size(axis).nm()).then_some((Length::from_nm(amount), axis))
            };
            let shrink = match (partial(first), partial(second)) {
                (Some(x), Some(y)) => Some(if x.0 <= y.0 { x } else { y }),
                (x, y) => x.or(y),
            };
            let gap = if toward.is_positive() {
                f.min()
                    .get(axis)
                    .nm()
                    .saturating_sub(m.max().get(axis).nm())
            } else {
                m.min()
                    .get(axis)
                    .nm()
                    .saturating_sub(f.max().get(axis).nm())
            };
            // Overlapping already (reported at rest), or behind it.
            if gap < -tolerance || shared(&m, f, axis) > tolerance {
                continue;
            }
            let hit = SlideHit {
                distance: Length::from_nm(gap.max(0)),
                shrink,
            };
            // It goes deeper than the tolerance once past the touch point.
            if gap.saturating_add(tolerance) < travel.nm() && best.is_none_or(|b| hit < b) {
                best = Some(hit);
            }
        }
    }
    // A round shape that starts inside a hole of the other solid (the
    // room a cut made for it) overlaps the block the hole is drilled in,
    // and is clear of its wood only while it stays in the hole: once it
    // slides out, into wood, it hits. Exact for a solid placed by a
    // translation; the moving solid's holes meet the fixed one's rods
    // sliding the other way.
    if let Placement::Exact(offset) = fixed_at {
        let fixed_holes: Vec<WorldRod> = fixed
            .holes()
            .iter()
            .map(|hole| rod_translated(hole, *offset))
            .collect();
        let fixed_blocks: Vec<WorldBox> = fixed
            .boxes()
            .iter()
            .map(|block| translate_box(block, *offset))
            .collect();
        let fixed_rods: Vec<WorldRod> = fixed
            .rods()
            .iter()
            .map(|rod| rod_translated(rod, *offset))
            .collect();
        let holes_of_fixed = fixed_holes.as_slice();
        let pairs = moving
            .rods()
            .iter()
            .flat_map(|rod| {
                fixed_blocks
                    .iter()
                    .map(move |block| (*rod, holes_of_fixed, *block, toward))
            })
            .chain(fixed_rods.iter().flat_map(|rod| {
                moving
                    .boxes()
                    .iter()
                    .map(move |block| (*rod, moving.holes(), *block, toward.opposite()))
            }));
        for (rod, holes, block, way) in pairs {
            let Some(distance) = room_exit(&rod, holes, &block, way) else {
                continue;
            };
            let hit = SlideHit {
                distance: Length::from_nm(distance),
                shrink: None,
            };
            if distance.saturating_add(tolerance) < travel.nm() && best.is_none_or(|b| hit < b) {
                best = Some(hit);
            }
        }
    }
    best
}

/// A rod moved by an exact offset.
fn rod_translated(rod: &WorldRod, by: Point) -> WorldRod {
    let [first, second] = rod.axis.others();
    let moved =
        |value: Length, axis: Axis| Length::from_nm(value.nm().saturating_add(by.get(axis).nm()));
    WorldRod {
        centre: [moved(rod.centre[0], first), moved(rod.centre[1], second)],
        start: moved(rod.start, rod.axis),
        end: moved(rod.end, rod.axis),
        ..*rod
    }
}

/// How far `rod` slides `toward` before it leaves the round holes of
/// `holes` it starts inside, out into the wood of `block` (the block they
/// are drilled in), in whole nanometres; `None` when it doesn't start
/// overlapping the block inside one of them (then it is clear of the
/// block, or the static checks report it), or never meets wood beyond
/// the hole. A hole that it slides straight on into (one of a row close
/// enough to hold it on the way) carries it on.
fn room_exit(
    rod: &WorldRod,
    holes: &[WorldRod],
    block: &WorldBox,
    toward: Direction,
) -> Option<i64> {
    let tolerance = render::nm(CONTACT_TOLERANCE);
    let axis = rod.axis;
    let [first, second] = axis.others();
    let bounds = rod.bounds();
    let clear = CONTACT_TOLERANCE.nm();
    if Axis::ALL
        .into_iter()
        .any(|on| shared(&bounds, block, on) <= clear)
    {
        return None;
    }
    let radius = render::nm(rod.diameter) / 2.0;
    let sign = if toward.is_positive() { 1.0 } else { -1.0 };
    let (low, high) = (
        render::nm(rod.start.max(block.min().get(axis))),
        render::nm(rod.end.min(block.max().get(axis))),
    );
    let centre = [render::nm(rod.centre[0]), render::nm(rod.centre[1])];
    let hole_centre = |hole: &WorldRod| [render::nm(hole.centre[0]), render::nm(hole.centre[1])];
    // Whether the rod, slid `t` along, lies inside `hole` where it is in
    // the block.
    let holds = |hole: &WorldRod, t: f64| -> bool {
        if hole.axis != axis {
            return false;
        }
        let shift = |on: Axis| if toward.axis() == on { sign * t } else { 0.0 };
        let (from, to) = (low + shift(axis), high + shift(axis));
        let along =
            render::nm(hole.start) <= from + tolerance && render::nm(hole.end) >= to - tolerance;
        let [h0, h1] = hole_centre(hole);
        let d0 = centre[0] + shift(first) - h0;
        let d1 = centre[1] + shift(second) - h1;
        along && (d0 * d0 + d1 * d1).sqrt() + radius <= render::nm(hole.diameter) / 2.0 + tolerance
    };
    let mut hole = holes.iter().find(|hole| holds(hole, 0.0))?;
    let mut at = 0.0;
    let mut exit = at;
    for _ in 0..=holes.len() {
        let room = render::nm(hole.diameter) / 2.0 + tolerance;
        exit = if toward.axis() == axis {
            // Along its own axis: until its leading end reaches the
            // hole's end.
            if toward.is_positive() {
                render::nm(hole.end) - render::nm(rod.end)
            } else {
                render::nm(rod.start) - render::nm(hole.start)
            }
        } else {
            let [h0, h1] = hole_centre(hole);
            let (ahead, beside) = if toward.axis() == first {
                (sign * (centre[0] - h0), centre[1] - h1)
            } else {
                (sign * (centre[1] - h1), centre[0] - h0)
            };
            let reach = room - radius;
            (reach * reach - beside * beside).max(0.0).sqrt() - ahead
        }
        .max(at);
        let onward = exit + tolerance;
        match holes
            .iter()
            .find(|next| !std::ptr::eq(*next, hole) && holds(next, onward))
        {
            Some(next) => {
                hole = next;
                at = onward;
            }
            None => break,
        }
    }
    // Wood beyond: the block reaches past where the rod's leading side
    // leaves the hole.
    let leading = if toward.axis() == axis {
        let end = if toward.is_positive() {
            rod.end
        } else {
            rod.start
        };
        render::nm(end) + sign * exit
    } else {
        let index = usize::from(toward.axis() != first);
        centre.get(index).copied().unwrap_or(0.0) + sign * (exit + radius)
    };
    let face = if toward.is_positive() {
        block.max().get(toward.axis())
    } else {
        block.min().get(toward.axis())
    };
    if sign * (render::nm(face) - leading) <= tolerance {
        return None;
    }
    render::length_from_nm(exit.floor()).map(|length| length.nm())
}

/// The angles in `(0, limit]` at which the circle of radius `|p|` about the
/// origin meets the segment `a..b`, measured from `p` counter-clockwise
/// when `forward`, else from the meeting point to `p`.
pub(crate) fn crossings(
    p: DVec2,
    a: DVec2,
    b: DVec2,
    forward: bool,
    limit: f64,
    out: &mut Vec<f64>,
) {
    let radius2 = p.length_squared();
    let d = b - a;
    let (qa, qb, qc) = (d.dot(d), 2.0 * a.dot(d), a.dot(a) - radius2);
    if qa < 1e-12 {
        return;
    }
    let discriminant = qb * qb - 4.0 * qa * qc;
    if discriminant < 0.0 {
        return;
    }
    let root = discriminant.sqrt();
    for t in [(-qb - root) / (2.0 * qa), (-qb + root) / (2.0 * qa)] {
        if !(-1e-9..=1.0 + 1e-9).contains(&t) {
            continue;
        }
        let q = a + d * t;
        let (from, to) = if forward { (p, q) } else { (q, p) };
        let angle = (to.to_angle() - from.to_angle()).rem_euclid(TAU);
        if angle > 0.0 && angle <= limit {
            out.push(angle);
        }
    }
}

/// The first angle in `(0, limit]` at which the moving polygon, turning
/// counter-clockwise about the origin, overlaps the fixed one; `None` when
/// it turns the whole way clear. Polygons overlapping at the start are
/// left out (reported by the static checks).
#[cfg(test)]
fn turn_first(moving: &[DVec2], fixed: &[DVec2], limit: f64) -> Option<f64> {
    let moving = Piece {
        corners: moving.to_vec(),
        low: 0.0,
        high: 0.0,
        round: None,
    };
    let fixed = Piece {
        corners: fixed.to_vec(),
        low: 0.0,
        high: 0.0,
        round: None,
    };
    turn_first_in(&moving, &fixed, None, limit)
}

/// [`turn_first`] for two pieces, one of which may start inside a round
/// hole of the other's solid (its room, see [`Room`]): they overlap only
/// where the polygons do and the piece is out of the hole. The angles at
/// which it leaves the hole are found in closed form with the rest.
fn turn_first_in(moving: &Piece, fixed: &Piece, room: Option<Room>, limit: f64) -> Option<f64> {
    let overlapping = |angle: f64| {
        let turn = Motion2 {
            rotation: DVec2::from_angle(angle),
            offset: DVec2::ZERO,
        };
        let turned = moving.moved(turn);
        intersect(&turned.corners, &fixed.corners)
            && !room.is_some_and(|room| room.holds(moving, fixed, turn, Motion2::STILL))
    };
    if overlapping(0.0) {
        return None;
    }
    let (moving_corners, fixed_corners) = (moving.corners.as_slice(), fixed.corners.as_slice());
    let mut events = vec![0.0, limit];
    if let Some(room) = room {
        room_exits(moving, fixed, &room, limit, &mut events);
    }
    for (polygon, other, forward) in [
        (moving_corners, fixed_corners, true),
        (fixed_corners, moving_corners, false),
    ] {
        let n = other.len();
        for p in polygon {
            for i in 0..n {
                crossings(
                    *p,
                    other[i],
                    other[(i + 1) % n],
                    forward,
                    limit,
                    &mut events,
                );
            }
        }
    }
    events.sort_by(f64::total_cmp);
    events.dedup_by(|a, b| (*a - *b).abs() < 1e-12);
    for pair in events.windows(2) {
        let &[from, to] = pair else { continue };
        if overlapping((from + to) / 2.0) {
            return Some(from);
        }
    }
    None
}

/// The angles in `(0, limit]` at which a piece turning about the origin
/// (or the hole, when the hole is in the turning piece's solid) brings
/// the round piece to the side of its room: where `|turned(u) - v|` is the
/// room's radius less the piece's, `u` the centre that turns and `v` the
/// one that stays.
fn room_exits(moving: &Piece, fixed: &Piece, room: &Room, limit: f64, out: &mut Vec<f64>) {
    let (round, hole_turns) = if room.in_second {
        (moving.round, false)
    } else {
        (fixed.round, true)
    };
    let Some((centre, radius)) = round else {
        return;
    };
    let reach = room.radius - radius;
    let (u, v) = if hole_turns {
        (room.centre, centre)
    } else {
        (centre, room.centre)
    };
    let (lu, lv) = (u.length(), v.length());
    if lu * lv < 1e-9 {
        return;
    }
    let cosine = (lu * lu + lv * lv - reach * reach) / (2.0 * lu * lv);
    if !(-1.0..=1.0).contains(&cosine) {
        return;
    }
    // turned(u)·v = |u||v|cos(θ + α), α the angle from v to u.
    let alpha = u.to_angle() - v.to_angle();
    let spread = cosine.acos();
    for angle in [spread - alpha, -spread - alpha] {
        let angle = angle.rem_euclid(TAU);
        if angle > 0.0 && angle <= limit {
            out.push(angle);
        }
    }
}

/// A motion ready to sweep (grammar, "Scenes"): an assembly's mechanism
/// solved in floats, with its exact shape where it has one (a turn about
/// one fixed line, a travel along one rail).
#[derive(Debug, Clone, PartialEq)]
pub struct Path {
    mechanism: MechanismPath,
}

/// How a body moves in a sweep.
#[derive(Debug, Clone, Copy)]
pub enum Mover<'a> {
    /// It stays where it is placed.
    Fixed(Placement),
    /// It moves with the path from rest, as one piece with the assembly
    /// (with the mechanism's driven member).
    Rigid(&'a Path),
    /// It is (or rides on) that member of the path's mechanism, from rest:
    /// the mechanism places it as the assembly moves.
    Member(&'a Path, MemberIndex),
    /// It moves with the path as one piece with the assembly, from where
    /// an assembly inside it holds it (that placement, from rest): a tray
    /// tilted on its hinge in a drawer that slides. Swept by conservative
    /// advancement, the body as that placement turns it.
    Carried(&'a Path, Placement),
}

impl PartialEq for Mover<'_> {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Mover::Fixed(a), Mover::Fixed(b)) => a == b,
            (Mover::Rigid(a), Mover::Rigid(b)) => std::ptr::eq(*a, *b),
            (Mover::Member(a, x), Mover::Member(b, y)) => std::ptr::eq(*a, *b) && x == y,
            (Mover::Carried(a, x), Mover::Carried(b, y)) => std::ptr::eq(*a, *b) && x == y,
            (Mover::Fixed(_) | Mover::Rigid(_) | Mover::Member(..) | Mover::Carried(..), _) => {
                false
            }
        }
    }
}

impl Mover<'_> {
    fn path(&self) -> Option<&Path> {
        match self {
            Mover::Fixed(_) => None,
            Mover::Rigid(path) | Mover::Member(path, _) | Mover::Carried(path, _) => Some(path),
        }
    }

    /// Whether it moves with its path as one piece with the assembly.
    fn rigid(&self) -> bool {
        match self {
            Mover::Rigid(_) | Mover::Carried(..) => true,
            Mover::Fixed(_) | Mover::Member(..) => false,
        }
    }
}

/// Where a motion first brings two bodies into each other.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reach {
    /// A slide, this far out.
    Distance(SlideHit),
    /// A hinge, opened this far (rounded down to the micro-degree).
    Angle(Angle),
}

/// How deep two bodies go into each other at the worst sampled point of a
/// motion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Depth {
    /// How far one would have to move to clear the other.
    pub depth: Length,
    /// Where on the motion: an angle or a distance.
    pub at: PoseValue,
}

impl Path {
    /// An assembly's mechanism, run through its range and ready to sweep;
    /// the solver's error when it can't be (see [`MechanismPath::new`]).
    pub fn new(mechanism: &Mechanism) -> Result<Path, MechanismError> {
        Ok(Path {
            mechanism: MechanismPath::new(mechanism)?,
        })
    }

    /// One drive of an assembly's mechanism run through its range with
    /// every other drive held at `held` (one value for each drive,
    /// absolute), ready to sweep (see [`MechanismPath::sweeping`]).
    pub fn sweeping(
        mechanism: &Mechanism,
        drive: DriveIndex,
        held: &[DriveValue],
    ) -> Result<Path, MechanismError> {
        Ok(Path {
            mechanism: MechanismPath::sweeping(mechanism, drive, held)?,
        })
    }

    /// The mechanism, posed in floats.
    pub fn mechanism(&self) -> &MechanismPath {
        &self.mechanism
    }

    /// The plane the mechanism moves in.
    pub(crate) fn plane(&self) -> Plane {
        Plane::of_mechanism(&self.mechanism)
    }

    /// How far the motion reaches from closed, in radians or nanometres.
    pub(crate) fn limit(&self) -> f64 {
        self.mechanism.limit()
    }

    /// Where a mover has moved its pieces in the plane, `amount` along the
    /// motion.
    pub(crate) fn motion2(&self, mover: &Mover<'_>, amount: f64) -> Option<Motion2> {
        let member = match mover {
            Mover::Fixed(_) => return Some(Motion2::STILL),
            Mover::Rigid(_) | Mover::Carried(..) => self.mechanism.moving(),
            Mover::Member(_, member) => *member,
        };
        let (rotation, offset) = self.mechanism.motion(member, amount)?;
        Some(Motion2 { rotation, offset })
    }

    /// The fastest any corner of a piece can move per unit of the motion.
    pub(crate) fn speed(&self, mover: &Mover<'_>, piece: &Piece) -> f64 {
        let member = match mover {
            Mover::Fixed(_) => return 0.0,
            Mover::Rigid(_) | Mover::Carried(..) => self.mechanism.moving(),
            Mover::Member(_, member) => *member,
        };
        self.mechanism
            .speed(member, piece.radius_about(DVec2::ZERO))
            .unwrap_or(f64::INFINITY)
    }

    /// The exact shape of the motion as the moving bodies see it at rest:
    /// with the other drives held away from closed (a pin raised in its
    /// slot), the bodies start where [`MechanismPath::offset`] puts them,
    /// so a turn's line is taken back by that much, and everything else
    /// is placed back by it to meet them ([`Path::relative`]).
    pub(crate) fn relative_shape(&self) -> MotionShape {
        match (self.mechanism.shape(), self.mechanism.offset()) {
            (MotionShape::Turn { line, opening }, Some(offset)) => MotionShape::Turn {
                line: TurnLine {
                    through: line.through.minus(offset),
                    ..line
                },
                opening,
            },
            (shape @ MotionShape::Travel { .. }, Some(_)) => shape,
            (MotionShape::Turn { .. } | MotionShape::Travel { .. }, None)
            | (MotionShape::General, _) => MotionShape::General,
        }
    }

    /// A fixed body's placement as the moving bodies at rest see it: moved
    /// back by where the held drives put them.
    pub(crate) fn relative(&self, placement: &Placement) -> Placement {
        match self.mechanism.offset() {
            Some(offset) if offset != Point::default() => match placement {
                Placement::Exact(at) => Placement::Exact(at.minus(offset)),
                Placement::Turned(matrix) => Placement::Turned(
                    DMat4::from_translation(-crate::collide::point_vector(offset)) * *matrix,
                ),
            },
            Some(_) | None => *placement,
        }
    }

    /// The box a body can reach as the path moves it.
    pub fn reach(&self, solid: &WorldSolid, mover: &Mover<'_>) -> Option<WorldBox> {
        let offset = self.mechanism.offset().unwrap_or_default();
        if let Mover::Carried(path, held) = mover {
            // The box around it where the assembly inside holds it, moved
            // as one piece with this one.
            let held_box = WorldSolid::from_boxes(vec![placed_bounds(&solid.bounds()?, held)]);
            return self.reach(&held_box, &Mover::Rigid(path));
        }
        match (self.relative_shape(), mover) {
            (_, Mover::Fixed(placement)) => Some(placed_bounds(&solid.bounds()?, placement)),
            (MotionShape::Turn { line: pivot, .. }, Mover::Rigid(_)) => {
                Some(translate_box(&turn_reach(solid, &pivot)?, offset))
            }
            (MotionShape::Travel { toward, travel }, Mover::Rigid(_)) => {
                Some(translate_box(&slide_reach(solid, toward, travel)?, offset))
            }
            (MotionShape::General, Mover::Rigid(_)) => {
                let member = self.mechanism.moving();
                sampled_reach(solid, self, mover, |amount| {
                    self.mechanism.matrix(member, amount)
                })
            }
            (_, Mover::Member(_, member)) => sampled_reach(solid, self, mover, |amount| {
                self.mechanism.matrix(*member, amount)
            }),
            // Taken as the box where it is held, above.
            (_, Mover::Carried(..)) => None,
        }
    }

    /// The placement a mover gives its body at a pose value of the path.
    fn placement(&self, mover: &Mover<'_>, value: PoseValue) -> Placement {
        match mover {
            Mover::Fixed(placement) => *placement,
            Mover::Rigid(_) => Placement::of_mechanisms(&[(&self.mechanism, value)], None),
            Mover::Member(_, member) => {
                Placement::of_mechanisms(&[(&self.mechanism, value)], Some(*member))
            }
            Mover::Carried(_, held) => {
                Placement::of_mechanisms(&[(&self.mechanism, value)], None).after(held)
            }
        }
    }
}

/// The box a body reaches over a path, from its corners at 180 steps of
/// the motion (a degree or less each for a hinge), grown by the most it
/// can move within half a step.
fn sampled_reach(
    solid: &WorldSolid,
    path: &Path,
    mover: &Mover<'_>,
    matrix: impl Fn(f64) -> Option<DMat4>,
) -> Option<WorldBox> {
    let bounds = solid.bounds()?;
    let corners = corners_of(&bounds);
    let plane = path.plane();
    let flat = Piece {
        corners: corners.iter().map(|c| plane.project(*c)).collect(),
        low: 0.0,
        high: 0.0,
        round: None,
    };
    let count: u32 = 180;
    let step = path.limit() / f64::from(count);
    let grow = path.speed(mover, &flat) * step / 2.0 + 1.0;
    let mut low = DVec3::splat(f64::INFINITY);
    let mut high = DVec3::splat(f64::NEG_INFINITY);
    for k in 0..=count {
        let Some(matrix) = matrix(step * f64::from(k)) else {
            continue;
        };
        for corner in &corners {
            let placed = matrix.transform_point3(*corner);
            low = low.min(placed);
            high = high.max(placed);
        }
    }
    let point = |v: DVec3, round: fn(f64) -> f64| -> Option<Point> {
        Some(Point {
            across: render::length_from_nm(round(v.x))?,
            up: render::length_from_nm(round(v.y))?,
            deep: render::length_from_nm(round(v.z))?,
        })
    };
    Some(WorldBox::new(
        point(low - DVec3::splat(grow), f64::floor)?,
        point(high + DVec3::splat(grow), f64::ceil)?,
    ))
}

/// The pieces of two bodies in a turning path's plane, each where its
/// mover starts it (a moving body at rest, a fixed one at its placement).
fn start_pairs(
    plane: &Plane,
    a: &WorldSolid,
    ma: &Mover<'_>,
    b: &WorldSolid,
    mb: &Mover<'_>,
) -> Vec<Pair> {
    let matrix = |mover: &Mover<'_>| match mover {
        Mover::Fixed(placement) | Mover::Carried(_, placement) => placement.matrix(),
        Mover::Rigid(_) | Mover::Member(..) => DMat4::IDENTITY,
    };
    // The pieces move other than about one fixed line: a hole along the
    // axis keeps nothing apart all the motion long, but a piece that
    // starts inside one is clear while it stays in it (its room).
    piece_pairs(plane, a, &matrix(ma), b, &matrix(mb), Bores::Wood)
}

/// The first angle in `(0, limit]` at which two pieces, each moved by its
/// mover, overlap; `None` when they stay clear (or overlap at the start,
/// which the static checks report). Swept by conservative advancement:
/// each step is no more than the distance between them over the fastest
/// their corners can close it, never less than a quarter of the contact
/// tolerance's worth; the first overlap is then bisected.
fn advance(path: &Path, pair: &Pair, ma: &Mover<'_>, mb: &Mover<'_>, limit: f64) -> Option<f64> {
    let (a, b) = (&pair.first, &pair.second);
    let at = |theta: f64| -> Option<(Piece, Piece)> {
        Some((
            a.moved(path.motion2(ma, theta)?),
            b.moved(path.motion2(mb, theta)?),
        ))
    };
    // They overlap where their polygons do, unless one is still inside
    // the round hole it started in (its room).
    let overlapping = |x: &Piece, y: &Piece| {
        intersect(&x.corners, &y.corners)
            && !pair
                .room
                .is_some_and(|room| room.holds(x, y, Motion2::STILL, Motion2::STILL))
    };
    // How far apart they are, for the next step: inside its room, how far
    // the piece is from the hole's side.
    let apart = |x: &Piece, y: &Piece| match pair.room {
        Some(room) if intersect(&x.corners, &y.corners) => room.clearance(x, y).max(0.0),
        Some(_) | None => polygon_distance(&x.corners, &y.corners),
    };
    // Where the motion starts: at rest, or where the held drives put it.
    let (mut here_a, mut here_b) = if path.mechanism.held_closed() {
        (a.clone(), b.clone())
    } else {
        at(0.0)?
    };
    if overlapping(&here_a, &here_b) {
        return None;
    }
    let speed = path.speed(ma, a) + path.speed(mb, b);
    if speed <= 0.0 {
        return None;
    }
    let smallest = render::nm(CONTACT_TOLERANCE) / 4.0 / speed;
    let mut theta = 0.0;
    loop {
        let step = (apart(&here_a, &here_b) / speed).max(smallest);
        let next = (theta + step).min(limit);
        let (there_a, there_b) = at(next)?;
        if overlapping(&there_a, &there_b) {
            let (mut clear, mut hit) = (theta, next);
            for _ in 0..60 {
                let middle = (clear + hit) / 2.0;
                let (x, y) = at(middle)?;
                if overlapping(&x, &y) {
                    hit = middle;
                } else {
                    clear = middle;
                }
            }
            return Some(clear);
        }
        if next >= limit {
            return None;
        }
        theta = next;
        (here_a, here_b) = (there_a, there_b);
    }
}

/// Where a motion first brings body `a` into body `b`, each moved by its
/// mover from where it starts (a moving body at rest, a fixed one at its
/// placement); `None` when they stay clear over the whole motion, or when
/// nothing moves one against the other. Pieces already overlapping at the
/// start are left out: the static checks report them.
///
/// The motion's shape picks the method: a slide is exact, a turn about one
/// fixed line is closed form, and a linkage, or a member its mechanism
/// turns, is swept by conservative advancement.
pub fn first_contact(
    a: &WorldSolid,
    ma: Mover<'_>,
    b: &WorldSolid,
    mb: Mover<'_>,
) -> Option<Reach> {
    let path = ma.path().or_else(|| mb.path())?;
    // One rigid motion moves both: they never move against each other.
    if ma.rigid() && mb.rigid() {
        return None;
    }
    if ma == mb {
        return None;
    }
    // A body an inner assembly holds turned has no exact shape here.
    let carried = matches!(ma, Mover::Carried(..)) || matches!(mb, Mover::Carried(..));
    let shape = if carried {
        MotionShape::General
    } else {
        path.relative_shape()
    };
    // With the other drives held, the moving bodies start where they put
    // them: the fast paths meet them with the fixed body placed back.
    match (shape, &ma, &mb) {
        (MotionShape::Travel { toward, travel }, Mover::Rigid(_), Mover::Fixed(placement)) => {
            slide_contact(a, b, &path.relative(placement), toward, travel).map(Reach::Distance)
        }
        (MotionShape::Travel { toward, travel }, Mover::Fixed(placement), Mover::Rigid(_)) => {
            slide_contact(b, a, &path.relative(placement), toward, travel).map(Reach::Distance)
        }
        (
            MotionShape::Turn {
                line: pivot,
                opening,
            },
            Mover::Rigid(_),
            Mover::Fixed(placement),
        ) => turn_contact(a, b, &path.relative(placement), &pivot, opening).map(Reach::Angle),
        (
            MotionShape::Turn {
                line: pivot,
                opening,
            },
            Mover::Fixed(placement),
            Mover::Rigid(_),
        ) => turn_contact(b, a, &path.relative(placement), &pivot, opening).map(Reach::Angle),
        _ => {
            let plane = path.plane();
            let limit = path.limit();
            let mut best: Option<f64> = None;
            for pair in start_pairs(&plane, a, &ma, b, &mb) {
                if let Some(amount) = advance(path, &pair, &ma, &mb, best.unwrap_or(limit))
                    && best.is_none_or(|b| amount < b)
                {
                    best = Some(amount);
                }
            }
            let amount = best?;
            if path.mechanism.turns() {
                render::angle_from_radians_floor(amount).map(Reach::Angle)
            } else {
                render::length_from_nm(amount.floor()).map(|distance| {
                    Reach::Distance(SlideHit {
                        distance,
                        shrink: None,
                    })
                })
            }
        }
    }
}

/// How deep body `a` goes into body `b` at the worst of `samples + 1`
/// evenly spaced points of a motion, each moved by its mover (grammar,
/// "One collision model"); `None` when they never go deeper than the
/// contact tolerance, or when one rigid motion moves both (the static
/// checks measure what they share). Holes in the plane are taken out as
/// in the sweeps.
pub fn deepest_overlap(
    a: &WorldSolid,
    ma: Mover<'_>,
    b: &WorldSolid,
    mb: Mover<'_>,
    samples: u32,
) -> Option<Depth> {
    let path = ma.path().or_else(|| mb.path())?;
    // One rigid motion moves both: what they share at rest is all they
    // ever share, and the static checks measure it.
    if ma.rigid() && mb.rigid() {
        return None;
    }
    let samples = samples.max(1);
    let mut worst: Option<(f64, PoseValue)> = None;
    let rigid_only = [&ma, &mb]
        .iter()
        .all(|mover| matches!(mover, Mover::Fixed(_) | Mover::Rigid(_)));
    let travel = match path.mechanism.shape() {
        MotionShape::Travel { travel, .. } if rigid_only => Some(travel),
        MotionShape::Travel { .. } | MotionShape::Turn { .. } | MotionShape::General => None,
    };
    if travel.is_none() {
        let plane = path.plane();
        let limit = path.limit();
        let pairs = start_pairs(&plane, a, &ma, b, &mb);
        for k in 0..=samples {
            let amount = limit * f64::from(k) / f64::from(samples);
            let (Some(moved_a), Some(moved_b)) =
                (path.motion2(&ma, amount), path.motion2(&mb, amount))
            else {
                continue;
            };
            for Pair {
                first: p,
                second: q,
                ..
            } in &pairs
            {
                let Some((depth, _)) = piece_depth(&p.moved(moved_a), &q.moved(moved_b)) else {
                    continue;
                };
                if worst.is_none_or(|(w, _)| depth > w)
                    && let Some(value) = path.mechanism.value_of(amount)
                {
                    worst = Some((depth, value));
                }
            }
        }
    } else if let Some(travel) = travel {
        for k in 0..=samples {
            let along = render::nm(travel) * f64::from(k) / f64::from(samples);
            let Some(distance) = render::length_from_nm(along.floor()) else {
                continue;
            };
            let value = PoseValue::Travel(distance);
            let found = overlaps_placed(
                a,
                &path.placement(&ma, value),
                b,
                &path.placement(&mb, value),
            );
            for overlap in found {
                let depth = render::nm(overlap.depth);
                if worst.is_none_or(|(w, _)| depth > w) {
                    worst = Some((depth, value));
                }
            }
        }
    }
    let (depth, at) = worst?;
    Some(Depth {
        depth: render::length_from_nm(depth)?,
        at,
    })
}

/// How far `moving` (at rest) can turn about `pivot` before it first hits
/// `fixed` (placed at `fixed_at`); `None` when it turns the whole
/// `opening` clear. The angle is rounded down to the micro-degree.
pub fn turn_contact(
    moving: &WorldSolid,
    fixed: &WorldSolid,
    fixed_at: &Placement,
    pivot: &TurnLine,
    opening: Angle,
) -> Option<Angle> {
    let plane = Plane::of_pivot(pivot);
    let limit = render::radians_f64(opening);
    // A round shape turning about its own axis sweeps only itself, which
    // is where it starts: what it meets there the static checks see.
    let trimmed = moving.without_rods_on(pivot);
    let moving = trimmed.as_ref().unwrap_or(moving);
    let mut best: Option<f64> = None;
    for pair in piece_pairs(
        &plane,
        moving,
        &DMat4::IDENTITY,
        fixed,
        &fixed_at.matrix(),
        Bores::Turning,
    ) {
        if let Some(angle) =
            turn_first_in(&pair.first, &pair.second, pair.room, best.unwrap_or(limit))
            && best.is_none_or(|b| angle < b)
        {
            best = Some(angle);
        }
    }
    best.and_then(render::angle_from_radians_floor)
}

/// The box a turning solid can reach: along the axis its own range, across
/// it the square around the circle its farthest corner turns on.
pub fn turn_reach(moving: &WorldSolid, pivot: &TurnLine) -> Option<WorldBox> {
    let bounds = moving.bounds()?;
    let plane = Plane::of_pivot(pivot);
    let radius = corners_of(&bounds)
        .iter()
        .map(|c| plane.project(*c).length())
        .fold(0.0, f64::max);
    let reach = render::length_from_nm(radius.ceil())?;
    let mut min = pivot.through;
    let mut max = pivot.through;
    for axis in Axis::ALL {
        if axis == pivot.axis {
            min = min.with(axis, bounds.min().get(axis));
            max = max.with(axis, bounds.max().get(axis));
        } else {
            let centre = pivot.through.get(axis).nm();
            min = min.with(axis, Length::from_nm(centre.saturating_sub(reach.nm())));
            max = max.with(axis, Length::from_nm(centre.saturating_add(reach.nm())));
        }
    }
    Some(WorldBox::new(min, max))
}

/// The box a sliding solid covers over its travel.
pub fn slide_reach(moving: &WorldSolid, toward: Direction, travel: Length) -> Option<WorldBox> {
    let bounds = moving.bounds()?;
    let value = if toward.is_positive() {
        travel
    } else {
        Length::from_nm(travel.nm().saturating_neg())
    };
    let moved = translate_box(&bounds, Point::default().with(toward.axis(), value));
    Some(crate::collide::union(&bounds, &moved))
}

#[cfg(test)]
mod tests {
    use super::*;
    use stave_model::solved::Point;

    fn mm(value: i64) -> Length {
        Length::from_mm(value)
    }

    fn point(across: i64, up: i64, deep: i64) -> Point {
        Point {
            across: mm(across),
            up: mm(up),
            deep: mm(deep),
        }
    }

    fn block(a: Point, b: Point) -> WorldSolid {
        WorldSolid::from_boxes(vec![WorldBox::new(a, b)])
    }

    fn square(centre: DVec2, half: f64) -> Vec<DVec2> {
        [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)]
            .iter()
            .map(|&(x, y)| centre + DVec2::new(x, y) * half)
            .collect()
    }

    #[test]
    fn a_full_turn_finds_a_contact_past_half_way_round() {
        // A block 100mm out from the pin, and something in its way 300deg
        // round (counter-clockwise): a frame that turns all the way round
        // must meet it there, past 180deg, and turning the whole 360deg
        // with nothing in the way is clear.
        let moving = square(DVec2::new(100.0, 0.0), 10.0);
        let at = 300f64.to_radians();
        let fixed = square(DVec2::new(100.0 * at.cos(), 100.0 * at.sin()), 10.0);
        let angle = turn_first(&moving, &fixed, TAU).unwrap().to_degrees();
        assert!(
            (270.0..300.0).contains(&angle),
            "met at {angle}deg, expected just before 300deg"
        );
        assert_eq!(turn_first(&moving, &fixed, 250f64.to_radians()), None);
        let far = square(DVec2::new(0.0, 300.0), 10.0);
        assert_eq!(turn_first(&moving, &far, TAU), None);
    }

    #[test]
    fn a_drawer_hits_what_is_in_front_of_it() {
        // A drawer 400mm deep, front at deep 0, pulled toward the front;
        // a rail 100mm in front of it, overlapping it across.
        let drawer = block(point(0, 0, 0), point(400, 150, 400));
        let rail = block(point(300, 50, -120), point(500, 100, -100));
        let hit = slide_contact(&drawer, &rail, &Placement::REST, Direction::Front, mm(400));
        assert_eq!(
            hit,
            Some(SlideHit {
                distance: mm(100),
                shrink: Some((mm(100), Axis::Up)),
            })
        );
        // Not far enough to reach it.
        assert_eq!(
            slide_contact(&drawer, &rail, &Placement::REST, Direction::Front, mm(100)),
            None
        );
        // Beside its path, touching: clear.
        let beside = block(point(400, 0, -200), point(420, 150, 0));
        assert_eq!(
            slide_contact(
                &drawer,
                &beside,
                &Placement::REST,
                Direction::Front,
                mm(400)
            ),
            None
        );
    }

    fn door_pivot() -> TurnLine {
        TurnLine {
            axis: Axis::Up,
            through: point(0, 0, 0),
            free_side: Direction::Right,
            swing_toward: Direction::Front,
        }
    }

    #[test]
    fn a_door_first_hits_a_post_at_the_angle_it_reaches_it() {
        // A door 500 wide, 19 thick behind its front face, hinged on its
        // front left edge; a 50mm post whose nearest corner is 300mm out on
        // the 45 degree line. The door's face first meets the post's
        // corner further out, at atan(c / (c + 50)).
        let door = block(point(0, 0, 0), point(500, 700, 19));
        let c = 300.0 / 2f64.sqrt();
        let near = |v: f64| Length::from_nm((v * 1e6).round() as i64);
        let post = WorldSolid::from_boxes(vec![WorldBox::new(
            Point {
                across: near(c),
                up: mm(0),
                deep: near(-c - 50.0),
            },
            Point {
                across: near(c + 50.0),
                up: mm(700),
                deep: near(-c),
            },
        )]);
        let angle = turn_contact(
            &door,
            &post,
            &Placement::REST,
            &door_pivot(),
            Angle::from_deg(110),
        )
        .unwrap();
        let degrees = angle.microdeg() as f64 / 1e6;
        let expected = c.atan2(c + 50.0).to_degrees();
        // Touching within the tolerance is not hitting: about 0.01deg later.
        assert!((degrees - expected).abs() < 0.02, "{degrees} vs {expected}");
        // A door that opens only to 30 degrees never reaches it.
        assert_eq!(
            turn_contact(
                &door,
                &post,
                &Placement::REST,
                &door_pivot(),
                Angle::from_deg(30)
            ),
            None
        );
        // Below the door's height range: never in the way.
        let low = WorldSolid::from_boxes(vec![WorldBox::new(
            point(100, -100, -300),
            point(200, -1, -200),
        )]);
        assert_eq!(
            turn_contact(
                &door,
                &low,
                &Placement::REST,
                &door_pivot(),
                Angle::from_deg(110)
            ),
            None
        );
    }

    #[test]
    fn a_round_shape_turning_about_its_own_axis_sweeps_only_itself() {
        // A pin 20mm across standing on the door's pivot line, and a block
        // 1mm clear of it: the box around the pin would swing its corners
        // 4mm into the block, but the pin turning about its own axis
        // sweeps only itself (grammar, "Sweeps see cuts").
        let rod = |across: i64| WorldRod {
            axis: Axis::Up,
            centre: [mm(across), mm(0)],
            diameter: mm(20),
            start: mm(0),
            end: mm(100),
        };
        let pin = WorldSolid::default().with_rod(rod(0));
        let beside = block(point(11, 0, -5), point(30, 100, 5));
        assert_eq!(
            turn_contact(
                &pin,
                &beside,
                &Placement::REST,
                &door_pivot(),
                Angle::from_deg(90)
            ),
            None
        );
        // Off the line by a millimetre, it is swept as the box around it.
        let off = WorldSolid::default().with_rod(rod(-1));
        assert!(
            turn_contact(
                &off,
                &beside,
                &Placement::REST,
                &door_pivot(),
                Angle::from_deg(90)
            )
            .is_some()
        );
    }

    #[test]
    fn a_door_against_its_side_turns_away_from_it() {
        // The side the door closes against, behind it: touching, and the
        // door swings away from it.
        let door = block(point(0, 0, 0), point(500, 700, 19));
        let side = block(point(0, 0, 19), point(19, 700, 500));
        assert_eq!(
            turn_contact(
                &door,
                &side,
                &Placement::REST,
                &door_pivot(),
                Angle::from_deg(110)
            ),
            None
        );
        // Turned the wrong way it goes straight in.
        let backward = TurnLine {
            swing_toward: Direction::Back,
            ..door_pivot()
        };
        let angle = turn_contact(
            &door,
            &side,
            &Placement::REST,
            &backward,
            Angle::from_deg(110),
        )
        .unwrap();
        assert!(angle < Angle::from_deg(1), "{angle}");
    }

    /// An upright 45mm thick (across), with a row of four 9mm holes
    /// through it, 100mm apart from 850mm up down, and an 8mm bolt through
    /// the top one.
    fn holed_upright() -> (WorldSolid, WorldSolid) {
        let hole = |up: i64| WorldRod {
            axis: Axis::Across,
            // `Axis::Across`'s others are up, then deep.
            centre: [mm(up), mm(0)],
            diameter: mm(9),
            start: mm(0),
            end: mm(45),
        };
        let upright = [850, 750, 650, 550]
            .into_iter()
            .fold(block(point(0, 0, -35), point(45, 1500, 35)), |solid, up| {
                solid.with_hole(hole(up))
            });
        let bolt = WorldSolid::default().with_rod(WorldRod {
            diameter: mm(8),
            start: mm(-10),
            end: mm(60),
            ..hole(850)
        });
        (upright, bolt)
    }

    #[test]
    fn a_bolt_sliding_out_of_its_hole_into_wood_hits() {
        // Grammar, "Sweeps see cuts": the bolt overlaps the upright's
        // block where it starts, but only inside the hole cut for it, so
        // it is clear only while it stays in the hole. Slid down, it meets
        // the wood between the holes half a millimetre (and the contact
        // tolerance) on.
        let (upright, bolt) = holed_upright();
        let hit = slide_contact(&bolt, &upright, &Placement::REST, Direction::Down, mm(300))
            .expect("the bolt leaves its hole");
        assert_eq!(hit.distance, Length::from_nm(550_000));
        // The upright sliding up past the bolt is the same motion.
        let back = slide_contact(&upright, &bolt, &Placement::REST, Direction::Up, mm(300))
            .expect("the hole leaves the bolt");
        assert_eq!(back.distance, Length::from_nm(550_000));
        // Slid along its own axis, out of the through hole: no wood beyond.
        assert_eq!(
            slide_contact(&bolt, &upright, &Placement::REST, Direction::Right, mm(300)),
            None
        );
        // Less than half a millimetre: still in its hole.
        assert_eq!(
            slide_contact(
                &bolt,
                &upright,
                &Placement::REST,
                Direction::Down,
                Length::from_nm(400_000)
            ),
            None
        );
    }

    #[test]
    fn a_bolt_in_a_hole_that_holds_it_all_the_way_is_clear() {
        // A slot made of holes that overlap enough to hold the bolt the
        // whole way: each carries it on to the next.
        let hole = |up: i64| WorldRod {
            axis: Axis::Across,
            centre: [mm(up), mm(0)],
            diameter: mm(12),
            start: mm(0),
            end: mm(45),
        };
        let slotted = (0..=10).fold(block(point(0, 0, -35), point(45, 1500, 35)), |solid, k| {
            solid.with_hole(hole(850 - k))
        });
        let (_, bolt) = holed_upright();
        let hit = slide_contact(&bolt, &slotted, &Placement::REST, Direction::Down, mm(300))
            .expect("past the last hole it meets wood");
        assert!(
            hit.distance > mm(10) && hit.distance < mm(13),
            "{:?}",
            hit.distance
        );
    }

    #[test]
    fn a_piece_turning_out_of_its_hole_into_wood_hits() {
        // A pin 8mm across, 100mm from the line it turns about, in a 9mm
        // hole: it leaves the hole once its centre has moved 0.55mm, about
        // 0.315deg round, and is then in the wood.
        let pin = Piece {
            corners: vec![
                DVec2::new(96.0, -4.0),
                DVec2::new(104.0, -4.0),
                DVec2::new(104.0, 4.0),
                DVec2::new(96.0, 4.0),
            ],
            low: 0.0,
            high: 10.0,
            round: Some((DVec2::new(100.0, 0.0), 4.0)),
        };
        let wood = Piece {
            corners: vec![
                DVec2::new(50.0, -50.0),
                DVec2::new(150.0, -50.0),
                DVec2::new(150.0, 50.0),
                DVec2::new(50.0, 50.0),
            ],
            low: 0.0,
            high: 10.0,
            round: None,
        };
        let room = Room {
            centre: DVec2::new(100.0, 0.0),
            radius: 4.55,
            in_second: true,
        };
        let angle = turn_first_in(&pin, &wood, Some(room), TAU).expect("it leaves the hole");
        let expected = 2.0 * (0.55f64 / 200.0).asin();
        assert!((angle - expected).abs() < 1e-9, "{angle} vs {expected}");
        // Without its room it overlaps the wood from the start: left to
        // the static checks.
        assert_eq!(turn_first_in(&pin, &wood, None, TAU), None);
    }
}

#[cfg(test)]
mod linkage_tests {
    use super::*;
    use stave_lang::placement::{Axis, Direction};
    use stave_model::mechanism::{
        FramePoint, Holding, MechanismDrive, MechanismEntry, MechanismFrame, MechanismMember,
        MemberBody,
    };
    use stave_model::solved::{BodyId, HardwareId};

    fn mm(value: i64) -> Length {
        Length::from_mm(value)
    }

    /// The pins of `euro_hinge` (`std/hardware/hinges.stave`), in
    /// nanometres across from the cup's centre and in from the door's
    /// back: the arm's pin, then the cup's, for the inner link and then the
    /// outer link.
    const PINS: [[(i64, i64); 2]; 2] = [
        [(13_500_000, 10_900_000), (-8_800_000, 7_330_000)],
        [(13_280_000, 3_560_000), (-9_540_000, 1_390_000)],
    ];

    fn index(i: usize) -> MemberIndex {
        MemberIndex::new(i).unwrap()
    }

    /// The cup hinge on a 19mm door whose front hinge corner is the world
    /// origin, as its kind declares it: the frame's origin at the cup's
    /// centre on the door's back face (deep 19mm), `across` toward the
    /// middle of the door, `depth` into it; the arm (held), the cup
    /// (driven), the inner and the outer link.
    fn cup_hinge() -> Mechanism {
        let frame = MechanismFrame::new(
            Axis::Up,
            Point {
                across: Length::from_nm(22_500_000),
                up: mm(0),
                deep: mm(19),
            },
            Direction::Right,
            Direction::Front,
        )
        .unwrap();
        let member = |name: &str, holding| MechanismMember {
            hardware: HardwareId::from_usize(0),
            body: MemberBody::Body(BodyId::new(0).unwrap()),
            name: name.to_owned(),
            holding,
        };
        let members = vec![
            member("arm", Holding::Held),
            member("cup", Holding::Free),
            member("inner_link", Holding::Free),
            member("outer_link", Holding::Free),
        ];
        let at = |(across, depth): (i64, i64)| {
            FramePoint::new(Length::from_nm(across), Length::from_nm(depth))
        };
        let mut entries = Vec::new();
        for (link, [arm, cup]) in PINS.iter().enumerate() {
            entries.push(MechanismEntry::Pivot {
                first: index(0),
                second: index(2 + link),
                at: at(*arm),
            });
            entries.push(MechanismEntry::Pivot {
                first: index(1),
                second: index(2 + link),
                at: at(*cup),
            });
        }
        Mechanism::new(
            frame,
            members,
            entries,
            MechanismDrive::Turn {
                member: index(1),
                from: Angle::ZERO,
                to: Angle::from_deg(110),
            },
            None,
            vec![HardwareId::from_usize(0)],
        )
        .unwrap()
    }

    fn path() -> Path {
        Path::new(&cup_hinge()).unwrap()
    }

    fn block(a: [i64; 3], b: [i64; 3]) -> WorldSolid {
        let point = |[across, up, deep]: [i64; 3]| Point {
            across: mm(across),
            up: mm(up),
            deep: mm(deep),
        };
        WorldSolid::from_boxes(vec![WorldBox::new(point(a), point(b))])
    }

    /// The linkage sweep finds the same first contact as stepping the door
    /// through every hundredth of a degree, to within that step, and
    /// never later.
    #[test]
    fn a_door_on_a_linkage_first_hits_where_dense_steps_find_it() {
        let path = path();
        assert_eq!(path.mechanism().shape(), MotionShape::General);
        // A 450mm door, 19mm thick, in front of the side's edge.
        let door = block([0, 0, 0], [450, 700, 19]);
        // A post in front of the door's free end, and one beside its hinge
        // edge (a wall 0.5mm off it, as deep as the door).
        let post = block([300, 0, -200], [340, 700, -150]);
        let wall_point = |across: f64, deep: i64| Point {
            across: Length::from_nm((across * 1e6).round() as i64),
            up: mm(0),
            deep: mm(deep),
        };
        let wall = WorldSolid::from_boxes(vec![WorldBox::new(
            wall_point(-20.0, 0),
            Point {
                up: mm(700),
                ..wall_point(-0.5, 400)
            },
        )]);
        let rest = Mover::Fixed(Placement::REST);
        for fixed in [&post, &wall] {
            let Some(Reach::Angle(found)) = first_contact(&door, Mover::Rigid(&path), fixed, rest)
            else {
                panic!("a hit");
            };
            let plane = path.plane();
            let pairs = piece_pairs(
                &plane,
                &door,
                &DMat4::IDENTITY,
                fixed,
                &DMat4::IDENTITY,
                Bores::Wood,
            );
            let (m, f) = (&pairs[0].first, &pairs[0].second.corners);
            let cup = path.mechanism().moving();
            let dense = (1..=11_000)
                .map(|k| (f64::from(k) / 100.0).to_radians())
                .find(|theta| {
                    let (rotation, offset) = path.mechanism().motion(cup, *theta).unwrap();
                    intersect(&m.moved(Motion2 { rotation, offset }).corners, f)
                })
                .unwrap();
            let found_radians = render::radians_f64(found);
            assert!(found_radians <= dense, "{found} after {dense}");
            assert!(dense - found_radians < 0.0002, "{found} vs {dense}");
            // The same either way round.
            assert_eq!(
                first_contact(fixed, rest, &door, Mover::Rigid(&path)),
                Some(Reach::Angle(found))
            );
        }
        // Nothing in the way: clear.
        let far = block([600, 0, -200], [640, 700, -150]);
        assert_eq!(first_contact(&door, Mover::Rigid(&path), &far, rest), None);
        // The side the door rests on: the linkage lifts it off.
        let side = block([-19, 0, 19], [0, 700, 400]);
        assert_eq!(first_contact(&door, Mover::Rigid(&path), &side, rest), None);
    }

    /// The inner link's bar, 9mm long along the pins, at rest.
    fn inner_link() -> WorldSolid {
        let [(ax, ad), (bx, bd)] = PINS[0];
        let world = |across: i64, depth: i64| {
            [
                Length::from_nm(22_500_000 + across),
                Length::from_nm(19_000_000 - depth),
            ]
        };
        // `Axis::Up`'s others are across, then deep.
        WorldSolid::with_prism(WorldPrism::bar(
            Axis::Up,
            world(ax, ad),
            world(bx, bd),
            Length::from_nm(1_500_000),
            Length::from_nm(345_500_000),
            Length::from_nm(354_500_000),
        ))
    }

    /// The door, 19mm thick, and with the 35mm cup bored 12.8mm into its
    /// back 5mm from its hinge edge.
    fn door(bored: bool) -> WorldSolid {
        let door = block([0, 0, 0], [450, 700, 19]);
        if !bored {
            return door;
        }
        door.with_hole(WorldRod {
            axis: Axis::Deep,
            // `Axis::Deep`'s others are up, then across.
            centre: [mm(350), Length::from_nm(22_500_000)],
            diameter: mm(35),
            start: Length::from_nm(6_200_000),
            end: mm(19),
        })
    }

    /// Swept wood has its bores: the link that turns in the cup's bore
    /// stays out of the door's wood all the way open, measured on the
    /// same pieces the sweep uses; without the bore it is deep in the
    /// door from the start.
    #[test]
    fn swept_wood_has_its_bores() {
        let path = path();
        let link = Mover::Member(&path, index(2));
        let door_moves = Mover::Rigid(&path);
        assert_eq!(
            deepest_overlap(&inner_link(), link, &door(true), door_moves, 2000),
            None
        );
        assert_eq!(
            first_contact(&inner_link(), link, &door(true), door_moves),
            None
        );
        let unbored = deepest_overlap(&inner_link(), link, &door(false), door_moves, 2000).unwrap();
        assert!(unbored.depth > mm(1), "{:?}", unbored);
    }

    /// A runner on a rail along a held guide, its pins along `across`,
    /// running down 300mm.
    fn runner() -> Path {
        let frame = MechanismFrame::new(
            Axis::Across,
            Point::default(),
            Direction::Up,
            Direction::Front,
        )
        .unwrap();
        let member = |name: &str, holding| MechanismMember {
            hardware: HardwareId::from_usize(0),
            body: MemberBody::Body(BodyId::new(0).unwrap()),
            name: name.to_owned(),
            holding,
        };
        let mechanism = Mechanism::new(
            frame,
            vec![
                member("runner", Holding::Free),
                member("guide", Holding::Held),
            ],
            vec![MechanismEntry::Rail {
                runner: index(0),
                guide: index(1),
                direction: stave_model::mechanism::FrameDirection::new(mm(1), Length::ZERO)
                    .unwrap(),
            }],
            MechanismDrive::Travel {
                member: index(0),
                from: Length::ZERO,
                to: mm(-300),
            },
            None,
            vec![HardwareId::from_usize(0)],
        )
        .unwrap();
        Path::new(&mechanism).unwrap()
    }

    /// Swept by conservative advancement (as a member its mechanism
    /// places), a bolt that starts in its hole is clear only while it
    /// stays in it, and hits the wood between the holes once out: its
    /// room, as in the exact sweeps.
    #[test]
    fn a_member_leaving_its_hole_into_wood_hits_when_advanced() {
        let path = runner();
        let hole = |up: i64| WorldRod {
            axis: Axis::Across,
            centre: [mm(up), mm(0)],
            diameter: mm(9),
            start: mm(0),
            end: mm(45),
        };
        let upright = [850, 750]
            .into_iter()
            .fold(block([0, 0, -35], [45, 1500, 35]), |solid, up| {
                solid.with_hole(hole(up))
            });
        let bolt = WorldSolid::default().with_rod(WorldRod {
            diameter: mm(8),
            start: mm(-10),
            end: mm(60),
            ..hole(850)
        });
        let Some(Reach::Distance(hit)) = first_contact(
            &bolt,
            Mover::Member(&path, index(0)),
            &upright,
            Mover::Fixed(Placement::REST),
        ) else {
            panic!("the bolt leaves its hole into wood");
        };
        assert!(
            (Length::from_nm(540_000)..=Length::from_nm(560_000)).contains(&hit.distance),
            "{:?}",
            hit.distance
        );
    }
}
