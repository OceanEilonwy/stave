//! Running clearance (grammar, "Running clearance"): the least gap
//! between a body a motion moves and a body it moves past, over the whole
//! sweep, leaving out **seating** (a closest approach at an end of the
//! range where the gap opens as the motion leaves it), with the line of
//! that closest approach as each body's own axis nearest to it.
//!
//! Each body is its wood as whole pieces: a part's boxes with its square
//! cuts out, its holes counted as wood, so a gap is never found larger
//! than it is. The distance between two bodies is the least distance
//! between any two of their pieces, and each pair of pieces is measured on
//! its own: a closest approach of one pair that is seating is left out
//! even where another pair of the same bodies rubs.
//!
//! - **A slide** moves boxes along one direction: between two boxes the
//!   gap across that direction never changes and the gap along it falls,
//!   stays at nothing while they pass, then grows. So two boxes come
//!   closest either at an end, where the gap opens away from it (seating),
//!   or over the stretch where they pass, at the gap across: exact, in
//!   whole nanometres.
//! - **A turn about one fixed line** reduces to the plane across the line,
//!   where each piece is a convex polygon with a range along the line;
//!   that range never changes, so the distance is the distance in the
//!   plane and the gap along the line together. In the plane a closest
//!   approach is a corner of one polygon nearest an edge of the other, or
//!   nearest one of its ends, and each happens at an angle found in closed
//!   form (the corner's circle against the edge's line and ends), or where
//!   the polygons start to overlap (the circle crossing the edge). Every
//!   such angle is tried.
//! - **Any other motion** (a linkage, a body an inner assembly holds
//!   turned) is stepped by conservative advancement in the mechanism's
//!   plane: by how far the pieces are apart, less the gap that matters,
//!   over the fastest their corners can close it, and by no more than
//!   0.1mm of movement once within it; each closest approach found is then
//!   narrowed by golden-section search.
//!
//! Each closest approach is then tried a millionth of the range either
//! side: it counts when neither side is closer (a closest approach, or a
//! stretch where the gap holds steady), and at an end of the range only
//! when the gap holds steady as the motion leaves it, since one that opens
//! is seating. Floats are `f64` nanometres and radians, converted in
//! [`render`]; the gap is rounded down to the nanometre.

use std::f64::consts::{PI, TAU};

use glam::{DMat4, DVec2, DVec3};
use stave_lang::placement::{Axis, Direction};
use stave_lang::units::Length;
use stave_model::mechanism::MotionShape;
use stave_model::solved::PoseValue;

use crate::collide::{CONTACT_TOLERANCE, Placement, WorldSolid, axis_vector};
use crate::plane::{
    Motion2, Piece, Plane, intersect, nearest_direction, overlap_depth, polygon_distance,
    separation, whole_pieces,
};
use crate::render;
use crate::sweep::{Mover, Path, crossings, placed_boxes, shared, wood_boxes};

/// Where a moving body comes closest to a body it moves past.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LeastGap {
    /// The least gap, rounded down to the nanometre.
    pub gap: Length,
    /// Where along the motion: an angle or a distance from closed (the
    /// first, when it holds steady).
    pub at: PoseValue,
    /// `true` when the gap stays at its least over the whole range.
    pub whole_way: bool,
    /// The moving body's own axis (at rest) nearest the line of the
    /// closest approach.
    pub moving_axis: Axis,
    /// The other body's own axis (at rest) nearest that line.
    pub fixed_axis: Axis,
}

/// Two distances closer than this (nanometres) are the same: what the
/// floats can't tell apart.
const SAME: f64 = 0.01;

/// A gap that varies by no more than this (nanometres) holds steady for
/// "the whole way".
const STEADY: f64 = 500.0;

/// The most any point moves between two samples of a stepped motion once
/// the pieces are within the gap that matters (nanometres): 0.1mm.
const FINE: f64 = 100_000.0;

/// How far either side of a closest approach it is tried, as a fraction of
/// the range.
const NEAR: f64 = 1e-6;

/// The most samples one pair of pieces is stepped through.
const MOST_SAMPLES: u32 = 200_000;

/// The least gap, leaving out seating, between `moving`, moved by `mover`
/// (one piece with a swept assembly: [`Mover::Rigid`] or
/// [`Mover::Carried`]), and `fixed` at `placement`, over the whole motion;
/// `None` when they never come closer than `within`, or when `mover`
/// doesn't move a body that way.
pub fn least_gap(
    moving: &WorldSolid,
    mover: Mover<'_>,
    fixed: &WorldSolid,
    placement: Placement,
    within: Length,
) -> Option<LeastGap> {
    let path = match mover {
        Mover::Rigid(path) | Mover::Carried(path, _) => path,
        Mover::Fixed(_) | Mover::Member(..) => return None,
    };
    let within = render::nm(within);
    match (mover, path.relative_shape()) {
        (Mover::Rigid(_), MotionShape::Travel { toward, travel }) => slide(
            moving,
            fixed,
            &path.relative(&placement),
            toward,
            travel,
            within,
        ),
        (Mover::Rigid(_), MotionShape::Turn { line, opening }) => {
            let plane = Plane::of_pivot(&line);
            let fixed_at = path.relative(&placement);
            let flat = Flat::new(
                plane,
                &whole_pieces(&plane, moving, &DMat4::IDENTITY),
                &whole_pieces(&plane, fixed, &fixed_at.matrix()),
                render::radians_f64(opening),
                within,
                &|angle| {
                    Some(Motion2 {
                        rotation: DVec2::from_angle(angle),
                        offset: DVec2::ZERO,
                    })
                },
            );
            let found = flat.least(&|pair| pair.turn_candidates(flat.limit))?;
            flat.result(found, path, DMat4::IDENTITY, fixed_at.matrix())
        }
        (Mover::Rigid(_) | Mover::Carried(..), _) => {
            let plane = path.plane();
            let start = match mover {
                Mover::Carried(_, held) => held.matrix(),
                Mover::Rigid(_) | Mover::Fixed(_) | Mover::Member(..) => DMat4::IDENTITY,
            };
            let motion = |amount: f64| path.motion2(&mover, amount);
            let flat = Flat::new(
                plane,
                &whole_pieces(&plane, moving, &start),
                &whole_pieces(&plane, fixed, &placement.matrix()),
                path.limit(),
                within,
                &motion,
            );
            let found = flat.least(&|pair| {
                let speed = path.speed(&mover, &pair.moving);
                flat.stepped_candidates(pair, speed)
            })?;
            flat.result(found, path, start, placement.matrix())
        }
        (Mover::Fixed(_) | Mover::Member(..), _) => None,
    }
}

/// The world axis a direction lies nearest (the first of equals).
fn nearest_axis(direction: DVec3) -> Axis {
    let mut best = (Axis::Across, f64::NEG_INFINITY);
    for axis in Axis::ALL {
        let along = direction.dot(axis_vector(axis)).abs();
        if along > best.1 {
            best = (axis, along);
        }
    }
    best.0
}

/// A direction in the world as a body placed by `matrix` sees it at rest.
fn own_direction(matrix: &DMat4, direction: DVec3) -> DVec3 {
    matrix.inverse().transform_vector3(direction)
}

// ---------------------------------------------------------------------------
// Slides: exact
// ---------------------------------------------------------------------------

/// A slide, box by box (see the module docs).
fn slide(
    moving: &WorldSolid,
    fixed: &WorldSolid,
    fixed_at: &Placement,
    toward: Direction,
    travel: Length,
    within: f64,
) -> Option<LeastGap> {
    let axis = toward.axis();
    let [first, second] = axis.others();
    let tolerance = CONTACT_TOLERANCE.nm();
    let total = travel.nm().max(0);
    let fixed_boxes = placed_boxes(fixed, fixed_at);
    // The least gap: its square, where along the travel, whether it holds
    // the whole way, and the axis across the travel it is measured along.
    let mut best: Option<(i128, i64, bool, Axis)> = None;
    for m in wood_boxes(moving) {
        for f in &fixed_boxes {
            // How far apart across the travel (negative where they
            // overlap), which never changes.
            let (gap_first, gap_second) = (-shared(&m, f, first), -shared(&m, f, second));
            let across = |gap: i64| i128::from(gap.max(0));
            let squared =
                across(gap_first) * across(gap_first) + across(gap_second) * across(gap_second);
            if render::nm(Length::from_nm(
                i64::try_from(isqrt(squared)).unwrap_or(i64::MAX),
            )) >= within
            {
                continue;
            }
            // Along the travel, the moving range is (m0 + t, m1 + t), the
            // travel mirrored so that it runs up the axis.
            let range = |region: &stave_model::solved::WorldBox| {
                (region.min().get(axis).nm(), region.max().get(axis).nm())
            };
            let ((m0, m1), (f0, f1)) = (range(&m), range(f));
            let (m0, m1, f0, f1) = if toward.is_positive() {
                (m0, m1, f0, f1)
            } else {
                (-m1, -m0, -f1, -f0)
            };
            // Overlapping already where it starts: the static checks'.
            let along_shared = m1.min(f1).saturating_sub(m0.max(f0));
            if gap_first < -tolerance && gap_second < -tolerance && along_shared > tolerance {
                continue;
            }
            // They pass each other for t in [f0 - m1, f1 - m0]: there the
            // gap is the gap across, and anywhere else it is larger and
            // grows away from that stretch.
            let low = f0.saturating_sub(m1).max(0);
            let high = f1.saturating_sub(m0).min(total);
            if high <= low {
                // Never passing within the travel: closest at one end,
                // and opening away from it. Seating.
                continue;
            }
            let whole_way = low == 0 && high == total;
            let measured = if gap_first >= gap_second {
                first
            } else {
                second
            };
            if best.is_none_or(|(b, at, ..)| squared < b || (squared == b && low < at)) {
                best = Some((squared, low, whole_way, measured));
            }
        }
    }
    let (squared, at, whole_way, measured) = best?;
    let gap = Length::from_nm(i64::try_from(isqrt(squared)).unwrap_or(i64::MAX));
    let fixed_axis = match fixed_at {
        Placement::Exact(_) => measured,
        Placement::Turned(matrix) => nearest_axis(own_direction(matrix, axis_vector(measured))),
    };
    Some(LeastGap {
        gap,
        at: PoseValue::Travel(Length::from_nm(at)),
        whole_way,
        moving_axis: measured,
        fixed_axis,
    })
}

/// The floor of a square root.
fn isqrt(value: i128) -> i128 {
    if value <= 0 {
        return 0;
    }
    let mut root = value;
    loop {
        let next = (root + value / root) / 2;
        if next >= root {
            return root;
        }
        root = next;
    }
}

// ---------------------------------------------------------------------------
// Turns and other motions: in the plane
// ---------------------------------------------------------------------------

/// A piece of the moving body and a piece of the other, where the motion
/// starts, with how far apart they are along the plane's axis (negative
/// where their ranges overlap), which the motion never changes.
struct FlatPair {
    moving: Piece,
    fixed: Piece,
    along: f64,
}

impl FlatPair {
    /// The closed-form angles in `(0, limit)` at which a closest approach
    /// of the pair, the moving piece turning counter-clockwise about the
    /// plane's origin, can happen, with both ends of the range.
    fn turn_candidates(&self, limit: f64) -> Vec<f64> {
        let mut out = vec![0.0, limit];
        let push = |angle: f64, out: &mut Vec<f64>| {
            let angle = angle.rem_euclid(TAU);
            if angle > 0.0 && angle < limit {
                out.push(angle);
            }
        };
        let (moving, fixed) = (&self.moving.corners, &self.fixed.corners);
        for (polygon, other, forward) in [(moving, fixed, true), (fixed, moving, false)] {
            let count = other.len();
            for corner in polygon {
                for index in 0..count {
                    let (Some(a), Some(b)) = (other.get(index), other.get((index + 1) % count))
                    else {
                        continue;
                    };
                    let normal = (*b - *a).perp();
                    // The angles at which the corner, turning (or the
                    // edge turning past it), lies straight out from the
                    // edge's line, and in line with each of its ends.
                    let targets = [
                        normal.to_angle(),
                        normal.to_angle() + PI,
                        a.to_angle(),
                        b.to_angle(),
                    ];
                    for target in targets {
                        let angle = if forward {
                            target - corner.to_angle()
                        } else {
                            corner.to_angle() - target
                        };
                        push(angle, &mut out);
                    }
                    let mut crossing = Vec::new();
                    crossings(*corner, *a, *b, forward, limit, &mut crossing);
                    for angle in crossing {
                        push(angle, &mut out);
                    }
                }
            }
        }
        out.sort_by(f64::total_cmp);
        out.dedup_by(|a, b| (*a - *b).abs() < 1e-12);
        out
    }
}

/// A motion in a plane, with the pairs of pieces of two bodies that can
/// come within the gap that matters.
struct Flat<'a> {
    plane: Plane,
    pairs: Vec<FlatPair>,
    limit: f64,
    within: f64,
    motion: &'a dyn Fn(f64) -> Option<Motion2>,
}

/// The least gap of one pair: the distance, where, and the pair.
#[derive(Debug, Clone, Copy)]
struct Found {
    distance: f64,
    at: f64,
    pair: usize,
}

impl<'a> Flat<'a> {
    fn new(
        plane: Plane,
        moving: &[Piece],
        fixed: &[Piece],
        limit: f64,
        within: f64,
        motion: &'a dyn Fn(f64) -> Option<Motion2>,
    ) -> Flat<'a> {
        let tolerance = render::nm(CONTACT_TOLERANCE);
        let start = motion(0.0);
        let mut pairs = Vec::new();
        for p in moving {
            for q in fixed {
                let along = (q.low - p.high).max(p.low - q.high);
                if along >= within {
                    continue;
                }
                // Overlapping already where the motion starts: the static
                // checks' (a round shape in its hole, a cut's room).
                if let Some(start) = start {
                    let placed = p.moved(start);
                    if along < -tolerance
                        && intersect(&placed.corners, &q.corners)
                        && overlap_depth(&placed.corners, &q.corners).0 > tolerance
                    {
                        continue;
                    }
                }
                pairs.push(FlatPair {
                    moving: p.clone(),
                    fixed: q.clone(),
                    along,
                });
            }
        }
        Flat {
            plane,
            pairs,
            limit,
            within,
            motion,
        }
    }

    /// The signed distance between a pair's pieces `amount` along the
    /// motion: how far apart, or, where they overlap (within the contact
    /// tolerance, since anything deeper is the sweep's), less than nothing
    /// by how far they would have to move apart. Signed, so that a
    /// closest approach that goes on closing to the end of the range is
    /// seen as seating, not as a stretch of touching.
    fn distance(&self, pair: &FlatPair, amount: f64) -> Option<f64> {
        let moved = pair.moving.moved((self.motion)(amount)?);
        let flat = if intersect(&moved.corners, &pair.fixed.corners) {
            separation(&moved.corners, &pair.fixed.corners).0.min(0.0)
        } else {
            polygon_distance(&moved.corners, &pair.fixed.corners)
        };
        Some(match (flat >= 0.0, pair.along >= 0.0) {
            (true, true) => flat.hypot(pair.along),
            (true, false) => flat,
            (false, true) => pair.along,
            (false, false) => flat.max(pair.along),
        })
    }

    /// The candidates of a stepped motion for one pair (see the module
    /// docs), its corners moving at most `speed` per unit of the motion.
    fn stepped_candidates(&self, pair: &FlatPair, speed: f64) -> Vec<f64> {
        let mut out = vec![0.0, self.limit];
        if !(speed > 0.0 && speed.is_finite()) {
            return out;
        }
        let fine = FINE / speed;
        let mut samples: Vec<(f64, f64)> = Vec::new();
        let mut amount = 0.0;
        for _ in 0..MOST_SAMPLES {
            let Some(distance) = self.distance(pair, amount) else {
                break;
            };
            samples.push((amount, distance));
            if amount >= self.limit {
                break;
            }
            let step = if distance > self.within + FINE {
                ((distance - self.within) / speed).max(fine)
            } else {
                fine
            };
            amount = (amount + step).min(self.limit);
        }
        for window in samples.windows(3) {
            let &[(before, d0), (here, d1), (after, d2)] = window else {
                continue;
            };
            if d1 >= self.within || d1 > d0 + SAME || d1 > d2 + SAME {
                continue;
            }
            // Holding steady: the first of the stretch is enough.
            if (d1 - d0).abs() <= SAME {
                continue;
            }
            out.push(here);
            if let Some(narrowed) = self.narrow(pair, before, after) {
                out.push(narrowed);
            }
        }
        out.sort_by(f64::total_cmp);
        out.dedup_by(|a, b| (*a - *b).abs() < 1e-12);
        out
    }

    /// The closest approach between `low` and `high` by golden-section
    /// search.
    fn narrow(&self, pair: &FlatPair, low: f64, high: f64) -> Option<f64> {
        let ratio = (5.0_f64.sqrt() - 1.0) / 2.0;
        let (mut a, mut b) = (low, high);
        let mut c = b - ratio * (b - a);
        let mut d = a + ratio * (b - a);
        let mut fc = self.distance(pair, c)?;
        let mut fd = self.distance(pair, d)?;
        for _ in 0..60 {
            if fc <= fd {
                b = d;
                d = c;
                fd = fc;
                c = b - ratio * (b - a);
                fc = self.distance(pair, c)?;
            } else {
                a = c;
                c = d;
                fc = fd;
                d = a + ratio * (b - a);
                fd = self.distance(pair, d)?;
            }
        }
        Some((a + b) / 2.0)
    }

    /// The least gap over every pair, each pair's candidates from
    /// `candidates`, seating left out.
    fn least(&self, candidates: &dyn Fn(&FlatPair) -> Vec<f64>) -> Option<Found> {
        let delta = self.limit * NEAR;
        let mut best: Option<Found> = None;
        for (index, pair) in self.pairs.iter().enumerate() {
            for amount in candidates(pair) {
                let Some(distance) = self.distance(pair, amount) else {
                    continue;
                };
                if distance >= self.within
                    || best.is_some_and(|b| {
                        distance > b.distance || (distance == b.distance && amount >= b.at)
                    })
                {
                    continue;
                }
                let side = |at: f64| {
                    (0.0..=self.limit)
                        .contains(&at)
                        .then(|| self.distance(pair, at))
                        .flatten()
                };
                let counts = match (side(amount - delta), side(amount + delta)) {
                    (Some(before), Some(after)) => {
                        distance <= before + SAME && distance <= after + SAME
                    }
                    // At an end: only where it holds steady as the motion
                    // leaves it. A gap that opens is seating.
                    (Some(beside), None) | (None, Some(beside)) => {
                        (distance - beside).abs() <= SAME
                    }
                    (None, None) => false,
                };
                if counts {
                    best = Some(Found {
                        distance,
                        at: amount,
                        pair: index,
                    });
                }
            }
        }
        best
    }

    /// Whether the two bodies' gap (the least over every pair of pieces)
    /// stays within [`STEADY`] of `least` over the whole range.
    fn steady(&self, least: f64, candidates: &[f64]) -> bool {
        let count: u32 = 64;
        (0..=count)
            .map(|k| self.limit * f64::from(k) / f64::from(count))
            .chain(candidates.iter().copied())
            .all(|amount| {
                self.pairs.iter().any(|pair| {
                    self.distance(pair, amount)
                        .is_some_and(|distance| distance <= least + STEADY)
                })
            })
    }

    /// The least gap as a result: rounded, with where it is, whether it
    /// holds the whole way, and the line of the closest approach as each
    /// body sees it at rest (`moving_start` and `fixed_matrix` place the
    /// pieces where the motion starts from rest).
    fn result(
        &self,
        found: Found,
        path: &Path,
        moving_start: DMat4,
        fixed_matrix: DMat4,
    ) -> Option<LeastGap> {
        let pair = self.pairs.get(found.pair)?;
        let motion = (self.motion)(found.at)?;
        let moved = pair.moving.moved(motion);
        let candidates = [found.at];
        let whole_way = self.steady(found.distance, &candidates);
        // The line of the closest approach: between the nearest points,
        // in the plane and along its axis; where they touch, the way they
        // are least deep into each other.
        let axis = axis_vector(self.plane.axis);
        let in_plane = |v: DVec2| self.plane.x * v.x + self.plane.y * v.y;
        let (flat, normal) = if intersect(&moved.corners, &pair.fixed.corners) {
            let (depth, normal) = separation(&moved.corners, &pair.fixed.corners);
            (depth.min(0.0), normal)
        } else {
            nearest_direction(&moved.corners, &pair.fixed.corners)
        };
        let direction = match (flat > 0.0, pair.along > 0.0) {
            (true, true) => in_plane(normal) * flat + axis * pair.along,
            (true, false) => in_plane(normal),
            (false, true) => axis,
            (false, false) if pair.along >= flat => axis,
            (false, false) => in_plane(normal),
        };
        // The moving body is turned by the motion from where it starts.
        let back = DVec2::new(motion.rotation.x, -motion.rotation.y);
        let flat_part = back.rotate(DVec2::new(
            direction.dot(self.plane.x),
            direction.dot(self.plane.y),
        ));
        let own = in_plane(flat_part) + axis * direction.dot(axis);
        Some(LeastGap {
            gap: render::length_from_nm(found.distance.max(0.0).floor())?,
            at: path.mechanism().value_of(found.at)?,
            whole_way,
            moving_axis: nearest_axis(own_direction(&moving_start, own)),
            fixed_axis: nearest_axis(own_direction(&fixed_matrix, direction)),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use stave_model::solved::{Point, WorldBox};

    fn mm(value: i64) -> Length {
        Length::from_mm(value)
    }

    fn block(min: [i64; 3], max: [i64; 3]) -> WorldSolid {
        let point = |p: [i64; 3]| Point {
            across: mm(p[0]),
            up: mm(p[1]),
            deep: mm(p[2]),
        };
        WorldSolid::from_boxes(vec![WorldBox::new(point(min), point(max))])
    }

    #[test]
    fn square_roots_round_down() {
        assert_eq!(isqrt(0), 0);
        assert_eq!(isqrt(15), 3);
        assert_eq!(isqrt(16), 4);
    }

    #[test]
    fn a_slide_past_a_block_measures_the_gap_across() {
        // A drawer side 2mm from a runner, slid 300mm along it.
        let moving = block([0, 0, 0], [10, 100, 400]);
        let fixed = block([12, 0, 0], [30, 100, 400]);
        let found = slide(
            &moving,
            &fixed,
            &Placement::REST,
            Direction::Front,
            mm(300),
            render::nm(mm(5)),
        )
        .expect("they pass 2mm apart");
        assert_eq!(found.gap, mm(2));
        assert!(found.whole_way);
        assert_eq!(found.moving_axis, Axis::Across);
        assert_eq!(found.at, PoseValue::Travel(Length::ZERO));
    }

    #[test]
    fn a_slide_away_from_what_it_rests_on_is_seating() {
        // A drawer front against the cabinet, pulled straight out.
        let moving = block([0, 0, -20], [100, 100, 0]);
        let fixed = block([0, 0, 0], [100, 100, 400]);
        let toward = Direction::Front;
        let found = slide(
            &moving,
            &fixed,
            &Placement::REST,
            toward,
            mm(300),
            render::nm(mm(5)),
        );
        // Whichever way is out, one of the two is seating; the other runs
        // into the cabinet, which the sweep reports.
        let other = slide(
            &moving,
            &fixed,
            &Placement::REST,
            toward.opposite(),
            mm(300),
            render::nm(mm(5)),
        );
        assert!(found.is_none() || other.is_none(), "{found:?} {other:?}");
    }

    #[test]
    fn turn_candidates_include_both_ends() {
        let square = |x: f64, y: f64| Piece {
            corners: vec![
                DVec2::new(x, y),
                DVec2::new(x + 1.0, y),
                DVec2::new(x + 1.0, y + 1.0),
                DVec2::new(x, y + 1.0),
            ],
            low: 0.0,
            high: 1.0,
            round: None,
        };
        let pair = FlatPair {
            moving: square(5.0, 0.0),
            fixed: square(0.0, 5.0),
            along: -1.0,
        };
        let found = pair.turn_candidates(PI);
        assert_eq!(found.first().copied(), Some(0.0));
        assert_eq!(found.last().copied(), Some(PI));
        assert!(found.len() > 2);
    }
}
