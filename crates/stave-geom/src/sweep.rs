//! Motion sweeps: how far a drawer slides, or a door turns, before one
//! body first runs into another (grammar, "Scenes": every motion is
//! checked through its whole travel, not only at its ends; "One collision
//! model": every body, part or hardware member, is swept the same way).
//!
//! A [`Path`] is a motion ready to sweep, and a [`Mover`] says how it moves
//! one body: not at all (where a scene places it), with the assembly as
//! one piece, or as a link the hardware's mechanism turns. [`first_contact`]
//! picks the method from the motion's shape, never from a kind:
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
//! - **A linkage** (a cup hinge) moves the door and each link differently
//!   ([`crate::linkage`]), so there is no closed form. The same pieces are
//!   swept by **conservative advancement**: at each angle the distance
//!   between two pieces is measured, and the motion goes on by that
//!   distance over the fastest their corners can close it (a door's
//!   corner turns about the frame's origin and moves with the offset; a
//!   link's corner turns about its arm pin as fast as its boss pin moves
//!   over its length), so no contact can be stepped over; near contact the
//!   step never falls below a quarter of the contact tolerance's worth of
//!   movement. The first overlap is then bisected to far below a
//!   micro-degree.
//!
//! **Sweeps see cuts**: a part's boxes are taken less the round holes
//! whose axis lies in the plane, as their narrowest chord over the width
//! the other piece shares with them ([`crate::plane`]), so a cup's arm and
//! links can move in its bore. Other holes and angled cuts are left as
//! wood, a rod is the box around it, and a fixed solid turned about
//! another axis is the outline of its corners: each finds more wood than
//! there is, never less. Angles are floats here (`f64` nanometres and
//! radians, converted in [`render`](crate::render)); results are rounded
//! down to the micro-degree.

use std::f64::consts::TAU;

use glam::{DMat4, DVec2, DVec3};
use stave_lang::placement::{Axis, Direction};
use stave_lang::units::{Angle, Length};
use stave_model::solved::{LinkIndex, Linkage, Motion, Pivot, Point, PoseValue, WorldBox};

use crate::collide::{
    CONTACT_TOLERANCE, Placement, WorldPrism, WorldRod, WorldSolid, overlaps_placed, placed_bounds,
    translate_box,
};
use crate::linkage::LinkagePath;
use crate::plane::{
    Motion2, Piece, Plane, corners_of, intersect, piece_depth, piece_pairs, polygon_distance,
};
use crate::render;

/// The boxes of a solid's wood: its blocks and the boxes around its rods
/// and prisms.
fn wood_boxes(solid: &WorldSolid) -> impl Iterator<Item = WorldBox> + '_ {
    solid
        .boxes()
        .iter()
        .copied()
        .chain(solid.rods().iter().map(WorldRod::bounds))
        .chain(solid.prisms().iter().filter_map(WorldPrism::bounds))
}

/// A fixed solid's boxes at its placement: exact when it only moved, the
/// box around each turned box otherwise.
fn placed_boxes(solid: &WorldSolid, placement: &Placement) -> Vec<WorldBox> {
    wood_boxes(solid)
        .map(|b| match placement {
            Placement::Exact(offset) => translate_box(&b, *offset),
            Placement::Turned(_) => placed_bounds(&b, placement),
        })
        .collect()
}

/// Overlap of two ranges, in nanometres (negative when apart).
fn shared(a: &WorldBox, b: &WorldBox, axis: Axis) -> i64 {
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
    best
}

fn rotate(p: DVec2, angle: f64) -> DVec2 {
    DVec2::from_angle(angle).rotate(p)
}

/// The angles in `(0, limit]` at which the circle of radius `|p|` about the
/// origin meets the segment `a..b`, measured from `p` counter-clockwise
/// when `forward`, else from the meeting point to `p`.
fn crossings(p: DVec2, a: DVec2, b: DVec2, forward: bool, limit: f64, out: &mut Vec<f64>) {
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
fn turn_first(moving: &[DVec2], fixed: &[DVec2], limit: f64) -> Option<f64> {
    if intersect(moving, fixed) {
        return None;
    }
    let mut events = vec![0.0, limit];
    for (polygon, other, forward) in [(moving, fixed, true), (fixed, moving, false)] {
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
        let middle = (from + to) / 2.0;
        let turned: Vec<DVec2> = moving.iter().map(|p| rotate(*p, middle)).collect();
        if intersect(&turned, fixed) {
            return Some(from);
        }
    }
    None
}

/// A motion ready to sweep (grammar, "Scenes"): a turn about one line, a
/// linkage solved once in floats with the fastest its offset changes, or a
/// slide's travel.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Path {
    motion: PathMotion,
}

// One per motion swept, and never in a collection: boxing the linkage
// would buy nothing.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, Copy, PartialEq)]
enum PathMotion {
    Turn {
        pivot: Pivot,
        opening: Angle,
    },
    Linkage {
        linkage: Linkage,
        path: LinkagePath,
        offset_speed: f64,
    },
    Travel {
        toward: Direction,
        travel: Length,
    },
}

/// How a body moves in a sweep.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Mover<'a> {
    /// It stays where it is placed.
    Fixed(Placement),
    /// It moves with the path from rest, as one piece with the assembly.
    Rigid(&'a Path),
    /// It is that link of the path's linkage, from rest: it turns about its
    /// arm pin as the linkage opens.
    Member(&'a Path, LinkIndex),
}

impl Mover<'_> {
    fn path(&self) -> Option<&Path> {
        match self {
            Mover::Fixed(_) => None,
            Mover::Rigid(path) | Mover::Member(path, _) => Some(path),
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
    /// A motion ready to sweep.
    pub fn new(motion: &Motion) -> Path {
        let motion = match *motion {
            Motion::Turn {
                pivot,
                opening_angle,
            } => PathMotion::Turn {
                pivot,
                opening: opening_angle,
            },
            Motion::Linkage { linkage } => {
                let path = LinkagePath::new(&linkage);
                PathMotion::Linkage {
                    linkage,
                    path,
                    offset_speed: path.offset_speed(),
                }
            }
            Motion::Travel { toward, travel } => PathMotion::Travel { toward, travel },
        };
        Path { motion }
    }

    /// The plane a turning motion moves in.
    fn plane(&self) -> Option<Plane> {
        match &self.motion {
            PathMotion::Turn { pivot, .. } => Some(Plane::of_pivot(pivot)),
            PathMotion::Linkage { linkage, .. } => Some(Plane::of_linkage(linkage)),
            PathMotion::Travel { .. } => None,
        }
    }

    /// How far a turning motion opens, in radians.
    fn limit(&self) -> f64 {
        match &self.motion {
            PathMotion::Turn { opening, .. } => render::radians_f64(*opening),
            PathMotion::Linkage { path, .. } => path.opening(),
            PathMotion::Travel { .. } => 0.0,
        }
    }

    /// Where a mover has moved its pieces in the plane at `theta`.
    fn motion2(&self, mover: &Mover<'_>, theta: f64) -> Option<Motion2> {
        let turn = |angle: f64| DVec2::from_angle(angle);
        match (&self.motion, mover) {
            (_, Mover::Fixed(_)) => Some(Motion2::STILL),
            (PathMotion::Turn { .. }, Mover::Rigid(_)) => Some(Motion2 {
                rotation: turn(theta),
                offset: DVec2::ZERO,
            }),
            (PathMotion::Linkage { path, .. }, Mover::Rigid(_)) => {
                let (rotation, offset) = path.plane_pose(theta)?;
                Some(Motion2 { rotation, offset })
            }
            (PathMotion::Linkage { path, .. }, Mover::Member(_, link)) => {
                let (rotation, offset) = path.link_motion(link.index(), theta)?;
                Some(Motion2 { rotation, offset })
            }
            (PathMotion::Turn { .. } | PathMotion::Travel { .. }, Mover::Member(..))
            | (PathMotion::Travel { .. }, Mover::Rigid(_)) => None,
        }
    }

    /// The fastest any corner of a piece can move per radian.
    fn speed(&self, mover: &Mover<'_>, piece: &Piece) -> f64 {
        match (&self.motion, mover) {
            (_, Mover::Fixed(_)) | (PathMotion::Travel { .. }, _) => 0.0,
            (PathMotion::Turn { .. }, _) => piece.radius_about(DVec2::ZERO),
            (PathMotion::Linkage { offset_speed, .. }, Mover::Rigid(_)) => {
                LinkagePath::speed_bound(piece.radius_about(DVec2::ZERO), *offset_speed)
            }
            (
                PathMotion::Linkage {
                    path, offset_speed, ..
                },
                Mover::Member(_, link),
            ) => {
                let ground = path.ground_pin_of(link.index()).unwrap_or(DVec2::ZERO);
                path.link_speed(link.index(), piece.radius_about(ground), *offset_speed)
                    .unwrap_or(f64::INFINITY)
            }
        }
    }

    /// The box a body can reach as the path moves it.
    pub fn reach(&self, solid: &WorldSolid, mover: &Mover<'_>) -> Option<WorldBox> {
        match (&self.motion, mover) {
            (_, Mover::Fixed(placement)) => Some(placed_bounds(&solid.bounds()?, placement)),
            (PathMotion::Turn { pivot, .. }, Mover::Rigid(_)) => turn_reach(solid, pivot),
            (PathMotion::Travel { toward, travel }, Mover::Rigid(_)) => {
                slide_reach(solid, *toward, *travel)
            }
            (PathMotion::Linkage { path, .. }, Mover::Rigid(_)) => {
                sampled_reach(solid, self, mover, |theta| path.coupler(theta))
            }
            (PathMotion::Linkage { path, .. }, Mover::Member(_, link)) => {
                sampled_reach(solid, self, mover, |theta| {
                    path.pose(theta)
                        .and_then(|pose| pose.links.get(link.index()).copied())
                })
            }
            (PathMotion::Turn { .. } | PathMotion::Travel { .. }, Mover::Member(..)) => None,
        }
    }

    /// The placement a mover gives its body at a pose value of the path.
    fn placement(&self, mover: &Mover<'_>, value: PoseValue) -> Placement {
        let motion = match self.motion {
            PathMotion::Turn { pivot, opening } => Motion::Turn {
                pivot,
                opening_angle: opening,
            },
            PathMotion::Linkage { linkage, .. } => Motion::Linkage { linkage },
            PathMotion::Travel { toward, travel } => Motion::Travel { toward, travel },
        };
        match mover {
            Mover::Fixed(placement) => *placement,
            Mover::Rigid(_) => Placement::of_motions(&[(motion, value)]),
            Mover::Member(_, link) => Placement::of_member(&[(motion, value)], Some(*link)),
        }
    }
}

/// The box a body reaches over a turning path, from its corners at 180
/// steps of the opening (a degree or less each), grown by the most it can
/// move within half a step.
fn sampled_reach(
    solid: &WorldSolid,
    path: &Path,
    mover: &Mover<'_>,
    matrix: impl Fn(f64) -> Option<DMat4>,
) -> Option<WorldBox> {
    let bounds = solid.bounds()?;
    let corners = corners_of(&bounds);
    let plane = path.plane()?;
    let flat = Piece {
        corners: corners.iter().map(|c| plane.project(*c)).collect(),
        low: 0.0,
        high: 0.0,
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
) -> Vec<(Piece, Piece)> {
    let matrix = |mover: &Mover<'_>| match mover {
        Mover::Fixed(placement) => placement.matrix(),
        Mover::Rigid(_) | Mover::Member(..) => DMat4::IDENTITY,
    };
    piece_pairs(plane, a, &matrix(ma), b, &matrix(mb))
}

/// The first angle in `(0, limit]` at which two pieces, each moved by its
/// mover, overlap; `None` when they stay clear (or overlap at the start,
/// which the static checks report). Swept by conservative advancement:
/// each step is no more than the distance between them over the fastest
/// their corners can close it, never less than a quarter of the contact
/// tolerance's worth; the first overlap is then bisected.
fn advance(
    path: &Path,
    a: &Piece,
    ma: &Mover<'_>,
    b: &Piece,
    mb: &Mover<'_>,
    limit: f64,
) -> Option<f64> {
    if intersect(&a.corners, &b.corners) {
        return None;
    }
    let speed = path.speed(ma, a) + path.speed(mb, b);
    if speed <= 0.0 {
        return None;
    }
    let at = |theta: f64| -> Option<(Vec<DVec2>, Vec<DVec2>)> {
        Some((
            a.moved(path.motion2(ma, theta)?).corners,
            b.moved(path.motion2(mb, theta)?).corners,
        ))
    };
    let smallest = render::nm(CONTACT_TOLERANCE) / 4.0 / speed;
    let mut theta = 0.0;
    let (mut here_a, mut here_b) = (a.corners.clone(), b.corners.clone());
    loop {
        let step = (polygon_distance(&here_a, &here_b) / speed).max(smallest);
        let next = (theta + step).min(limit);
        let (there_a, there_b) = at(next)?;
        if intersect(&there_a, &there_b) {
            let (mut clear, mut hit) = (theta, next);
            for _ in 0..60 {
                let middle = (clear + hit) / 2.0;
                let (x, y) = at(middle)?;
                if intersect(&x, &y) {
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
    if matches!((&ma, &mb), (Mover::Rigid(_), Mover::Rigid(_))) {
        return None;
    }
    match (&path.motion, &ma, &mb) {
        (PathMotion::Travel { toward, travel }, Mover::Rigid(_), Mover::Fixed(placement)) => {
            slide_contact(a, b, placement, *toward, *travel).map(Reach::Distance)
        }
        (PathMotion::Travel { toward, travel }, Mover::Fixed(placement), Mover::Rigid(_)) => {
            slide_contact(b, a, placement, *toward, *travel).map(Reach::Distance)
        }
        (PathMotion::Travel { .. }, _, _) => None,
        (PathMotion::Turn { pivot, opening }, Mover::Rigid(_), Mover::Fixed(placement)) => {
            turn_contact(a, b, placement, pivot, *opening).map(Reach::Angle)
        }
        (PathMotion::Turn { pivot, opening }, Mover::Fixed(placement), Mover::Rigid(_)) => {
            turn_contact(b, a, placement, pivot, *opening).map(Reach::Angle)
        }
        (PathMotion::Turn { .. }, _, _) => None,
        (PathMotion::Linkage { .. }, _, _) => {
            if ma == mb {
                return None;
            }
            let plane = path.plane()?;
            let limit = path.limit();
            let mut best: Option<f64> = None;
            for (p, q) in start_pairs(&plane, a, &ma, b, &mb) {
                if let Some(angle) = advance(path, &p, &ma, &q, &mb, best.unwrap_or(limit))
                    && best.is_none_or(|b| angle < b)
                {
                    best = Some(angle);
                }
            }
            best.and_then(render::angle_from_radians_floor)
                .map(Reach::Angle)
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
    if matches!((&ma, &mb), (Mover::Rigid(_), Mover::Rigid(_))) {
        return None;
    }
    let samples = samples.max(1);
    let mut worst: Option<(f64, PoseValue)> = None;
    if let Some(plane) = path.plane() {
        let limit = path.limit();
        let pairs = start_pairs(&plane, a, &ma, b, &mb);
        for k in 0..=samples {
            let theta = limit * f64::from(k) / f64::from(samples);
            let (Some(moved_a), Some(moved_b)) =
                (path.motion2(&ma, theta), path.motion2(&mb, theta))
            else {
                continue;
            };
            for (p, q) in &pairs {
                let Some((depth, _)) = piece_depth(&p.moved(moved_a), &q.moved(moved_b)) else {
                    continue;
                };
                if worst.is_none_or(|(w, _)| depth > w)
                    && let Some(angle) = render::angle_from_radians_floor(theta)
                {
                    worst = Some((depth, PoseValue::Angle(angle)));
                }
            }
        }
    } else if let PathMotion::Travel { travel, .. } = path.motion {
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
    pivot: &Pivot,
    opening: Angle,
) -> Option<Angle> {
    let plane = Plane::of_pivot(pivot);
    let limit = render::radians_f64(opening);
    let mut best: Option<f64> = None;
    for (m, f) in piece_pairs(&plane, moving, &DMat4::IDENTITY, fixed, &fixed_at.matrix()) {
        if let Some(angle) = turn_first(&m.corners, &f.corners, best.unwrap_or(limit))
            && best.is_none_or(|b| angle < b)
        {
            best = Some(angle);
        }
    }
    best.and_then(render::angle_from_radians_floor)
}

/// The box a turning solid can reach: along the axis its own range, across
/// it the square around the circle its farthest corner turns on.
pub fn turn_reach(moving: &WorldSolid, pivot: &Pivot) -> Option<WorldBox> {
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

    fn door_pivot() -> Pivot {
        Pivot {
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
        let backward = Pivot {
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
}

#[cfg(test)]
mod linkage_tests {
    use super::*;
    use stave_lang::placement::{Axis, Direction};
    use stave_model::solved::{Link, LinkageFrame, PlanePoint};

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

    /// The cup hinge on a 19mm door in a frame at the door's front hinge
    /// corner (the world origin): the door's back face is at deep 19mm,
    /// the side's front edge behind it.
    fn cup_hinge() -> Linkage {
        let place = |(across, depth): (i64, i64)| {
            PlanePoint::new(
                Length::from_nm(22_500_000 + across),
                Length::from_nm(depth - 19_000_000),
            )
        };
        let links = PINS.map(|[arm, cup]| Link {
            ground: place(arm),
            coupler: place(cup),
        });
        let frame = LinkageFrame::new(
            Axis::Up,
            Point::default(),
            Direction::Right,
            Direction::Front,
        )
        .unwrap();
        Linkage::new(frame, links, Angle::from_deg(110)).unwrap()
    }

    fn path() -> Path {
        Path::new(&Motion::Linkage {
            linkage: cup_hinge(),
        })
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
        let PathMotion::Linkage {
            linkage,
            path: solved,
            ..
        } = path.motion
        else {
            panic!("a linkage");
        };
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
            let plane = Plane::of_linkage(&linkage);
            let pairs = piece_pairs(&plane, &door, &DMat4::IDENTITY, fixed, &DMat4::IDENTITY);
            let (m, f) = (&pairs[0].0, &pairs[0].1.corners);
            let dense = (1..=11_000)
                .map(|k| (f64::from(k) / 100.0).to_radians())
                .find(|theta| {
                    let (rotation, offset) = solved.plane_pose(*theta).unwrap();
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
        let link = Mover::Member(&path, LinkIndex::First);
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
}
