//! How close a moving assembly comes to the parts around it over its whole
//! travel: the smallest gap ("closest to the side: 1.4 mm").
//!
//! Parts touching at rest (a door's back against the side it covers, a
//! drawer front against the cabinet) are measured from where they start to
//! move apart: a swing is measured after the gap first stops growing, so
//! the closest approach on the way round counts but the contact at rest
//! doesn't; a slide's box pairs touching at rest only ever move apart or
//! slide along each other, and are left out.
//!
//! - A **slide** moves boxes along an axis: the gap between two boxes is
//!   then the same on the two other axes all the way and changes linearly
//!   along the travel, so the smallest gap is found **exactly**, in whole
//!   nanometres (rounded down to the nanometre at the square root).
//! - A **swing** (a hinge turning, a cup hinge's linkage) is **sampled**,
//!   then refined where the gap could be smaller than the best found:
//!   between two samples no point of the moving assembly moves more than
//!   [`SWING_STEP`] (0.1mm), so the true smallest gap is at most half that
//!   (0.05mm, the contact tolerance) less than the one reported. Gaps
//!   between turned boxes are exact distances between the boxes (vertex to
//!   box and edge to edge); holes are left out, which only ever finds less
//!   room than there is.

use glam::{DMat4, DVec3};
use stave_lang::placement::{Axis, Direction};
use stave_lang::units::Length;
use stave_model::solved::{Motion, PoseValue, WorldBox};

use crate::collide::{CONTACT_TOLERANCE, Placement, WorldRod, WorldSolid, point_vector};
use crate::render;

/// A swing is refined until no point moves more than this between
/// samples (nanometres): 0.1mm.
pub const SWING_STEP: f64 = 100_000.0;

/// What a moving assembly comes closest to.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Closest {
    /// The index of the fixed solid.
    pub fixed: usize,
    /// The smallest gap.
    pub gap: Length,
    /// Where along the travel: radians for a swing, nanometres for a
    /// slide.
    pub at: f64,
    /// The two nearest points there (world nanometres): on the moving
    /// assembly as moved, and on the fixed solid.
    pub points: Option<(DVec3, DVec3)>,
}

/// How an assembly moves.
pub enum Travel<'a> {
    /// Along a direction, up to a distance.
    Slide {
        /// The direction.
        toward: Direction,
        /// How far.
        travel: Length,
    },
    /// Turned through `0..=range` radians; `pose` gives the placement
    /// (world nanometres to world nanometres) at each angle.
    Swing {
        /// The placement at an angle; `None` where the mechanism doesn't
        /// reach.
        pose: &'a dyn Fn(f64) -> Option<DMat4>,
        /// The whole travel, radians.
        range: f64,
        /// The world axis the swing turns about, when it turns about one
        /// (a hinge's pin, a cup hinge's linkage plane): nothing moves
        /// along it, so the gap along it between two boxes never changes
        /// and bounds the gap between them all the way.
        about: Option<Axis>,
    },
}

/// The boxes of a solid: its blocks and the boxes around its rods.
fn boxes(solid: &WorldSolid) -> Vec<WorldBox> {
    solid
        .boxes()
        .iter()
        .copied()
        .chain(solid.rods().iter().map(WorldRod::bounds))
        .collect()
}

/// The smallest gap between the `moving` solids and each of the `fixed`
/// ones over the travel; `None` when nothing comes near.
pub fn closest(
    moving: &[WorldSolid],
    fixed: &[WorldSolid],
    travel: &Travel<'_>,
) -> Option<Closest> {
    closest_near(moving, fixed, travel, None)
}

/// [`closest`], with the nearest points placed as near `prefer` as they
/// can be without changing the gap: where the two nearest boxes run side
/// by side along an axis the motion doesn't move along, any point of that
/// run is as near as any other, and the one level with `prefer` (the
/// hardware) is the one worth showing.
pub fn closest_near(
    moving: &[WorldSolid],
    fixed: &[WorldSolid],
    travel: &Travel<'_>,
    prefer: Option<DVec3>,
) -> Option<Closest> {
    let moving: Vec<WorldBox> = moving.iter().flat_map(boxes).collect();
    let fixed: Vec<Vec<WorldBox>> = fixed.iter().map(boxes).collect();
    match travel {
        Travel::Slide { toward, travel } => slide(&moving, &fixed, *toward, *travel, prefer),
        Travel::Swing { pose, range, about } => {
            swing(&moving, &fixed, *pose, *range, *about, prefer)
        }
    }
}

/// The middle of a box in world nanometres, for [`closest_near`]'s
/// `prefer`.
pub fn box_centre(region: &WorldBox) -> DVec3 {
    (point_vector(region.min()) + point_vector(region.max())) / 2.0
}

/// Slides two nearest points together along `axis` toward `prefer`,
/// within the range both boxes cover there (`low..high`, nanometres).
fn slide_points(points: &mut (DVec3, DVec3), slot: usize, low: f64, high: f64, prefer: f64) {
    if low > high {
        return;
    }
    let (a, b) = (points.0[slot], points.1[slot]);
    if (a - b).abs() > 1.0 {
        return;
    }
    let at = prefer.clamp(low, high);
    points.0[slot] = at;
    points.1[slot] = at;
}

// ---------------------------------------------------------------------------
// Slides: exact
// ---------------------------------------------------------------------------

/// The gap between two ranges (0 when they meet or overlap).
fn range_gap(a: (i64, i64), b: (i64, i64)) -> i64 {
    (b.0.saturating_sub(a.1))
        .max(a.0.saturating_sub(b.1))
        .max(0)
}

fn axis_range(region: &WorldBox, axis: Axis) -> (i64, i64) {
    (region.min().get(axis).nm(), region.max().get(axis).nm())
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

fn slide(
    moving: &[WorldBox],
    fixed: &[Vec<WorldBox>],
    toward: Direction,
    travel: Length,
    prefer: Option<DVec3>,
) -> Option<Closest> {
    let axis = toward.axis();
    let [first, second] = axis.others();
    let tolerance = i128::from(CONTACT_TOLERANCE.nm());
    let total = travel.nm().max(0);
    let mut best: Option<(i128, usize, i64, WorldBox, WorldBox)> = None;
    for (index, boxes) in fixed.iter().enumerate() {
        for m in moving {
            for f in boxes {
                let across = [first, second]
                    .map(|a| i128::from(range_gap(axis_range(m, a), axis_range(f, a))));
                let along_rest = i128::from(range_gap(axis_range(m, axis), axis_range(f, axis)));
                let rest = across[0] * across[0] + across[1] * across[1] + along_rest * along_rest;
                if rest <= tolerance * tolerance {
                    continue;
                }
                // Moved by t along the axis (t from 0 to the travel), the
                // moving range is (m0 + st, m1 + st), s the direction's
                // sign. Mirror a negative travel so s is +1.
                let (m0, m1) = axis_range(m, axis);
                let (f0, f1) = axis_range(f, axis);
                let (m0, m1, f0, f1) = if toward.is_positive() {
                    (m0, m1, f0, f1)
                } else {
                    (-m1, -m0, -f1, -f0)
                };
                // Overlapping along the axis for t in [f0 - m1, f1 - m0].
                let (low, high) = (f0.saturating_sub(m1), f1.saturating_sub(m0));
                let (along, at) = if high < 0 {
                    (m0.saturating_sub(f1), 0)
                } else if low > total {
                    (f0.saturating_sub(m1).saturating_sub(total), total)
                } else {
                    (0, low.max(0))
                };
                let along = i128::from(along);
                let gap = across[0] * across[0] + across[1] * across[1] + along * along;
                if best.is_none_or(|(b, ..)| gap < b) {
                    best = Some((gap, index, at, *m, *f));
                }
            }
        }
    }
    let (squared, index, at, m, f) = best?;
    // The nearest points: on each axis the facing ends, or the middle of
    // where the two overlap; the moving box moved to where it is nearest.
    let shift = if toward.is_positive() { at } else { -at };
    let mut on_moving = DVec3::ZERO;
    let mut on_fixed = DVec3::ZERO;
    for (slot, world_axis) in [Axis::Across, Axis::Up, Axis::Deep].into_iter().enumerate() {
        let (mut m0, mut m1) = axis_range(&m, world_axis);
        if world_axis == axis {
            m0 = m0.saturating_add(shift);
            m1 = m1.saturating_add(shift);
        }
        let (f0, f1) = axis_range(&f, world_axis);
        let nm = |value: i64| render::nm(Length::from_nm(value));
        let (a, b) = if m1 < f0 {
            (nm(m1), nm(f0))
        } else if f1 < m0 {
            (nm(m0), nm(f1))
        } else {
            let middle = (nm(m0.max(f0)) + nm(m1.min(f1))) / 2.0;
            (middle, middle)
        };
        on_moving[slot] = a;
        on_fixed[slot] = b;
    }
    let mut points = (on_moving, on_fixed);
    if let Some(prefer) = prefer {
        for (slot, world_axis) in [Axis::Across, Axis::Up, Axis::Deep].into_iter().enumerate() {
            if world_axis == axis {
                continue;
            }
            let (m0, m1) = axis_range(&m, world_axis);
            let (f0, f1) = axis_range(&f, world_axis);
            let nm = |value: i64| render::nm(Length::from_nm(value));
            slide_points(
                &mut points,
                slot,
                nm(m0.max(f0)),
                nm(m1.min(f1)),
                prefer[slot],
            );
        }
    }
    let (on_moving, on_fixed) = points;
    Some(Closest {
        fixed: index,
        gap: Length::from_nm(i64::try_from(isqrt(squared)).unwrap_or(i64::MAX)),
        at: render::nm(Length::from_nm(at)),
        points: Some((on_moving, on_fixed)),
    })
}

// ---------------------------------------------------------------------------
// Swings: sampled and refined
// ---------------------------------------------------------------------------

/// A box moved by a matrix: its eight corners.
fn corners(region: &WorldBox, matrix: &DMat4) -> [DVec3; 8] {
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
    .map(|p| matrix.transform_point3(p))
}

/// The twelve edges of a box by corner index (bits: x 1, y 2, z 4).
const EDGES: [(usize, usize); 12] = [
    (0, 1),
    (2, 3),
    (4, 5),
    (6, 7),
    (0, 2),
    (1, 3),
    (4, 6),
    (5, 7),
    (0, 4),
    (1, 5),
    (2, 6),
    (3, 7),
];

/// The distance between two segments.
fn segment_distance(p1: DVec3, q1: DVec3, p2: DVec3, q2: DVec3) -> f64 {
    let (a, b) = segment_points(p1, q1, p2, q2);
    a.distance(b)
}

/// The nearest points of two segments.
fn segment_points(p1: DVec3, q1: DVec3, p2: DVec3, q2: DVec3) -> (DVec3, DVec3) {
    let d1 = q1 - p1;
    let d2 = q2 - p2;
    let r = p1 - p2;
    let a = d1.dot(d1);
    let e = d2.dot(d2);
    let f = d2.dot(r);
    let (s, t) = if a <= 1e-9 && e <= 1e-9 {
        (0.0, 0.0)
    } else if a <= 1e-9 {
        (0.0, (f / e).clamp(0.0, 1.0))
    } else {
        let c = d1.dot(r);
        if e <= 1e-9 {
            ((-c / a).clamp(0.0, 1.0), 0.0)
        } else {
            let b = d1.dot(d2);
            let denominator = a * e - b * b;
            let mut s = if denominator > 1e-9 {
                ((b * f - c * e) / denominator).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let mut t = (b * s + f) / e;
            if t < 0.0 {
                t = 0.0;
                s = (-c / a).clamp(0.0, 1.0);
            } else if t > 1.0 {
                t = 1.0;
                s = ((b - c) / a).clamp(0.0, 1.0);
            }
            (s, t)
        }
    };
    (p1 + d1 * s, p2 + d2 * t)
}

/// A box as its corners and its frame: centre, unit axes, half sizes.
struct Placed {
    corners: [DVec3; 8],
    centre: DVec3,
    axes: [DVec3; 3],
    half: [f64; 3],
    low: DVec3,
    high: DVec3,
}

impl Placed {
    fn of(region: &WorldBox, matrix: &DMat4) -> Placed {
        let corners = corners(region, matrix);
        let (a, b) = (point_vector(region.min()), point_vector(region.max()));
        let size = b - a;
        let (low, high) = corners.iter().fold(
            (DVec3::splat(f64::INFINITY), DVec3::splat(f64::NEG_INFINITY)),
            |(lo, hi), c| (lo.min(*c), hi.max(*c)),
        );
        Placed {
            corners,
            centre: matrix.transform_point3((a + b) / 2.0),
            axes: [DVec3::X, DVec3::Y, DVec3::Z].map(|v| matrix.transform_vector3(v)),
            half: [size.x / 2.0, size.y / 2.0, size.z / 2.0],
            low,
            high,
        }
    }

    /// The distance from a point to the box.
    fn to_point(&self, point: DVec3) -> f64 {
        let offset = point - self.centre;
        (0..3)
            .map(|i| {
                let along = offset.dot(self.axes[i]);
                (along.abs() - self.half[i]).max(0.0).powi(2)
            })
            .sum::<f64>()
            .sqrt()
    }

    /// The gap between the boxes' bounds: never more than their distance.
    fn bounds_gap(&self, other: &Placed) -> f64 {
        let gap = (other.low - self.high)
            .max(self.low - other.high)
            .max(DVec3::ZERO);
        gap.length()
    }

    /// Do the boxes overlap (the separating axis test)?
    fn overlaps(&self, other: &Placed) -> bool {
        let reach = |placed: &Placed, axis: DVec3| {
            (0..3)
                .map(|i| placed.half[i] * placed.axes[i].dot(axis).abs())
                .sum::<f64>()
        };
        let between = other.centre - self.centre;
        let mut candidates: Vec<DVec3> = self.axes.iter().chain(&other.axes).copied().collect();
        for x in &self.axes {
            for y in &other.axes {
                let cross = x.cross(*y);
                if cross.length_squared() > 1e-12 {
                    candidates.push(cross.normalize());
                }
            }
        }
        candidates
            .iter()
            .all(|axis| reach(self, *axis) + reach(other, *axis) > between.dot(*axis).abs())
    }

    /// The nearest point of the box to a point.
    fn nearest_to(&self, point: DVec3) -> DVec3 {
        let offset = point - self.centre;
        (0..3).fold(self.centre, |at, i| {
            at + self.axes[i] * offset.dot(self.axes[i]).clamp(-self.half[i], self.half[i])
        })
    }

    /// The distance between the boxes and the nearest point on each (this
    /// one's first).
    fn nearest(&self, other: &Placed) -> (f64, DVec3, DVec3) {
        let mut best = (f64::INFINITY, self.centre, other.centre);
        let mut consider = |a: DVec3, b: DVec3| {
            let d = a.distance(b);
            if d < best.0 {
                best = (d, a, b);
            }
        };
        for corner in &self.corners {
            consider(*corner, other.nearest_to(*corner));
        }
        for corner in &other.corners {
            consider(self.nearest_to(*corner), *corner);
        }
        for (a, b) in EDGES {
            for (c, d) in EDGES {
                if let (Some(p1), Some(q1), Some(p2), Some(q2)) = (
                    self.corners.get(a),
                    self.corners.get(b),
                    other.corners.get(c),
                    other.corners.get(d),
                ) {
                    let (x, y) = segment_points(*p1, *q1, *p2, *q2);
                    consider(x, y);
                }
            }
        }
        best
    }

    /// The distance between the boxes (0 when they overlap).
    fn distance(&self, other: &Placed) -> f64 {
        if self.overlaps(other) {
            return 0.0;
        }
        let mut best = f64::INFINITY;
        for corner in &self.corners {
            best = best.min(other.to_point(*corner));
        }
        for corner in &other.corners {
            best = best.min(self.to_point(*corner));
        }
        for (a, b) in EDGES {
            for (c, d) in EDGES {
                if let (Some(p1), Some(q1), Some(p2), Some(q2)) = (
                    self.corners.get(a),
                    self.corners.get(b),
                    other.corners.get(c),
                    other.corners.get(d),
                ) {
                    best = best.min(segment_distance(*p1, *q1, *p2, *q2));
                }
            }
        }
        best
    }
}

/// Gaps wider than this (nanometres, 100mm) are not measured: a swing
/// reports the closest approach only when something comes this near.
pub const SWING_REACH: f64 = 100_000_000.0;

/// The gap from the moving boxes at a placement to each fixed solid, up
/// to [`SWING_REACH`] (anything farther counts as that far).
fn gaps(moving: &[WorldBox], fixed: &[Vec<Placed>], matrix: &DMat4) -> Vec<f64> {
    let placed: Vec<Placed> = moving.iter().map(|b| Placed::of(b, matrix)).collect();
    let (low, high) = placed.iter().fold(
        (DVec3::splat(f64::INFINITY), DVec3::splat(f64::NEG_INFINITY)),
        |(lo, hi), p| (lo.min(p.low), hi.max(p.high)),
    );
    fixed
        .iter()
        .map(|boxes| {
            let mut best = SWING_REACH;
            for f in boxes {
                let far = (f.low - high).max(low - f.high).max(DVec3::ZERO).length();
                if far >= best {
                    continue;
                }
                for m in &placed {
                    if m.bounds_gap(f) >= best {
                        continue;
                    }
                    best = best.min(m.distance(f));
                }
            }
            best
        })
        .collect()
}

fn swing(
    moving: &[WorldBox],
    fixed: &[Vec<WorldBox>],
    pose: &dyn Fn(f64) -> Option<DMat4>,
    range: f64,
    about: Option<Axis>,
    prefer: Option<DVec3>,
) -> Option<Closest> {
    let rest = moving
        .iter()
        .copied()
        .reduce(|a, b| crate::collide::union(&a, &b))?;
    let reference = corners(&rest, &DMat4::IDENTITY);
    // How far any point moves between two angles (the box's corners
    // bound every point inside it: a rigid move's displacement is affine).
    let moved = |a: &DMat4, b: &DMat4| {
        reference
            .iter()
            .map(|c| (a.transform_point3(*c) - b.transform_point3(*c)).length())
            .fold(0.0, f64::max)
    };
    // Coarse samples, no more than 4mm of movement apart.
    let probe = 256.0;
    let mut path = 0.0;
    let mut last = pose(0.0)?;
    let mut swept_low = DVec3::splat(f64::INFINITY);
    let mut swept_high = DVec3::splat(f64::NEG_INFINITY);
    for k in 0..=256_u32 {
        let angle = range * f64::from(k) / probe;
        let Some(matrix) = pose(angle) else { continue };
        path += moved(&last, &matrix);
        last = matrix;
        for c in corners(&rest, &matrix) {
            swept_low = swept_low.min(c);
            swept_high = swept_high.max(c);
        }
    }
    // Only fixed boxes near the swept volume matter.
    let margin = 50_000_000.0;
    let near = |b: &WorldBox| {
        let (low, high) = (point_vector(b.min()), point_vector(b.max()));
        (0..3).all(|i| low[i] <= swept_high[i] + margin && high[i] >= swept_low[i] - margin)
    };
    let fixed_placed: Vec<Vec<Placed>> = fixed
        .iter()
        .map(|boxes| {
            boxes
                .iter()
                .filter(|b| near(b))
                .map(|b| Placed::of(b, &DMat4::IDENTITY))
                .collect()
        })
        .collect();
    let count = (path / 4_000_000.0).ceil().clamp(16.0, 10_000.0);
    let samples: Vec<(f64, DMat4, Vec<f64>)> = (0..=integer(count))
        .filter_map(|k| {
            let angle = range * f64::from(k) / count;
            let matrix = pose(angle)?;
            let gaps = gaps(moving, &fixed_placed, &matrix);
            Some((angle, matrix, gaps))
        })
        .collect();
    let tolerance = render::nm(CONTACT_TOLERANCE);
    let mut best: Option<(f64, usize, f64)> = None;
    for index in 0..fixed_placed.len() {
        // The gap along the axis it turns about never changes: no gap to
        // this part is ever smaller than the smallest of those.
        let floor = about.map_or(0.0, |axis| {
            fixed
                .get(index)
                .into_iter()
                .flatten()
                .flat_map(|f| {
                    moving
                        .iter()
                        .map(move |m| range_gap(axis_range(m, axis), axis_range(f, axis)))
                })
                .min()
                .map_or(0.0, |gap| render::nm(Length::from_nm(gap)))
        });
        let one = |matrix: &DMat4| {
            gaps(
                moving,
                std::slice::from_ref(fixed_placed.get(index)?),
                matrix,
            )
            .first()
            .copied()
        };
        let value = |k: usize| {
            samples
                .get(k)
                .and_then(|(_, _, g)| g.get(index).copied())
                .unwrap_or(f64::INFINITY)
        };
        if value(0) >= SWING_REACH && (0..samples.len()).all(|k| value(k) >= SWING_REACH) {
            continue;
        }
        // Touching at rest: start once the gap stops growing.
        let start = if value(0) <= tolerance {
            match (1..samples.len()).find(|&k| value(k + 1) < value(k)) {
                Some(k) => k,
                None => continue,
            }
        } else {
            0
        };
        let mut part_best = (f64::INFINITY, 0.0);
        for k in start..samples.len() {
            if value(k) < part_best.0 {
                part_best = (value(k), samples.get(k).map_or(0.0, |s| s.0));
            }
        }
        // Refine every interval whose bound could beat the best.
        let mut stack: Vec<(f64, DMat4, f64, f64, DMat4, f64)> = Vec::new();
        for k in start..samples.len().saturating_sub(1) {
            if let (Some((a, ma, _)), Some((b, mb, _))) = (samples.get(k), samples.get(k + 1)) {
                stack.push((*a, *ma, value(k), *b, *mb, value(k + 1)));
            }
        }
        while let Some((a, ma, ga, b, mb, gb)) = stack.pop() {
            let step = moved(&ma, &mb);
            let bound = (ga.min(gb) - step / 2.0).max(floor);
            if bound >= part_best.0 - 1.0 || step <= SWING_STEP {
                continue;
            }
            let middle = (a + b) / 2.0;
            let Some(mm) = pose(middle) else { continue };
            let gm = one(&mm).unwrap_or(f64::INFINITY);
            if gm < part_best.0 {
                part_best = (gm, middle);
            }
            stack.push((a, ma, ga, middle, mm, gm));
            stack.push((middle, mm, gm, b, mb, gb));
        }
        if part_best.0 < SWING_REACH && best.is_none_or(|(b, _, _)| part_best.0 < b) {
            best = Some((part_best.0, index, part_best.1));
        }
    }
    let (gap, index, at) = best?;
    let points = pose(at).and_then(|matrix| {
        let placed: Vec<Placed> = moving.iter().map(|b| Placed::of(b, &matrix)).collect();
        let (_, mut points, m, f) = fixed_placed
            .get(index)?
            .iter()
            .flat_map(|f| {
                placed.iter().map(move |m| {
                    let (d, a, b) = m.nearest(f);
                    (d, (a, b), m, f)
                })
            })
            .min_by(|a, b| a.0.total_cmp(&b.0))?;
        // Along the axis it turns about the boxes keep their section, so
        // the nearest points may slide level with the hardware.
        if let (Some(axis), Some(prefer)) = (about, prefer) {
            let slot = match axis {
                Axis::Across => 0,
                Axis::Up => 1,
                Axis::Deep => 2,
            };
            slide_points(
                &mut points,
                slot,
                m.low[slot].max(f.low[slot]),
                m.high[slot].min(f.high[slot]),
                prefer[slot],
            );
        }
        Some(points)
    });
    Some(Closest {
        fixed: index,
        gap: render::length_from_nm(gap.floor())?,
        at,
        points,
    })
}

/// A whole, non-negative count of samples as a loop bound.
fn integer(count: f64) -> u32 {
    let whole = render::integer_from_f64(count.floor()).unwrap_or(0);
    u32::try_from(whole.max(0)).unwrap_or(u32::MAX)
}

/// The smallest gap between an assembly's solids (`moving`, at rest) and
/// the `fixed` ones over its whole motion: exact for a slide, sampled for a
/// hinge's turn or a cup hinge's linkage. The nearest points are placed as
/// near `prefer` (world nanometres) as the gap allows; see
/// [`closest_near`].
pub fn closest_for(
    moving: &[WorldSolid],
    fixed: &[WorldSolid],
    motion: &Motion,
    prefer: Option<DVec3>,
) -> Option<Closest> {
    match *motion {
        Motion::Travel { toward, travel } => {
            closest_near(moving, fixed, &Travel::Slide { toward, travel }, prefer)
        }
        Motion::Turn { .. } | Motion::Linkage { .. } => {
            let range = render::radians_f64(motion.opening_angle()?);
            let pose = |radians: f64| {
                let angle = render::angle_from_radians_floor(radians)?;
                Some(Placement::of_motions(&[(*motion, PoseValue::Angle(angle))]).matrix())
            };
            let about = match *motion {
                Motion::Turn { pivot, .. } => Some(pivot.axis),
                Motion::Linkage { linkage } => Some(linkage.frame().axis()),
                Motion::Travel { .. } => None,
            };
            closest_near(
                moving,
                fixed,
                &Travel::Swing {
                    pose: &pose,
                    range,
                    about,
                },
                prefer,
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use stave_model::solved::Point;

    fn mm(value: i64) -> Length {
        Length::from_mm(value)
    }

    fn block(a: (i64, i64, i64), b: (i64, i64, i64)) -> WorldSolid {
        let point = |(x, y, z): (i64, i64, i64)| Point {
            across: mm(x),
            up: mm(y),
            deep: mm(z),
        };
        WorldSolid::from_boxes(vec![WorldBox::new(point(a), point(b))])
    }

    #[test]
    fn a_slide_past_a_box_is_exact() {
        // A drawer 10mm left of a post, sliding forward past it: the gap
        // stays 10mm; one 30mm in front of it, sliding back toward it
        // by 25mm, comes within 5mm.
        let drawer = block((0, 0, 0), (100, 100, 100));
        let post = block((110, 0, 200), (120, 100, 300));
        let closest = closest(
            std::slice::from_ref(&drawer),
            std::slice::from_ref(&post),
            &Travel::Slide {
                toward: Direction::Back,
                travel: mm(400),
            },
        )
        .unwrap();
        assert_eq!(closest.gap, mm(10));
        let wall = block((0, 0, 130), (100, 100, 140));
        let closest = closest_of(&drawer, &wall, Direction::Back, mm(25));
        assert_eq!(closest.gap, mm(5));
        assert_eq!(closest.at, 25_000_000.0);
        // A pair touching at rest doesn't count.
        let touching = block((100, 0, 0), (110, 100, 100));
        assert!(
            super::closest(
                std::slice::from_ref(&drawer),
                std::slice::from_ref(&touching),
                &Travel::Slide {
                    toward: Direction::Back,
                    travel: mm(25)
                }
            )
            .is_none()
        );
    }

    fn closest_of(
        moving: &WorldSolid,
        fixed: &WorldSolid,
        toward: Direction,
        travel: Length,
    ) -> Closest {
        closest(
            std::slice::from_ref(moving),
            std::slice::from_ref(fixed),
            &Travel::Slide { toward, travel },
        )
        .unwrap()
    }

    #[test]
    fn a_swing_finds_the_closest_corner_to_within_the_bound() {
        // A 100mm door turning about the up axis through its left end,
        // past a post: its far corner sweeps a 100mm (and a bit) circle.
        // The post's near corner is 150mm from the pivot, so the door's
        // corner comes within 150 - |corner| of it.
        let door = block((0, 0, 0), (100, 10, 10));
        let post = block((0, 0, -160), (10, 10, -150));
        let pose = |angle: f64| Some(DMat4::from_axis_angle(DVec3::Y, angle));
        let found = closest(
            std::slice::from_ref(&door),
            std::slice::from_ref(&post),
            &Travel::Swing {
                pose: &pose,
                range: std::f64::consts::FRAC_PI_2,
                about: Some(Axis::Up),
            },
        )
        .unwrap();
        // The door's far corner (100, *, 10) is sqrt(100² + 10²) from the
        // pivot; at 90° the door lies along -deep... the post's corner at
        // (10, *, -150): the true gap is found by dense sampling.
        let mut truth = f64::INFINITY;
        let door_boxes = boxes(&door);
        let post_placed: Vec<Placed> = boxes(&post)
            .iter()
            .map(|b| Placed::of(b, &DMat4::IDENTITY))
            .collect();
        for k in 0..=20_000 {
            let angle = std::f64::consts::FRAC_PI_2 * f64::from(k) / 20_000.0;
            let g = gaps(
                &door_boxes,
                std::slice::from_ref(&post_placed),
                &pose(angle).unwrap(),
            );
            truth = truth.min(g[0]);
        }
        let reported = render::nm(found.gap);
        assert!(reported >= truth - 1.0, "{reported} < {truth}");
        assert!(
            reported - truth <= SWING_STEP / 2.0 + 1.0,
            "{reported} vs {truth}"
        );
    }

    #[test]
    fn box_distance_is_exact() {
        let a = Placed::of(
            &WorldBox::new(
                Point::default(),
                Point {
                    across: mm(10),
                    up: mm(10),
                    deep: mm(10),
                },
            ),
            &DMat4::IDENTITY,
        );
        // A box turned 45° about up, its edge 5mm off a's face.
        let turned = DMat4::from_translation(DVec3::new(15e6 + 5e6 * 2f64.sqrt(), 5e6, 5e6))
            * DMat4::from_axis_angle(DVec3::Y, std::f64::consts::FRAC_PI_4)
            * DMat4::from_translation(DVec3::new(-5e6, -5e6, -5e6));
        let b = Placed::of(
            &WorldBox::new(
                Point::default(),
                Point {
                    across: mm(10),
                    up: mm(10),
                    deep: mm(10),
                },
            ),
            &turned,
        );
        assert!((a.distance(&b) - 5e6).abs() < 1.0, "{}", a.distance(&b));
        assert_eq!(a.distance(&a), 0.0);
    }
}
