//! Convex pieces of wood less the angled cuts, for collision tests and
//! assembly sequences.
//!
//! An angled cut placed in the world is a [`WorldCut`]: an exact convex
//! polygon on the plane across a world axis, over a range of that axis (a
//! part is only ever turned by quarter turns at rest, so a part axis is a
//! world axis). What is left of a box of wood once the cuts are taken out
//! is a set of convex [`Piece`]s: a cut along the piece's axis splits its
//! range at the cut's ends, and within the cut's range the polygon less
//! the cut's polygon is one convex piece per side of the cut (outside that
//! side, inside the sides before it).
//!
//! The pieces are in `f64` nanometres: their corners where a cut's side
//! crosses a box's are rarely whole nanometres. Every input is exact, and
//! every decision compares a depth with the 0.05mm contact tolerance, so
//! the rounding (well under a nanometre for a design's sizes) never
//! changes one. A cut along another axis than a piece that is no longer a
//! box is left out of that piece, which only ever finds more wood than
//! there is; no example cuts one part along two axes at an angle.

use glam::DVec3;
use stave_lang::placement::Axis;
use stave_lang::units::Length;
use stave_model::solved::{Point, WorldBox};

use crate::render;

/// An angled cut's convex piece, placed: exact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorldCut {
    /// The world axis it is cut along.
    pub axis: Axis,
    /// Its corners on the two other axes, in [`Axis::others`] order.
    pub corners: Vec<[Length; 2]>,
    /// Its low end along `axis`.
    pub start: Length,
    /// Its high end along `axis`.
    pub end: Length,
}

impl WorldCut {
    /// The same cut moved by an exact offset.
    pub fn translated(&self, by: Point) -> WorldCut {
        let [first, second] = self.axis.others();
        let shift = |value: Length, axis: Axis| {
            Length::from_nm(value.nm().saturating_add(by.get(axis).nm()))
        };
        WorldCut {
            axis: self.axis,
            corners: self
                .corners
                .iter()
                .map(|[a, b]| [shift(*a, first), shift(*b, second)])
                .collect(),
            start: shift(self.start, self.axis),
            end: shift(self.end, self.axis),
        }
    }

    /// Does its box share a volume with `region`?
    pub(crate) fn meets(&self, region: &WorldBox) -> bool {
        let [first, second] = self.axis.others();
        let apart = |low: Length, high: Length, axis: Axis| {
            high <= region.min().get(axis) || low >= region.max().get(axis)
        };
        if apart(self.start, self.end, self.axis) {
            return false;
        }
        for (index, axis) in [first, second].into_iter().enumerate() {
            let values = self.corners.iter().filter_map(|c| c.get(index).copied());
            let (low, high) = values.fold((None::<Length>, None::<Length>), |(lo, hi), v| {
                (
                    Some(lo.map_or(v, |l| l.min(v))),
                    Some(hi.map_or(v, |h| h.max(v))),
                )
            });
            if let (Some(low), Some(high)) = (low, high)
                && apart(low, high, axis)
            {
                return false;
            }
        }
        true
    }

    /// The cut as a piece (floats).
    pub(crate) fn piece(&self) -> Piece {
        let corners = self
            .corners
            .iter()
            .map(|[a, b]| [render::nm(*a), render::nm(*b)])
            .collect();
        Piece::new(
            self.axis,
            corners,
            (render::nm(self.start), render::nm(self.end)),
            None,
        )
    }
}

/// A convex polygon on the plane across a world axis (its corners on the
/// two other axes, in [`Axis::others`] order, counter-clockwise), over a
/// range of the axis; nanometres.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Piece {
    axis: Axis,
    corners: Vec<[f64; 2]>,
    range: (f64, f64),
    /// The box it still is the whole of, if any (so it can be seen along
    /// any axis).
    whole: Option<WorldBox>,
}

fn cross(o: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
    (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0])
}

fn signed_area(corners: &[[f64; 2]]) -> f64 {
    let mut sum = 0.0;
    for (index, p) in corners.iter().enumerate() {
        if let Some(q) = corners.get((index + 1) % corners.len()) {
            sum += p[0] * q[1] - q[0] * p[1];
        }
    }
    sum / 2.0
}

/// Sutherland–Hodgman against `side(p) <= 0`.
fn clip(polygon: &[[f64; 2]], side: impl Fn([f64; 2]) -> f64) -> Vec<[f64; 2]> {
    let mut out = Vec::with_capacity(polygon.len() + 2);
    let count = polygon.len();
    for index in 0..count {
        let (Some(&current), Some(&next)) = (polygon.get(index), polygon.get((index + 1) % count))
        else {
            continue;
        };
        let (a, b) = (side(current), side(next));
        if a <= 0.0 {
            out.push(current);
        }
        if (a < 0.0 && b > 0.0) || (a > 0.0 && b < 0.0) {
            let t = a / (a - b);
            out.push([
                current[0] + (next[0] - current[0]) * t,
                current[1] + (next[1] - current[1]) * t,
            ]);
        }
    }
    out
}

fn unit(axis: Axis) -> DVec3 {
    match axis {
        Axis::Across => DVec3::X,
        Axis::Up => DVec3::Y,
        Axis::Deep => DVec3::Z,
    }
}

impl Piece {
    fn new(
        axis: Axis,
        mut corners: Vec<[f64; 2]>,
        range: (f64, f64),
        whole: Option<WorldBox>,
    ) -> Piece {
        if signed_area(&corners) < 0.0 {
            corners.reverse();
        }
        Piece {
            axis,
            corners,
            range,
            whole,
        }
    }

    /// A box as a piece along `axis`.
    pub(crate) fn of_box(region: &WorldBox, axis: Axis) -> Piece {
        let [first, second] = axis.others();
        let low = |a: Axis| render::nm(region.min().get(a));
        let high = |a: Axis| render::nm(region.max().get(a));
        Piece::new(
            axis,
            vec![
                [low(first), low(second)],
                [high(first), low(second)],
                [high(first), high(second)],
                [low(first), high(second)],
            ],
            (low(axis), high(axis)),
            Some(*region),
        )
    }

    /// The piece seen along `axis`: itself, or (a whole box) the same box
    /// along another axis.
    fn along(&self, axis: Axis) -> Option<Piece> {
        if self.axis == axis {
            return Some(self.clone());
        }
        self.whole.map(|whole| Piece::of_box(&whole, axis))
    }

    /// The corners in 3D.
    fn vertices(&self) -> Vec<DVec3> {
        let [first, second] = self.axis.others();
        let mut out = Vec::with_capacity(self.corners.len() * 2);
        for along in [self.range.0, self.range.1] {
            for c in &self.corners {
                out.push(unit(self.axis) * along + unit(first) * c[0] + unit(second) * c[1]);
            }
        }
        out
    }

    /// The box around it (floats): (low, high).
    fn extent(&self) -> (DVec3, DVec3) {
        let vertices = self.vertices();
        let low = vertices
            .iter()
            .copied()
            .reduce(DVec3::min)
            .unwrap_or(DVec3::ZERO);
        let high = vertices
            .iter()
            .copied()
            .reduce(DVec3::max)
            .unwrap_or(DVec3::ZERO);
        (low, high)
    }

    /// The box around it, rounded outward to the nanometre.
    pub(crate) fn bounds(&self) -> Option<WorldBox> {
        let (low, high) = self.extent();
        let corner = |v: DVec3, round: fn(f64) -> f64| {
            Some(Point {
                across: render::length_from_nm(round(v.x))?,
                up: render::length_from_nm(round(v.y))?,
                deep: render::length_from_nm(round(v.z))?,
            })
        };
        Some(WorldBox::new(
            corner(low, f64::floor)?,
            corner(high, f64::ceil)?,
        ))
    }

    /// The piece less a cut: pieces that never overlap each other.
    fn minus(&self, cut: &Piece) -> Vec<Piece> {
        let Some(piece) = self.along(cut.axis) else {
            return vec![self.clone()];
        };
        let low = piece.range.0.max(cut.range.0);
        let high = piece.range.1.min(cut.range.1);
        if high <= low || !polygons_overlap(&piece.corners, &cut.corners) {
            return vec![self.clone()];
        }
        let mut out = Vec::new();
        // The part of a box beyond a cut's ends is a box (the ends are
        // whole nanometres).
        let slab = |from: f64, to: f64| {
            let whole = piece.whole.and_then(|whole| {
                let (from, to) = (render::length_from_nm(from)?, render::length_from_nm(to)?);
                Some(WorldBox::new(
                    whole.min().with(piece.axis, from),
                    whole.max().with(piece.axis, to),
                ))
            });
            Piece {
                range: (from, to),
                whole,
                ..piece.clone()
            }
        };
        if piece.range.0 < low {
            out.push(slab(piece.range.0, low));
        }
        if high < piece.range.1 {
            out.push(slab(high, piece.range.1));
        }
        let count = cut.corners.len();
        for j in 0..count {
            let (Some(&a), Some(&b)) = (cut.corners.get(j), cut.corners.get((j + 1) % count))
            else {
                continue;
            };
            // Outside side j...
            let mut polygon = clip(&piece.corners, |p| cross(a, b, p));
            // ...and inside the sides before it.
            for k in 0..j {
                if polygon.len() < 3 {
                    break;
                }
                let (Some(&c), Some(&d)) = (cut.corners.get(k), cut.corners.get((k + 1) % count))
                else {
                    continue;
                };
                polygon = clip(&polygon, |p| -cross(c, d, p));
            }
            if polygon.len() >= 3 && signed_area(&polygon) > 0.0 {
                out.push(Piece {
                    axis: piece.axis,
                    corners: polygon,
                    range: (low, high),
                    whole: None,
                });
            }
        }
        out
    }

    /// How far it would have to move to be gone: the smaller of its
    /// length along its axis and its narrowest width across, with the
    /// world axis nearest that direction.
    pub(crate) fn depth(&self) -> (f64, Axis) {
        let [first, second] = self.axis.others();
        let mut best = (self.range.1 - self.range.0, self.axis);
        let count = self.corners.len();
        for j in 0..count {
            let (Some(&a), Some(&b)) = (self.corners.get(j), self.corners.get((j + 1) % count))
            else {
                continue;
            };
            let length = ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt();
            if length <= 0.0 {
                continue;
            }
            let width = self
                .corners
                .iter()
                .map(|p| cross(a, b, *p) / length)
                .fold(0.0, f64::max);
            if width < best.0 {
                // The side's normal, across it.
                let normal = [(b[1] - a[1]).abs(), (b[0] - a[0]).abs()];
                let axis = if normal[0] >= normal[1] {
                    first
                } else {
                    second
                };
                best = (width, axis);
            }
        }
        best
    }

    /// The directions a separating plane can face, and the directions of
    /// the edges.
    fn directions(&self) -> (Vec<DVec3>, Vec<DVec3>) {
        let [first, second] = self.axis.others();
        let along = unit(self.axis);
        let mut normals = vec![along];
        let mut edges = vec![along];
        let count = self.corners.len();
        for j in 0..count {
            let (Some(&a), Some(&b)) = (self.corners.get(j), self.corners.get((j + 1) % count))
            else {
                continue;
            };
            let run = unit(first) * (b[0] - a[0]) + unit(second) * (b[1] - a[1]);
            if run.length_squared() <= 0.0 {
                continue;
            }
            edges.push(run.normalize());
            normals.push(run.cross(along).normalize());
        }
        (normals, edges)
    }
}

/// Do two counter-clockwise convex polygons share an area (more than
/// touching)?
fn polygons_overlap(a: &[[f64; 2]], b: &[[f64; 2]]) -> bool {
    for (polygon, other) in [(a, b), (b, a)] {
        let count = polygon.len();
        for j in 0..count {
            let (Some(&p), Some(&q)) = (polygon.get(j), polygon.get((j + 1) % count)) else {
                continue;
            };
            // Every point of `other` on or outside this side: apart.
            if other.iter().all(|r| cross(p, q, *r) <= 0.0) {
                return false;
            }
        }
    }
    true
}

/// A box of wood less the cuts that reach it.
pub(crate) fn residual(region: &WorldBox, cuts: &[Piece]) -> Vec<Piece> {
    let mut pieces = vec![Piece::of_box(region, Axis::Up)];
    for cut in cuts {
        pieces = pieces.iter().flat_map(|p| p.minus(cut)).collect();
        if pieces.is_empty() {
            break;
        }
    }
    pieces
}

/// The range of moves `t` along a unit `direction` over which `mover`,
/// moved by `t`, goes deeper than `tolerance` into `other`: the separating
/// axis test over the faces of both and the crosses of their edges. `None`
/// when it never does.
pub(crate) fn push_interval(
    mover: &Piece,
    other: &Piece,
    direction: DVec3,
    tolerance: f64,
) -> Option<(f64, f64)> {
    let (mut normals, mover_edges) = mover.directions();
    let (other_normals, other_edges) = other.directions();
    normals.extend(other_normals);
    for x in &mover_edges {
        for y in &other_edges {
            let c = x.cross(*y);
            if c.length_squared() > 1e-12 {
                normals.push(c.normalize());
            }
        }
    }
    let (mv, ov) = (mover.vertices(), other.vertices());
    let span = |vertices: &[DVec3], n: DVec3| {
        vertices
            .iter()
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), v| {
                let d = v.dot(n);
                (lo.min(d), hi.max(d))
            })
    };
    let mut low = f64::NEG_INFINITY;
    let mut high = f64::INFINITY;
    for n in normals {
        let (p0, p1) = span(&mv, n);
        let (q0, q1) = span(&ov, n);
        let s = direction.dot(n);
        if s.abs() < 1e-12 {
            // Moving doesn't change this shadow.
            if p1.min(q1) - p0.max(q0) <= tolerance {
                return None;
            }
            continue;
        }
        // p1 + t s - q0 > tol and q1 - p0 - t s > tol.
        let (a, b) = ((q0 + tolerance - p1) / s, (q1 - tolerance - p0) / s);
        low = low.max(a.min(b));
        high = high.min(a.max(b));
        if high <= low {
            return None;
        }
    }
    Some((low, high))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mm(value: i64) -> Length {
        Length::from_mm(value)
    }

    fn region(a: [i64; 3], b: [i64; 3]) -> WorldBox {
        WorldBox::new(
            Point {
                across: mm(a[0]),
                up: mm(a[1]),
                deep: mm(a[2]),
            },
            Point {
                across: mm(b[0]),
                up: mm(b[1]),
                deep: mm(b[2]),
            },
        )
    }

    #[test]
    fn a_box_less_a_triangle_keeps_the_rest() {
        // A 10mm cube less the triangle on its across/deep half, through
        // its height.
        let cut = WorldCut {
            axis: Axis::Up,
            corners: vec![[mm(0), mm(0)], [mm(10), mm(0)], [mm(0), mm(10)]],
            start: mm(0),
            end: mm(10),
        };
        let left = residual(&region([0, 0, 0], [10, 10, 10]), &[cut.piece()]);
        let area: f64 = left.iter().map(|p| signed_area(&p.corners)).sum();
        assert!((area - 50e12).abs() < 1.0, "{area}");
        // The triangle itself has nothing left once the cube is cut away
        // from it.
        let cube = WorldCut {
            axis: Axis::Up,
            corners: vec![
                [mm(0), mm(0)],
                [mm(10), mm(0)],
                [mm(10), mm(10)],
                [mm(0), mm(10)],
            ],
            start: mm(0),
            end: mm(10),
        };
        let none = residual(&region([0, 0, 0], [10, 10, 10]), &[cube.piece()]);
        assert!(none.iter().all(|p| p.depth().0 < 1.0), "{none:?}");
    }

    #[test]
    fn a_push_meets_what_is_ahead_only() {
        let mover = Piece::of_box(&region([0, 0, 0], [10, 10, 10]), Axis::Up);
        let ahead = Piece::of_box(&region([20, 0, 0], [30, 10, 10]), Axis::Up);
        let (low, high) = push_interval(&mover, &ahead, DVec3::X, 50_000.0).unwrap();
        assert!((low - 10_050_000.0).abs() < 1.0, "{low}");
        assert!((high - 29_950_000.0).abs() < 1.0, "{high}");
        // Along the other axis it never meets it.
        assert!(push_interval(&mover, &ahead, DVec3::Y, 50_000.0).is_none());
    }
}
