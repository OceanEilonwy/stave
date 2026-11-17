//! Points in and on a part's cut solid, in floats: what an assembly step
//! that turns a part, or moves it on a diagonal, is tested with.
//!
//! [`Shape::contains`] says whether a point lies inside the solid deeper
//! than a margin (the contact tolerance), so surfaces that touch don't
//! count. The solid is the blocks and rods of wood (round sections with
//! their flats), less the holes and the slanted pieces of the angled
//! cuts ([`Prism`]). The margin is
//! applied by testing the point and the 26 points `margin` away along the
//! part's axes and their diagonals: a point deeper than the margin passes
//! them all, a point on a surface, an edge or a corner fails at least one.
//!
//! [`Shape::surface`] samples the solid's boundary at a spacing, near a
//! region: two solids share wood beyond the tolerance only if a boundary
//! point of one lies inside the other, up to the spacing.
//!
//! Coordinates are the part's own frame (`x` length, `y` width, `z`
//! thickness) in `f64` nanometres; [`Shape::matrix`] maps them to world
//! nanometres (across, up, deep).

use std::f64::consts::TAU;

use glam::{DMat4, DVec3};
use stave_lang::placement::PartAxis;
use stave_model::solved::Part;

use crate::collide::direction_vector;
use crate::render;
use crate::solid::{Point2, Prism, Rod, Solid, across};

/// Lengths below this (nanometres) count as on a surface.
const NUDGE: f64 = 1_000.0;
/// A point this close to a side of an angled cut (nanometres) is on it.
const ON_SIDE: f64 = 1.0;

/// The 26 unit directions to the neighbours of a cube's centre: along the
/// axes, the face diagonals and the corner diagonals.
const OFFSETS: [DVec3; 26] = {
    let mut out = [DVec3::ZERO; 26];
    let mut n = 0;
    let mut x = -1.0;
    while x <= 1.0 {
        let mut y = -1.0;
        while y <= 1.0 {
            let mut z = -1.0;
            while z <= 1.0 {
                let length2 = x * x + y * y + z * z;
                if length2 > 0.0 && n < 26 {
                    let scale = if length2 == 1.0 {
                        1.0
                    } else if length2 == 2.0 {
                        std::f64::consts::FRAC_1_SQRT_2
                    } else {
                        0.577_350_269_189_625_8
                    };
                    out[n] = DVec3::new(x * scale, y * scale, z * scale);
                    n += 1;
                }
                z += 1.0;
            }
            y += 1.0;
        }
        x += 1.0;
    }
    out
};

fn index(axis: PartAxis) -> usize {
    match axis {
        PartAxis::Length => 0,
        PartAxis::Width => 1,
        PartAxis::Thickness => 2,
    }
}

fn unit(axis: PartAxis) -> DVec3 {
    let mut v = DVec3::ZERO;
    v[index(axis)] = 1.0;
    v
}

/// A part's solid, ready for float tests.
pub struct Shape<'a> {
    solid: &'a Solid,
    rods: Vec<Rod>,
    size: DVec3,
    matrix: DMat4,
}

impl<'a> Shape<'a> {
    /// The shape of a part with its cuts made.
    pub fn new(part: &Part, solid: &'a Solid) -> Shape<'a> {
        let centre = [
            stave_lang::units::Length::from_nm(part.cut.width.nm().div_euclid(2)),
            stave_lang::units::Length::from_nm(part.cut.thickness.nm().div_euclid(2)),
        ];
        let o = part.pose.orientation;
        let matrix = DMat4::from_cols(
            direction_vector(o.length()).extend(0.0),
            direction_vector(o.width()).extend(0.0),
            direction_vector(o.thickness()).extend(0.0),
            crate::collide::point_vector(part.pose.origin).extend(1.0),
        );
        Shape {
            solid,
            rods: solid.all_rods(centre),
            size: DVec3::new(
                render::nm(part.cut.length),
                render::nm(part.cut.width),
                render::nm(part.cut.thickness),
            ),
            matrix,
        }
    }

    /// Local nanometres to world nanometres.
    pub fn matrix(&self) -> DMat4 {
        self.matrix
    }

    /// Is a local point in wood (no margin)?
    fn raw(&self, p: DVec3) -> bool {
        // Outside the cut box: no wood, without looking at the pieces.
        if (0..3).any(|i| p[i] < 0.0 || p[i] > self.size[i]) {
            return false;
        }
        let wood = self.solid.blocks().iter().any(|b| {
            PartAxis::ALL.into_iter().all(|axis| {
                let v = p[index(axis)];
                v >= render::nm(b.min().get(axis)) && v <= render::nm(b.max().get(axis))
            })
        }) || self.rods.iter().any(|rod| in_rod(rod, p, true));
        wood && !self.solid.holes().iter().any(|hole| in_rod(hole, p, false))
            && !self.solid.prisms().iter().any(|prism| in_prism(prism, p))
    }

    /// Is a local point inside the solid deeper than `margin`?
    pub fn contains(&self, p: DVec3, margin: f64) -> bool {
        self.raw(p) && OFFSETS.iter().all(|e| self.raw(p + *e * margin))
    }

    /// Is a local point on the boundary, its outside along `normal`?
    fn on_boundary(&self, p: DVec3, normal: DVec3) -> bool {
        self.raw(p - normal * NUDGE) && !self.raw(p + normal * NUDGE)
    }

    /// Points on the solid's boundary, about `spacing` apart, inside the
    /// local box `low..high`.
    pub fn surface(&self, spacing: f64, low: DVec3, high: DVec3) -> Vec<DVec3> {
        let mut out = Vec::new();
        let inside = |p: DVec3| (0..3).all(|i| p[i] >= low[i] && p[i] <= high[i]);
        let keep = |p: DVec3, normal: DVec3, out: &mut Vec<DVec3>| {
            if inside(p) && self.on_boundary(p, normal) {
                out.push(p);
            }
        };
        // Faces of blocks.
        for block in self.solid.blocks().iter() {
            let lo = DVec3::new(
                render::nm(block.min().length),
                render::nm(block.min().width),
                render::nm(block.min().thickness),
            );
            let hi = DVec3::new(
                render::nm(block.max().length),
                render::nm(block.max().width),
                render::nm(block.max().thickness),
            );
            for face in 0..3 {
                let (u, v) = ((face + 1) % 3, (face + 2) % 3);
                // Only the part of the face inside the region.
                let (u0, u1) = (lo[u].max(low[u]), hi[u].min(high[u]));
                let (v0, v1) = (lo[v].max(low[v]), hi[v].min(high[v]));
                if u0 > u1 || v0 > v1 {
                    continue;
                }
                for (plane, sign) in [(lo[face], -1.0), (hi[face], 1.0)] {
                    let mut normal = DVec3::ZERO;
                    normal[face] = sign;
                    for a in steps(u0, u1, spacing) {
                        for b in steps(v0, v1, spacing) {
                            let mut p = DVec3::ZERO;
                            p[face] = plane;
                            p[u] = a;
                            p[v] = b;
                            keep(p, normal, &mut out);
                        }
                    }
                }
            }
        }
        // Round wood, and holes (their walls face the hole's axis).
        for (rod, outward) in self
            .rods
            .iter()
            .map(|r| (r, 1.0))
            .chain(self.solid.holes().iter().map(|r| (r, -1.0)))
        {
            let axis = index(rod.axis);
            let [a, b] = across(rod.axis).map(index);
            let centre = (render::nm(rod.centre[0]), render::nm(rod.centre[1]));
            let radius = render::nm(rod.diameter) / 2.0;
            let around = ((TAU * radius / spacing).ceil()).max(8.0);
            let flats = rod
                .flats
                .filter(|_| outward > 0.0)
                .map(|f| (render::nm(f.across) / 2.0, render::radians_f64(f.angle)));
            let (s0, s1) = (
                render::nm(rod.start).max(low[axis]),
                render::nm(rod.end).min(high[axis]),
            );
            for along in steps(s0, s1, spacing) {
                let mut k = 0.0;
                while k < around {
                    let t = TAU * k / around;
                    k += 1.0;
                    let (mut dx, mut dy) = (t.cos() * radius, t.sin() * radius);
                    let mut normal = (t.cos(), t.sin());
                    if let Some((half, angle)) = flats {
                        let n = (angle.cos(), angle.sin());
                        let reach = dx * n.0 + dy * n.1;
                        if reach.abs() > half {
                            let excess = reach - half * reach.signum();
                            dx -= excess * n.0;
                            dy -= excess * n.1;
                            normal = (n.0 * reach.signum(), n.1 * reach.signum());
                        }
                    }
                    let mut p = DVec3::ZERO;
                    p[axis] = along;
                    p[a] = centre.0 + dx;
                    p[b] = centre.1 + dy;
                    let mut n = DVec3::ZERO;
                    n[a] = normal.0 * outward;
                    n[b] = normal.1 * outward;
                    keep(p, n, &mut out);
                }
            }
            // End faces of wood rods: a grid over the disc.
            if outward > 0.0 {
                for (plane, sign) in [(render::nm(rod.start), -1.0), (render::nm(rod.end), 1.0)] {
                    for x in steps(-radius, radius, spacing) {
                        for y in steps(-radius, radius, spacing) {
                            if x * x + y * y > radius * radius {
                                continue;
                            }
                            let mut p = DVec3::ZERO;
                            p[axis] = plane;
                            p[a] = centre.0 + x;
                            p[b] = centre.1 + y;
                            keep(p, unit(rod.axis) * sign, &mut out);
                        }
                    }
                }
            }
        }
        // Walls and floors of the angled cuts: every side of each outline
        // over its depth, and the slanted pieces at each end of the depth.
        for prism in self.solid.prisms() {
            let face = index(prism.face_axis());
            let [a, b] = across(prism.face_axis()).map(index);
            let (d0, d1) = (
                render::nm(prism.depth().from()),
                render::nm(prism.depth().to()),
            );
            let at = |point: Point2, depth: f64| {
                let mut p = DVec3::ZERO;
                p[a] = render::nm(point.first);
                p[b] = render::nm(point.second);
                p[face] = depth;
                p
            };
            let points = prism.outline().points();
            for (i, from) in points.iter().enumerate() {
                let Some(to) = points.get((i + 1) % points.len()) else {
                    continue;
                };
                let (start, end) = (at(*from, 0.0), at(*to, 0.0));
                let run = end - start;
                let length = run.length();
                if length <= 0.0 {
                    continue;
                }
                // Outward from the wood is into the cut: to the left of a
                // counter-clockwise side.
                let mut normal = DVec3::ZERO;
                normal[a] = -run[b] / length;
                normal[b] = run[a] / length;
                for along in steps(0.0, length, spacing) {
                    for depth in steps(d0, d1, spacing) {
                        let mut p = start + run * (along / length);
                        p[face] = depth;
                        keep(p, normal, &mut out);
                    }
                }
            }
            {
                let (low, high) = outline_bounds(points);
                for (plane, sign) in [(d0, 1.0), (d1, -1.0)] {
                    let mut normal = DVec3::ZERO;
                    normal[face] = sign;
                    for x in steps(render::nm(low.first), render::nm(high.first), spacing) {
                        for y in steps(render::nm(low.second), render::nm(high.second), spacing) {
                            let mut p = DVec3::ZERO;
                            p[a] = x;
                            p[b] = y;
                            p[face] = plane;
                            if outline_holds(points, x, y) {
                                keep(p, normal, &mut out);
                            }
                        }
                    }
                }
            }
        }
        out
    }
}

/// Evenly spaced values from `a` to `b` inclusive, at most `spacing`
/// apart.
fn steps(a: f64, b: f64, spacing: f64) -> impl Iterator<Item = f64> {
    let span = (b - a).max(0.0);
    let count = (span / spacing).ceil().max(1.0);
    let mut k = 0.0;
    std::iter::from_fn(move || {
        if k > count {
            return None;
        }
        let value = a + span * k / count;
        k += 1.0;
        Some(value)
    })
}

/// Is a local point in a rod (with its flats, for wood) or a hole?
fn in_rod(rod: &Rod, p: DVec3, wood: bool) -> bool {
    let along = p[index(rod.axis)];
    if along < render::nm(rod.start) || along > render::nm(rod.end) {
        return false;
    }
    let [a, b] = across(rod.axis).map(index);
    let dx = p[a] - render::nm(rod.centre[0]);
    let dy = p[b] - render::nm(rod.centre[1]);
    let radius = render::nm(rod.diameter) / 2.0;
    if wood {
        let mut inside = dx * dx + dy * dy <= radius * radius;
        if let Some(flats) = rod.flats {
            let angle = render::radians_f64(flats.angle);
            inside &= (dx * angle.cos() + dy * angle.sin()).abs() <= render::nm(flats.across) / 2.0;
        }
        inside
    } else {
        dx * dx + dy * dy < radius * radius
    }
}

/// The box around an outline's points: (low, high).
fn outline_bounds(points: &[Point2]) -> (Point2, Point2) {
    let mut low = points.first().copied().unwrap_or(Point2::new(
        stave_lang::units::Length::ZERO,
        stave_lang::units::Length::ZERO,
    ));
    let mut high = low;
    for p in points {
        low = Point2::new(low.first.min(p.first), low.second.min(p.second));
        high = Point2::new(high.first.max(p.first), high.second.max(p.second));
    }
    (low, high)
}

/// Is a point (two coordinates on the outline's plane) strictly inside an
/// outline, not on one of its sides? Even-odd crossings of a ray along
/// the first axis.
fn outline_holds(points: &[Point2], x: f64, y: f64) -> bool {
    let count = points.len();
    let mut inside = false;
    for i in 0..count {
        let (Some(p), Some(q)) = (points.get(i), points.get((i + 1) % count)) else {
            continue;
        };
        let (px, py) = (render::nm(p.first), render::nm(p.second));
        let (qx, qy) = (render::nm(q.first), render::nm(q.second));
        // On this side: not inside.
        let cross = (qx - px) * (y - py) - (qy - py) * (x - px);
        let length = ((qx - px).powi(2) + (qy - py).powi(2)).sqrt();
        if cross.abs() <= ON_SIDE * length
            && x >= px.min(qx) - ON_SIDE
            && x <= px.max(qx) + ON_SIDE
            && y >= py.min(qy) - ON_SIDE
            && y <= py.max(qy) + ON_SIDE
        {
            return false;
        }
        if (py > y) != (qy > y) {
            let at = px + (y - py) * (qx - px) / (qy - py);
            if at > x {
                inside = !inside;
            }
        }
    }
    inside
}

/// Is a local point inside an angled cut? Its sides count as wood; its
/// ends are open, so where it runs out through a face, the face is not
/// wood there.
fn in_prism(prism: &Prism, p: DVec3) -> bool {
    let depth = p[index(prism.face_axis())];
    if depth < render::nm(prism.depth().from()) || depth > render::nm(prism.depth().to()) {
        return false;
    }
    let [a, b] = across(prism.face_axis()).map(index);
    outline_holds(prism.outline().points(), p[a], p[b])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_offsets_are_26_unit_directions() {
        for (i, a) in OFFSETS.iter().enumerate() {
            assert!((a.length() - 1.0).abs() < 1e-12);
            for b in OFFSETS.iter().skip(i + 1) {
                assert!((*a - *b).length() > 0.1);
            }
        }
    }
}
