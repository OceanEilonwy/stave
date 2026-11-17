//! Points in and on a part's cut solid, in floats: what an assembly step
//! that turns a part, or moves it on a diagonal, is tested with.
//!
//! [`Shape::contains`] says whether a point lies inside the solid deeper
//! than a margin (the contact tolerance), so surfaces that touch don't
//! count. The solid is the blocks and rods of wood (round sections with
//! their flats), less the holes and the angled slots. The margin is
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
use crate::solid::{AngledSlot, Rod, Solid, across};

/// Lengths below this (nanometres) count as on a surface.
const NUDGE: f64 = 1_000.0;

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
            && !self.solid.slots().iter().any(|slot| in_slot(slot, p))
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
        // Walls of angled slots, across the whole part.
        let reach = self.size.length();
        for slot in self.solid.slots() {
            let (u, v) = slot_frame(slot);
            let centre = local(slot);
            let face = unit(slot.face_axis);
            let half = render::nm(slot.width) / 2.0;
            let (d0, d1) = (render::nm(slot.depth.0), render::nm(slot.depth.1));
            let start = if slot.from_centre { 0.0 } else { -reach };
            for side in [-1.0, 1.0] {
                for along in steps(start, reach, spacing) {
                    for depth in steps(d0, d1, spacing) {
                        let p = centre + u * along + v * (half * side) + face * depth;
                        keep(p, -v * side, &mut out);
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

/// The unit vectors along and across an angled slot, in local axes.
fn slot_frame(slot: &AngledSlot) -> (DVec3, DVec3) {
    let signed = |(axis, positive): (PartAxis, bool)| {
        if positive { unit(axis) } else { -unit(axis) }
    };
    let (base, next) = (signed(slot.base), signed(slot.next));
    let angle = render::radians_f64(slot.angle);
    let u = base * angle.cos() + next * angle.sin();
    let v = next * angle.cos() - base * angle.sin();
    (u, v)
}

fn local(slot: &AngledSlot) -> DVec3 {
    DVec3::new(
        render::nm(slot.centre.length),
        render::nm(slot.centre.width),
        render::nm(slot.centre.thickness),
    )
}

/// Is a local point in an angled slot (open, so its walls count as wood)?
fn in_slot(slot: &AngledSlot, p: DVec3) -> bool {
    let depth = p[index(slot.face_axis)];
    if depth < render::nm(slot.depth.0) || depth > render::nm(slot.depth.1) {
        return false;
    }
    let (u, v) = slot_frame(slot);
    let mut relative = p - local(slot);
    relative[index(slot.face_axis)] = 0.0;
    let across = relative.dot(v).abs();
    across < render::nm(slot.width) / 2.0 && (!slot.from_centre || relative.dot(u) > 0.0)
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
