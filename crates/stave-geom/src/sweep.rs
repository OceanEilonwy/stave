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
//! Approximations (each finds more wood than there is, never less): a rod
//! is projected as the box around it (exact for a knob, whose axis lies in
//! the plane; a rod along the hinge axis becomes its square), holes are
//! left out, and a fixed solid turned about another axis is projected as
//! the outline of its corners.

use std::f64::consts::TAU;

use glam::{DVec2, DVec3};
use stave_lang::placement::{Axis, Direction};
use stave_lang::units::{Angle, Length};
use stave_model::solved::{Pivot, Point, WorldBox};

use crate::collide::{
    CONTACT_TOLERANCE, Placement, WorldRod, WorldSolid, direction_vector, placed_bounds,
    point_vector, translate_box,
};
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
