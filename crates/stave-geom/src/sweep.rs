//! Motion sweeps: how far a drawer slides, or a door turns, before it first
//! hits something (grammar, "Scenes": every motion is checked through its
//! whole travel, not only at its ends).
//!
//! **Slides** are one-dimensional and exact. The moving solid travels
//! along a world direction; a fixed box is in its way when the two overlap
//! by more than the contact tolerance across that direction. The first
//! contact is then a subtraction in whole nanometres: the gap ahead.
//!
//! **Hinges** turn about a line along a world axis, so the sweep reduces to
//! two dimensions. Fixed solids that overlap the moving one along the axis
//! are projected onto the plane across it; both sides become convex
//! polygons, shrunk by half the contact tolerance so that touching is not
//! hitting. Contact can only begin where a corner of one polygon crosses an
//! edge of the other: each moving corner turns on a circle about the pivot,
//! met by each fixed edge, and each fixed corner meets each moving edge on
//! its own circle. Those angles are found in closed form; between two of
//! them the polygons either overlap all the way or not at all, so testing
//! one angle in each span finds the first contact. This is the only place
//! the angles are floats (`f64` nanometres and radians, converted in
//! [`render`](crate::render)); the result is rounded down to the
//! micro-degree.
//!
//! **Cup hinges** move on a four-bar linkage: the door turns and moves at
//! once ([`crate::linkage`]), so a fixed corner meets a moving edge on no
//! circle and there is no closed form. The same projected polygons are
//! swept by **conservative advancement**: at each angle the distance
//! between the two polygons is measured, and the door turns on by that
//! distance divided by the fastest any of its corners can move per radian
//! (its distance from the frame's origin plus the fastest the linkage's
//! offset changes), so no contact can be stepped over; near contact the
//! step never falls below a quarter of the contact tolerance's worth of
//! movement. The first angle at which they overlap is then found by
//! bisection, to far below a micro-degree. Butt hinges keep the exact
//! closed form.
//!
//! Approximations (each finds more wood than there is, never less): a rod
//! is projected as the box around it (exact for a knob, whose axis lies in
//! the plane; a rod along the hinge axis becomes its square), holes and
//! angled cuts are left out, and a fixed solid turned about another axis is
//! projected as the outline of its corners.

use std::f64::consts::TAU;

use glam::{DVec2, DVec3};
use stave_lang::placement::{Axis, Direction};
use stave_lang::units::{Angle, Length};
use stave_model::solved::{Linkage, Pivot, Point, WorldBox};

use crate::collide::{
    CONTACT_TOLERANCE, Placement, WorldRod, WorldSolid, direction_vector, placed_bounds,
    point_vector, translate_box,
};
use crate::linkage::LinkagePath;
use crate::render;

/// The boxes of a solid's wood: its blocks and the boxes around its rods.
fn wood_boxes(solid: &WorldSolid) -> impl Iterator<Item = WorldBox> + '_ {
    solid
        .boxes()
        .iter()
        .copied()
        .chain(solid.rods().iter().map(WorldRod::bounds))
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

/// The plane across a hinge axis: `x` toward the free side, `y` toward the
/// side it swings to, origin on the pivot. Turning by a positive angle is
/// counter-clockwise here.
struct Plane {
    origin: DVec3,
    x: DVec3,
    y: DVec3,
    axis: Axis,
}

impl Plane {
    fn of_linkage(linkage: &Linkage) -> Plane {
        let frame = linkage.frame();
        Plane {
            origin: point_vector(frame.origin()),
            x: direction_vector(frame.free_side()),
            y: direction_vector(frame.swing_toward()),
            axis: frame.axis(),
        }
    }

    fn of(pivot: &Pivot) -> Plane {
        Plane {
            origin: point_vector(pivot.through),
            x: direction_vector(pivot.free_side),
            y: direction_vector(pivot.swing_toward),
            axis: pivot.axis,
        }
    }

    fn project(&self, point: DVec3) -> DVec2 {
        let relative = point - self.origin;
        DVec2::new(relative.dot(self.x), relative.dot(self.y))
    }
}

/// A convex polygon, counter-clockwise, with its range along the axis.
#[derive(Debug, Clone)]
struct Piece {
    corners: Vec<DVec2>,
    low: f64,
    high: f64,
}

fn corners_of(region: &WorldBox) -> [DVec3; 8] {
    let (a, b) = (point_vector(region.min()), point_vector(region.max()));
    [
        DVec3::new(a.x, a.y, a.z),
        DVec3::new(b.x, a.y, a.z),
        DVec3::new(a.x, b.y, a.z),
        DVec3::new(b.x, b.y, a.z),
        DVec3::new(a.x, a.y, b.z),
        DVec3::new(b.x, a.y, b.z),
        DVec3::new(a.x, b.y, b.z),
        DVec3::new(b.x, b.y, b.z),
    ]
}

/// The convex hull of points, counter-clockwise (monotone chain).
fn hull(mut points: Vec<DVec2>) -> Vec<DVec2> {
    points.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));
    points.dedup_by(|a, b| (*a - *b).length_squared() < 1e-6);
    if points.len() < 3 {
        return points;
    }
    let cross = |o: DVec2, a: DVec2, b: DVec2| (a - o).perp_dot(b - o);
    let mut lower: Vec<DVec2> = Vec::new();
    for p in &points {
        while lower.len() >= 2 && cross(lower[lower.len() - 2], lower[lower.len() - 1], *p) <= 0.0 {
            lower.pop();
        }
        lower.push(*p);
    }
    let mut upper: Vec<DVec2> = Vec::new();
    for p in points.iter().rev() {
        while upper.len() >= 2 && cross(upper[upper.len() - 2], upper[upper.len() - 1], *p) <= 0.0 {
            upper.pop();
        }
        upper.push(*p);
    }
    lower.pop();
    upper.pop();
    lower.extend(upper);
    lower
}

/// A convex polygon moved `inset` inward on every edge; `None` when
/// nothing is left.
fn shrink(corners: &[DVec2], inset: f64) -> Option<Vec<DVec2>> {
    let n = corners.len();
    if n < 3 {
        return None;
    }
    // Each edge as a line moved inward (left of the edge, for CCW).
    let lines: Vec<(DVec2, DVec2)> = (0..n)
        .map(|i| {
            let (a, b) = (corners[i], corners[(i + 1) % n]);
            let direction = (b - a).normalize_or_zero();
            let inward = direction.perp();
            (a + inward * inset, direction)
        })
        .collect();
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let (p, d) = lines[(i + n - 1) % n];
        let (q, e) = lines[i];
        let denominator = d.perp_dot(e);
        if denominator.abs() < 1e-12 {
            return None;
        }
        let t = (q - p).perp_dot(e) / denominator;
        out.push(p + d * t);
    }
    // Still counter-clockwise with a positive area?
    let area: f64 = (0..n).map(|i| out[i].perp_dot(out[(i + 1) % n])).sum();
    (area > 0.0).then_some(out)
}

fn piece(plane: &Plane, corners: &[DVec3], low: f64, high: f64) -> Option<Piece> {
    let flat = hull(corners.iter().map(|c| plane.project(*c)).collect());
    let half = render::nm(CONTACT_TOLERANCE) / 2.0;
    Some(Piece {
        corners: shrink(&flat, half)?,
        low,
        high,
    })
}

fn rotate(p: DVec2, angle: f64) -> DVec2 {
    DVec2::from_angle(angle).rotate(p)
}

/// Do two convex polygons overlap (the separating axis test)?
fn intersect(a: &[DVec2], b: &[DVec2]) -> bool {
    for polygon in [a, b] {
        let n = polygon.len();
        for i in 0..n {
            let edge = polygon[(i + 1) % n] - polygon[i];
            let normal = edge.perp();
            let range = |points: &[DVec2]| {
                points
                    .iter()
                    .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), p| {
                        let v = p.dot(normal);
                        (lo.min(v), hi.max(v))
                    })
            };
            let (a_lo, a_hi) = range(a);
            let (b_lo, b_hi) = range(b);
            if a_hi <= b_lo || b_hi <= a_lo {
                return false;
            }
        }
    }
    true
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
fn first_contact(moving: &[DVec2], fixed: &[DVec2], limit: f64) -> Option<f64> {
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

/// The pieces of a solid in a hinge's plane, at a placement.
fn pieces(plane: &Plane, solid: &WorldSolid, placement: &Placement) -> Vec<Piece> {
    let matrix = placement.matrix();
    wood_boxes(solid)
        .filter_map(|region| {
            let corners = corners_of(&region).map(|c| matrix.transform_point3(c));
            let along = placed_bounds(&region, placement);
            piece(
                plane,
                &corners,
                render::nm(along.min().get(plane.axis)),
                render::nm(along.max().get(plane.axis)),
            )
        })
        .collect()
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
    let plane = Plane::of(pivot);
    let limit = render::radians_f64(opening);
    let tolerance = render::nm(CONTACT_TOLERANCE);
    let moving_pieces = pieces(&plane, moving, &Placement::REST);
    let fixed_pieces = pieces(&plane, fixed, fixed_at);
    let mut best: Option<f64> = None;
    for m in &moving_pieces {
        for f in &fixed_pieces {
            if m.high.min(f.high) - m.low.max(f.low) <= tolerance {
                continue;
            }
            if let Some(angle) = first_contact(&m.corners, &f.corners, best.unwrap_or(limit))
                && best.is_none_or(|b| angle < b)
            {
                best = Some(angle);
            }
        }
    }
    best.and_then(render::angle_from_radians_floor)
}

/// The box a turning solid can reach: along the axis its own range, across
/// it the square around the circle its farthest corner turns on.
pub fn turn_reach(moving: &WorldSolid, pivot: &Pivot) -> Option<WorldBox> {
    let bounds = moving.bounds()?;
    let plane = Plane::of(pivot);
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

/// A cup hinge's linkage, ready to sweep: solved in floats, with the
/// fastest its offset changes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LinkageSweep {
    linkage: Linkage,
    path: LinkagePath,
    offset_speed: f64,
}

impl LinkageSweep {
    /// Prepares a linkage for sweeping.
    pub fn new(linkage: &Linkage) -> LinkageSweep {
        let path = LinkagePath::new(linkage);
        LinkageSweep {
            linkage: *linkage,
            path,
            offset_speed: path.offset_speed(),
        }
    }

    /// The solved linkage.
    pub fn path(&self) -> &LinkagePath {
        &self.path
    }

    /// The fastest a point this far (nanometres, in the plane) from the
    /// frame's origin moves per radian of opening.
    fn speed(&self, radius: f64) -> f64 {
        LinkagePath::speed_bound(radius, self.offset_speed)
    }
}

/// The distance from a point to a segment.
fn point_segment(p: DVec2, a: DVec2, b: DVec2) -> f64 {
    let ab = b - a;
    let length2 = ab.length_squared();
    let t = if length2 > 0.0 {
        ((p - a).dot(ab) / length2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (a + ab * t - p).length()
}

/// The distance between two convex polygons that don't overlap.
fn polygon_distance(a: &[DVec2], b: &[DVec2]) -> f64 {
    let mut best = f64::INFINITY;
    for (polygon, other) in [(a, b), (b, a)] {
        let n = other.len();
        for p in polygon {
            for i in 0..n {
                if let (Some(&from), Some(&to)) = (other.get(i), other.get((i + 1) % n)) {
                    best = best.min(point_segment(*p, from, to));
                }
            }
        }
    }
    best
}

/// A polygon of the door moved to `theta` on the linkage.
fn on_linkage(corners: &[DVec2], path: &LinkagePath, theta: f64) -> Option<Vec<DVec2>> {
    let (rotation, offset) = path.plane_pose(theta)?;
    Some(
        corners
            .iter()
            .map(|c| rotation.rotate(*c) + offset)
            .collect(),
    )
}

/// The first angle in `(0, limit]` at which the moving polygon, carried
/// by the linkage, overlaps the fixed one; `None` when it moves the whole
/// way clear (or overlaps it at rest, which the static checks report).
fn linkage_first_contact(
    moving: &[DVec2],
    fixed: &[DVec2],
    sweep: &LinkageSweep,
    limit: f64,
) -> Option<f64> {
    if intersect(moving, fixed) {
        return None;
    }
    let radius = moving.iter().map(|c| c.length()).fold(0.0, f64::max);
    let speed = sweep.speed(radius);
    let smallest = render::nm(CONTACT_TOLERANCE) / 4.0 / speed;
    let at = |theta: f64| on_linkage(moving, &sweep.path, theta);
    let mut theta = 0.0;
    let mut here = moving.to_vec();
    loop {
        let step = (polygon_distance(&here, fixed) / speed).max(smallest);
        let next = (theta + step).min(limit);
        let there = at(next)?;
        if intersect(&there, fixed) {
            // Bisect between the last clear angle and the first overlap.
            let (mut clear, mut hit) = (theta, next);
            for _ in 0..60 {
                let middle = (clear + hit) / 2.0;
                if intersect(&at(middle)?, fixed) {
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
        here = there;
    }
}

/// How far `moving` (at rest) can turn on a cup hinge's linkage before it
/// first hits `fixed` (placed at `fixed_at`); `None` when it opens the
/// whole way clear. The angle is rounded down to the micro-degree.
pub fn linkage_contact(
    moving: &WorldSolid,
    fixed: &WorldSolid,
    fixed_at: &Placement,
    sweep: &LinkageSweep,
) -> Option<Angle> {
    let plane = Plane::of_linkage(&sweep.linkage);
    let limit = sweep.path.opening();
    let tolerance = render::nm(CONTACT_TOLERANCE);
    let moving_pieces = pieces(&plane, moving, &Placement::REST);
    let fixed_pieces = pieces(&plane, fixed, fixed_at);
    let mut best: Option<f64> = None;
    for m in &moving_pieces {
        for f in &fixed_pieces {
            if m.high.min(f.high) - m.low.max(f.low) <= tolerance {
                continue;
            }
            if let Some(angle) =
                linkage_first_contact(&m.corners, &f.corners, sweep, best.unwrap_or(limit))
                && best.is_none_or(|b| angle < b)
            {
                best = Some(angle);
            }
        }
    }
    best.and_then(render::angle_from_radians_floor)
}

/// The box a solid carried by a linkage can reach: its corners at 180
/// steps of the opening (a degree or less each), grown by the most it can
/// move within half a step.
pub fn linkage_reach(moving: &WorldSolid, sweep: &LinkageSweep) -> Option<WorldBox> {
    let bounds = moving.bounds()?;
    let corners = corners_of(&bounds);
    let plane = Plane::of_linkage(&sweep.linkage);
    let radius = corners
        .iter()
        .map(|c| plane.project(*c).length())
        .fold(0.0, f64::max);
    let count: u32 = 180;
    let step = sweep.path.opening() / f64::from(count);
    let grow = sweep.speed(radius) * step / 2.0 + 1.0;
    let mut low = DVec3::splat(f64::INFINITY);
    let mut high = DVec3::splat(f64::NEG_INFINITY);
    for k in 0..=count {
        let Some(matrix) = sweep.path.coupler(step * f64::from(k)) else {
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

/// A member of a cup hinge's mechanism that can run into wood.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MechanismMember {
    /// The arm's body, fixed to the plate.
    Arm,
    /// The arm's knuckle, reaching into the cup, fixed.
    Knuckle,
    /// The first link of the linkage (the inner link).
    InnerLink,
    /// The second (the outer link, a strap).
    OuterLink,
}

/// Which of a hinge's two parts a member runs into.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum HingePart {
    /// The piece the cup is bored into (the door's stile).
    Piece,
    /// The part the plate is screwed to (the cabinet side).
    On,
}

/// Where a cup hinge's own mechanism first runs into wood as the door
/// opens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MechanismHit {
    /// The member.
    pub member: MechanismMember,
    /// The part it runs into.
    pub part: HingePart,
    /// How far the door has opened, rounded down to the micro-degree.
    pub angle: Angle,
}

/// An axis-aligned rectangle in the plane as a counter-clockwise polygon,
/// shrunk by half the contact tolerance; `None` when nothing is left.
fn rectangle(low: DVec2, high: DVec2) -> Option<Vec<DVec2>> {
    if high.x <= low.x || high.y <= low.y {
        return None;
    }
    shrink(
        &[
            low,
            DVec2::new(high.x, low.y),
            high,
            DVec2::new(low.x, high.y),
        ],
        render::nm(CONTACT_TOLERANCE) / 2.0,
    )
}

/// The plane rectangle of a world box.
fn plane_rectangle(plane: &Plane, region: &WorldBox) -> (DVec2, DVec2, f64, f64) {
    let points = corners_of(region).map(|c| plane.project(c));
    let low = points
        .iter()
        .copied()
        .fold(DVec2::splat(f64::INFINITY), DVec2::min);
    let high = points
        .iter()
        .copied()
        .fold(DVec2::splat(f64::NEG_INFINITY), DVec2::max);
    let along = |v: Point| render::nm(v.get(plane.axis));
    (low, high, along(region.min()), along(region.max()))
}

/// A link's bar at an angle: from pin to pin, past each by half its
/// thickness (as it is drawn), shrunk by half the contact tolerance.
fn link_polygon(from: DVec2, to: DVec2, thickness: f64) -> Option<Vec<DVec2>> {
    let direction = (to - from).normalize_or_zero();
    let across = direction.perp() * (thickness / 2.0);
    let (a, b) = (
        from - direction * (thickness / 2.0),
        to + direction * (thickness / 2.0),
    );
    let polygon = [a - across, b - across, b + across, a + across];
    shrink(&polygon, render::nm(CONTACT_TOLERANCE) / 2.0)
}

/// A cup hinge's mechanism in the linkage's plane, ready to be tested
/// against wood: the members' polygons at any angle, and the wood of the
/// door (less only its cup's bore) and of the side.
struct Mechanism<'a> {
    plane: Plane,
    path: &'a LinkagePath,
    /// The arm's body and knuckle, fixed, in the plane.
    arm: Option<Vec<DVec2>>,
    knuckle: Option<Vec<DVec2>>,
    links: [stave_model::solved::LinkBar; 2],
    arm_half: f64,
    /// The middle of the mechanism along the hinge axis.
    middle: f64,
    bore_centre: f64,
    bore_radius: f64,
    /// The face the cup is bored into, and the bore's bottom.
    face: f64,
    bottom: f64,
}

impl<'a> Mechanism<'a> {
    fn of(
        hardware: &stave_model::solved::Hardware,
        path: &'a LinkagePath,
    ) -> Option<Mechanism<'a>> {
        let mechanism = hardware.mechanism.as_ref()?;
        let plane = Plane::of_linkage(&mechanism.linkage);
        let middle = render::nm(mechanism.linkage.frame().origin().get(plane.axis));
        let (bore_low, bore_high, _, _) = plane_rectangle(&plane, &hardware.bounds);
        let flat = |member: &stave_model::solved::HardwareMember| {
            let (low, high, _, _) = plane_rectangle(&plane, &member.bounds);
            rectangle(low, high)
        };
        Some(Mechanism {
            arm: flat(&mechanism.arm),
            knuckle: flat(&mechanism.knuckle),
            links: mechanism.links,
            arm_half: render::nm(mechanism.arm.envelope.size().length) / 2.0,
            middle,
            bore_centre: (bore_low.x + bore_high.x) / 2.0,
            bore_radius: (bore_high.x - bore_low.x) / 2.0,
            face: bore_low.y,
            bottom: bore_high.y,
            plane,
            path,
        })
    }

    /// How wide a member is along the hinge axis, halved.
    fn half_width(&self, member: MechanismMember) -> f64 {
        match member {
            MechanismMember::Arm | MechanismMember::Knuckle => self.arm_half,
            MechanismMember::InnerLink => render::nm(self.links[0].width) / 2.0,
            MechanismMember::OuterLink => render::nm(self.links[1].width) / 2.0,
        }
    }

    /// A member's polygon at an angle (the arm and knuckle don't move).
    fn member_at(&self, member: MechanismMember, theta: f64) -> Option<Vec<DVec2>> {
        let (index, bar) = match member {
            MechanismMember::Arm => return self.arm.clone(),
            MechanismMember::Knuckle => return self.knuckle.clone(),
            MechanismMember::InnerLink => (0, self.links[0]),
            MechanismMember::OuterLink => (1, self.links[1]),
        };
        let from = self.path.pin(index, false, theta)?;
        let to = self.path.pin(index, true, theta)?;
        link_polygon(from, to, render::nm(bar.thickness))
    }

    /// The wood of a solid within `half_width` of the mechanism's middle
    /// along the axis, as rectangles in the plane at rest. With `bored`,
    /// less the cup's bore (as narrow as the bore is at the member's edge,
    /// so never more than the bore takes away): only the bore. Nothing
    /// else is set aside: a cup's rim lies on the door's face, it removes
    /// no wood, and steel resting on it is steel on steel only where it
    /// is inside the bore's mouth.
    fn wood(&self, solid: &WorldSolid, half_width: f64, bored: bool) -> Vec<Vec<DVec2>> {
        let tolerance = render::nm(CONTACT_TOLERANCE);
        let mut out = Vec::new();
        for region in wood_boxes(solid) {
            let (low, high, start, end) = plane_rectangle(&self.plane, &region);
            if end.min(self.middle + half_width) - start.max(self.middle - half_width) <= tolerance
            {
                continue;
            }
            if !bored {
                out.extend(rectangle(low, high));
                continue;
            }
            let chord = (self.bore_radius * self.bore_radius - half_width * half_width)
                .max(0.0)
                .sqrt();
            let (inner, outer) = (self.bore_centre - chord, self.bore_centre + chord);
            // The box less the bore, as rectangles.
            let parts = [
                (low, DVec2::new(inner.min(high.x), high.y)),
                (DVec2::new(outer.max(low.x), low.y), high),
                (
                    DVec2::new(inner.max(low.x), self.bottom.max(low.y)),
                    DVec2::new(outer.min(high.x), high.y),
                ),
                (
                    DVec2::new(inner.max(low.x), low.y),
                    DVec2::new(outer.min(high.x), self.face.min(high.y)),
                ),
            ];
            for (a, b) in parts {
                out.extend(rectangle(a, b));
            }
        }
        out
    }

    /// The door's wood moved to an angle.
    fn piece_at(&self, wood: &[Vec<DVec2>], theta: f64) -> Option<Vec<Vec<DVec2>>> {
        let (rotation, offset) = self.path.plane_pose(theta)?;
        Some(
            wood.iter()
                .map(|p| p.iter().map(|c| rotation.rotate(*c) + offset).collect())
                .collect(),
        )
    }
}

/// The members a mechanism test covers.
const MEMBERS: [MechanismMember; 4] = [
    MechanismMember::Arm,
    MechanismMember::Knuckle,
    MechanismMember::InnerLink,
    MechanismMember::OuterLink,
];

/// How far a cup hinge's door opens before the hinge's own arm, knuckle or
/// links run into the wood of its piece (anywhere but the cup's bore) or
/// of the part it is screwed to; `None` when they stay clear to the full
/// opening. A member already in wood at rest is caught at 0.
///
/// In the linkage's plane, over each member's width along the pins.
/// Swept by conservative advancement, each step no more than the distance
/// to the nearest wood over the fastest any point involved can move, then
/// bisected to the contact tolerance.
pub fn mechanism_contact(
    hardware: &stave_model::solved::Hardware,
    piece: &WorldSolid,
    on: &WorldSolid,
    sweep: &LinkageSweep,
) -> Option<MechanismHit> {
    let mechanism = Mechanism::of(hardware, &sweep.path)?;
    let path = &sweep.path;
    let limit = path.opening();
    let tolerance = render::nm(CONTACT_TOLERANCE);
    let reach = |polygons: &[Vec<DVec2>]| {
        polygons
            .iter()
            .flatten()
            .map(|c| c.length())
            .fold(0.0, f64::max)
    };
    let mut best: Option<(f64, MechanismMember, HingePart)> = None;
    for member in MEMBERS {
        let half_width = mechanism.half_width(member);
        let piece_wood = mechanism.wood(piece, half_width, true);
        let on_wood = mechanism.wood(on, half_width, false);
        let Some(rest) = mechanism.member_at(member, 0.0) else {
            continue;
        };
        // The door's corners move with the boss; a link's corners at most
        // twice as fast as its boss pin (it turns about its arm pin).
        let speed = sweep.speed(reach(&piece_wood)) + 2.0 * sweep.speed(reach(&[rest]));
        let smallest = tolerance / 4.0 / speed;
        for part in [HingePart::Piece, HingePart::On] {
            let overlapping = |theta: f64| -> Option<(bool, f64)> {
                let mine = mechanism.member_at(member, theta)?;
                let wood = match part {
                    HingePart::On => on_wood.clone(),
                    HingePart::Piece => mechanism.piece_at(&piece_wood, theta)?,
                };
                let mut distance = f64::INFINITY;
                for b in &wood {
                    if intersect(&mine, b) {
                        return Some((true, 0.0));
                    }
                    distance = distance.min(polygon_distance(&mine, b));
                }
                Some((false, distance))
            };
            if overlapping(0.0)?.0 {
                best = Some((0.0, member, part));
                continue;
            }
            let stop = best.map_or(limit, |(angle, _, _)| angle);
            let mut theta = 0.0;
            loop {
                let (_, distance) = overlapping(theta)?;
                let next = (theta + (distance / speed).max(smallest)).min(stop);
                let (hit, _) = overlapping(next)?;
                if hit {
                    let (mut clear, mut into) = (theta, next);
                    for _ in 0..60 {
                        let middle_angle = (clear + into) / 2.0;
                        if overlapping(middle_angle)?.0 {
                            into = middle_angle;
                        } else {
                            clear = middle_angle;
                        }
                    }
                    if best.is_none_or(|(angle, _, _)| clear < angle) {
                        best = Some((clear, member, part));
                    }
                    break;
                }
                if next >= stop {
                    break;
                }
                theta = next;
            }
        }
    }
    let (angle, member, part) = best?;
    Some(MechanismHit {
        member,
        part,
        angle: render::angle_from_radians_floor(angle)?,
    })
}

/// How deep and how much of a mechanism member is in wood at its worst
/// over the door's opening.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SteelInWood {
    /// The member.
    pub member: MechanismMember,
    /// The part.
    pub part: HingePart,
    /// Where it is worst (radians).
    pub angle: f64,
    /// How deep, nanometres: how far it would have to move to come out
    /// (the smallest overlap over the separating axes).
    pub depth: f64,
    /// The volume in wood there, cubic nanometres.
    pub volume: f64,
}

/// Measures every member of a cup hinge against the wood of its piece
/// (less only the cup's bore) and of the part it is on, at `samples + 1`
/// angles from closed to fully open: the worst depth for each member and
/// part, with the volume there. Nothing is set aside; a member that
/// never touches wood is left out.
pub fn steel_in_wood(
    hardware: &stave_model::solved::Hardware,
    piece: &WorldSolid,
    on: &WorldSolid,
    sweep: &LinkageSweep,
    samples: u32,
) -> Vec<SteelInWood> {
    let Some(mechanism) = Mechanism::of(hardware, &sweep.path) else {
        return Vec::new();
    };
    let limit = sweep.path.opening();
    let mut out = Vec::new();
    for member in MEMBERS {
        let half_width = mechanism.half_width(member);
        for (part, solid, bored) in [(HingePart::Piece, piece, true), (HingePart::On, on, false)] {
            let wood = mechanism.wood(solid, half_width, bored);
            let mut worst: Option<SteelInWood> = None;
            for k in 0..=samples.max(1) {
                let theta = limit * f64::from(k) / f64::from(samples.max(1));
                let Some(mine) = mechanism.member_at(member, theta) else {
                    continue;
                };
                let placed = match part {
                    HingePart::Piece => mechanism.piece_at(&wood, theta).unwrap_or_default(),
                    HingePart::On => wood.clone(),
                };
                for b in &placed {
                    if !intersect(&mine, b) {
                        continue;
                    }
                    let depth = overlap_depth(&mine, b);
                    let volume = clipped_area(&mine, b) * 2.0 * half_width;
                    if worst.is_none_or(|w| depth > w.depth) {
                        worst = Some(SteelInWood {
                            member,
                            part,
                            angle: theta,
                            depth,
                            volume,
                        });
                    }
                }
            }
            out.extend(worst);
        }
    }
    out
}

/// How far two overlapping convex polygons would have to move apart: the
/// smallest overlap of their shadows over every edge normal.
fn overlap_depth(a: &[DVec2], b: &[DVec2]) -> f64 {
    let mut best = f64::INFINITY;
    for polygon in [a, b] {
        let n = polygon.len();
        for i in 0..n {
            let normal = (polygon[(i + 1) % n] - polygon[i])
                .perp()
                .normalize_or_zero();
            let range = |points: &[DVec2]| {
                points
                    .iter()
                    .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), p| {
                        let v = p.dot(normal);
                        (lo.min(v), hi.max(v))
                    })
            };
            let ((a_lo, a_hi), (b_lo, b_hi)) = (range(a), range(b));
            best = best.min(a_hi.min(b_hi) - a_lo.max(b_lo));
        }
    }
    best.max(0.0)
}

/// The area two convex counter-clockwise polygons share.
fn clipped_area(subject: &[DVec2], clip: &[DVec2]) -> f64 {
    let mut polygon: Vec<DVec2> = subject.to_vec();
    let n = clip.len();
    for i in 0..n {
        let (a, b) = (clip[i], clip[(i + 1) % n]);
        let inside = |p: DVec2| (b - a).perp_dot(p - a) >= 0.0;
        let mut next = Vec::new();
        for j in 0..polygon.len() {
            let (p, q) = (polygon[j], polygon[(j + 1) % polygon.len()]);
            if inside(p) {
                next.push(p);
            }
            if inside(p) != inside(q) {
                let d = q - p;
                let t = (b - a).perp_dot(a - p) / (b - a).perp_dot(d);
                next.push(p + d * t);
            }
        }
        polygon = next;
        if polygon.len() < 3 {
            return 0.0;
        }
    }
    let area: f64 = (0..polygon.len())
        .map(|i| polygon[i].perp_dot(polygon[(i + 1) % polygon.len()]))
        .sum();
    area.abs() / 2.0
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
    use stave_schema::hinges::{CUP_HINGE_110, CupPoint};

    fn mm(value: i64) -> Length {
        Length::from_mm(value)
    }

    /// The built-in cup hinge on a 19mm door in a frame at the door's
    /// front hinge corner (the world origin): the door's back face is at
    /// deep 19mm, the side's front edge behind it.
    fn cup_hinge() -> Linkage {
        let place = |p: CupPoint| {
            PlanePoint::new(
                Length::from_nm(22_500_000 + p.x),
                Length::from_nm(p.y - 19_000_000),
            )
        };
        let links = CUP_HINGE_110.links.map(|link| Link {
            ground: place(link.arm),
            coupler: place(link.boss),
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
        let sweep = LinkageSweep::new(&cup_hinge());
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
        for fixed in [&post, &wall] {
            let found = linkage_contact(&door, fixed, &Placement::REST, &sweep).unwrap();
            let plane = Plane::of_linkage(&sweep.linkage);
            let moving = pieces(&plane, &door, &Placement::REST);
            let still = pieces(&plane, fixed, &Placement::REST);
            let (m, f) = (&moving[0].corners, &still[0].corners);
            let dense = (1..=11_000)
                .map(|k| (f64::from(k) / 100.0).to_radians())
                .find(|theta| intersect(&on_linkage(m, sweep.path(), *theta).unwrap(), f))
                .unwrap();
            let found_radians = render::radians_f64(found);
            assert!(found_radians <= dense, "{found} after {dense}");
            assert!(dense - found_radians < 0.0002, "{found} vs {dense}");
        }
        // Nothing in the way: clear.
        let far = block([600, 0, -200], [640, 700, -150]);
        assert_eq!(linkage_contact(&door, &far, &Placement::REST, &sweep), None);
        // The side the door rests on: the linkage lifts it off.
        let side = block([-19, 0, 19], [0, 700, 400]);
        assert_eq!(
            linkage_contact(&door, &side, &Placement::REST, &sweep),
            None
        );
    }
}
