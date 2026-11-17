//! Drawing what the grid of a board can't: slots turned by an angle, and
//! the flats of a round section.
//!
//! An [`AngledSlot`] is a band across its face, `width` wide, running from
//! its centre out along `u` (or both ways), through the face axis over its
//! depth range. A board face in a plane across the slot's face axis is
//! clipped to the outside of the band: three convex pieces (left of the
//! band, right of it, and behind its start), each cut by Sutherland–Hodgman
//! from the face polygon. A face along the slot's face axis loses the
//! rectangle where the band runs out through it. The band's two walls are
//! drawn wherever they pass through wood.
//!
//! A round section with flats is drawn as a prism of its outline: each
//! ring point is the round's radius, cut back to the flat where the flat
//! is nearer, `min(R, (f/2) / |cos(θ - φ)|)`, with the ends of each flat
//! among the ring's angles so the flats are crisp.

use std::f32::consts::{PI, TAU};

use glam::Vec3;
use stave_lang::placement::PartAxis;

use crate::build::{Builder, Frame, GrainAxes};
use crate::render;
use crate::solid::{AngledSlot, Rod, across};

/// The slot of a part axis in a local vector.
pub(crate) fn slot_of(axis: PartAxis) -> usize {
    match axis {
        PartAxis::Length => 0,
        PartAxis::Width => 1,
        PartAxis::Thickness => 2,
    }
}

fn unit(axis: PartAxis) -> Vec3 {
    let mut v = Vec3::ZERO;
    v[slot_of(axis)] = 1.0;
    v
}

/// A slot in local millimetres.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Band {
    /// The face axis it runs through.
    pub(crate) face: usize,
    /// Its centre (the face coordinate is 0).
    pub(crate) centre: Vec3,
    /// Along it, from the centre out.
    pub(crate) u: Vec3,
    /// Across it.
    pub(crate) v: Vec3,
    /// Half its width.
    pub(crate) half: f32,
    /// Only on the `u` side of the centre.
    pub(crate) from_centre: bool,
    /// Its range along the face axis.
    pub(crate) depth: (f32, f32),
}

impl Band {
    pub(crate) fn of(slot: &AngledSlot) -> Band {
        let signed = |(axis, positive): (PartAxis, bool)| {
            if positive { unit(axis) } else { -unit(axis) }
        };
        let (base, next) = (signed(slot.base), signed(slot.next));
        let angle = render::radians(slot.angle);
        let u = base * angle.cos() + next * angle.sin();
        let v = next * angle.cos() - base * angle.sin();
        let mut centre = Vec3::new(
            render::mm(slot.centre.length),
            render::mm(slot.centre.width),
            render::mm(slot.centre.thickness),
        );
        centre[slot_of(slot.face_axis)] = 0.0;
        Band {
            face: slot_of(slot.face_axis),
            centre,
            u,
            v,
            half: render::mm(slot.width) * 0.5,
            from_centre: slot.from_centre,
            depth: (render::mm(slot.depth.0), render::mm(slot.depth.1)),
        }
    }

    fn relative(&self, p: Vec3) -> Vec3 {
        let mut r = p - self.centre;
        r[self.face] = 0.0;
        r
    }

    /// Is a point strictly inside the band (ignoring depth)?
    pub(crate) fn holds(&self, p: Vec3) -> bool {
        let r = self.relative(p);
        r.dot(self.v).abs() < self.half - 1e-4 && (!self.from_centre || r.dot(self.u) > 1e-4)
    }

    fn reaches(&self, along_face: f32) -> bool {
        along_face >= self.depth.0 - 1e-4 && along_face <= self.depth.1 + 1e-4
    }

    /// Clips a planar polygon (across the face axis) to the outside of the
    /// band: up to three convex pieces.
    pub(crate) fn outside(&self, polygon: &[Vec3]) -> Vec<Vec<Vec3>> {
        // Half-planes as (normal, offset) about the centre: n·r <= c.
        let left = [(self.v, -self.half)];
        let right = [(-self.v, -self.half)];
        let behind = [(self.u, 0.0), (self.v, self.half), (-self.v, self.half)];
        let mut pieces: Vec<&[(Vec3, f32)]> = vec![&left, &right];
        if self.from_centre {
            pieces.push(&behind);
        }
        pieces
            .into_iter()
            .filter_map(|planes| {
                let mut out = polygon.to_vec();
                for (normal, offset) in planes {
                    out = clip(&out, |p| normal.dot(self.relative(p)) - offset);
                    if out.len() < 3 {
                        return None;
                    }
                }
                (area(&out) > 1e-6).then_some(out)
            })
            .collect()
    }

    /// Where the band crosses a plane along the face axis (normal axis
    /// `a`, at `plane`), as a range along the in-plane axis `t`.
    fn crossing(&self, a: usize, plane: f32, t: usize) -> Option<(f32, f32)> {
        let ra = plane - self.centre[a];
        let mut low = f32::NEG_INFINITY;
        let mut high = f32::INFINITY;
        // half >= |v_a ra + v_t rt|, and (from centre) u_a ra + u_t rt > 0.
        let mut limit = |coefficient: f32, constant: f32, below: f32, above: f32| {
            // below <= constant + coefficient * rt <= above
            if coefficient.abs() < 1e-6 {
                if constant < below || constant > above {
                    low = f32::INFINITY;
                }
                return;
            }
            let (a0, a1) = (
                (below - constant) / coefficient,
                (above - constant) / coefficient,
            );
            low = low.max(a0.min(a1));
            high = high.min(a0.max(a1));
        };
        limit(self.v[t], self.v[a] * ra, -self.half, self.half);
        if self.from_centre {
            limit(self.u[t], self.u[a] * ra, 0.0, f32::INFINITY);
        }
        (low < high).then_some((low + self.centre[t], high + self.centre[t]))
    }
}

/// Sutherland–Hodgman against `side(p) <= 0`.
fn clip(polygon: &[Vec3], side: impl Fn(Vec3) -> f32) -> Vec<Vec3> {
    let mut out = Vec::with_capacity(polygon.len() + 2);
    for (index, current) in polygon.iter().enumerate() {
        let next = polygon[(index + 1) % polygon.len()];
        let (a, b) = (side(*current), side(next));
        if a <= 0.0 {
            out.push(*current);
        }
        if (a < 0.0 && b > 0.0) || (a > 0.0 && b < 0.0) {
            let t = a / (a - b);
            out.push(*current + (next - *current) * t);
        }
    }
    out
}

fn area(polygon: &[Vec3]) -> f32 {
    let mut sum = Vec3::ZERO;
    for (index, p) in polygon.iter().enumerate() {
        sum += p.cross(polygon[(index + 1) % polygon.len()]);
    }
    sum.length() * 0.5
}

/// Draws a face polygon (local, planar, normal along local axis `a`),
/// cut by every band that reaches its plane: across a band's face axis it
/// is clipped to the outside; along it, the band's footprint rectangle is
/// cut out.
pub(crate) fn face(
    builder: &mut Builder,
    frame: &Frame,
    grain: GrainAxes,
    bands: &[Band],
    a: usize,
    polygon: Vec<Vec3>,
    normal: Vec3,
) {
    let plane = polygon.first().map_or(0.0, |p| p[a]);
    let mut pieces = vec![polygon];
    for band in bands {
        if band.face == a {
            if !band.reaches(plane) {
                continue;
            }
            pieces = pieces.iter().flat_map(|p| band.outside(p)).collect();
        } else {
            // The in-plane axis across the band's face axis.
            let t = 3 - a - band.face;
            let Some((t0, t1)) = band.crossing(a, plane, t) else {
                continue;
            };
            let (d0, d1) = band.depth;
            pieces = pieces
                .iter()
                .flat_map(|p| cut_rectangle(p, t, (t0, t1), band.face, (d0, d1)))
                .collect();
        }
    }
    let world_normal = frame.vector(normal);
    for piece in pieces {
        let vertices: Vec<u32> = piece
            .iter()
            .map(|l| builder.vertex(frame.point(*l), world_normal, grain.apply(*l)))
            .collect();
        for index in 1..vertices.len().saturating_sub(1) {
            if let (Some(&x), Some(&y), Some(&z)) = (
                vertices.first(),
                vertices.get(index),
                vertices.get(index + 1),
            ) {
                builder.triangle(x, y, z);
            }
        }
    }
}

/// A convex polygon less an axis-aligned rectangle `t0..t1` × `d0..d1`
/// (on local axes `t` and `d`): four convex pieces at most.
fn cut_rectangle(
    polygon: &[Vec3],
    t: usize,
    (t0, t1): (f32, f32),
    d: usize,
    (d0, d1): (f32, f32),
) -> Vec<Vec<Vec3>> {
    let low_t = clip(polygon, |p| p[t] - t0);
    let high_t = clip(polygon, |p| t1 - p[t]);
    let middle = clip(&clip(polygon, |p| t0 - p[t]), |p| p[t] - t1);
    let low_d = clip(&middle, |p| p[d] - d0);
    let high_d = clip(&middle, |p| d1 - p[d]);
    [low_t, high_t, low_d, high_d]
        .into_iter()
        .filter(|p| p.len() >= 3 && area(p) > 1e-6)
        .collect()
}

/// Does any band reach a face polygon (normal along local axis `a`)?
pub(crate) fn cuts(bands: &[Band], a: usize, polygon: &[Vec3]) -> bool {
    let plane = polygon.first().map_or(0.0, |p| p[a]);
    bands.iter().any(|band| {
        if band.face == a {
            band.reaches(plane) && polygon_meets_band(band, polygon)
        } else {
            band.crossing(a, plane, 3 - a - band.face).is_some()
        }
    })
}

fn polygon_meets_band(band: &Band, polygon: &[Vec3]) -> bool {
    let outside: f32 = band.outside(polygon).iter().map(|p| area(p)).sum();
    (area(polygon) - outside).abs() > 1e-5
}

/// Is a point (on a plane across axis `a`) inside a band that reaches it?
pub(crate) fn in_any(bands: &[Band], a: usize, p: Vec3) -> bool {
    bands
        .iter()
        .any(|band| band.face == a && band.reaches(p[a]) && band.holds(p))
}

/// A rod's centre as a local point (its own axis left at 0).
pub(crate) fn flat_centre(rod: &Rod) -> Vec3 {
    let [ua, va] = across(rod.axis).map(slot_of);
    let mut p = Vec3::ZERO;
    p[ua] = render::mm(rod.centre[0]);
    p[va] = render::mm(rod.centre[1]);
    p
}

/// Draws a band's walls where they pass through a wood cell (local box
/// `low..high`), outside the holes (`holes`: centre on the face plane and
/// radius, for holes along the band's face axis). `open` says whether the
/// cell's neighbours below and above along the face axis are air, for the
/// outlines.
pub(crate) fn walls(
    builder: &mut Builder,
    frame: &Frame,
    grain: GrainAxes,
    band: &Band,
    (low, high): (Vec3, Vec3),
    holes: &[(Vec3, f32)],
    open: (bool, bool),
) {
    let (z0, z1) = (
        low[band.face].max(band.depth.0),
        high[band.face].min(band.depth.1),
    );
    if z1 - z0 < 1e-4 {
        return;
    }
    let flat = |p: Vec3| {
        let mut q = p;
        q[band.face] = 0.0;
        q
    };
    let (lo, hi) = (flat(low), flat(high));
    // Each wall as a line (origin, direction, range) and the way it faces
    // (into the band): the two long walls, and the end wall where a slot
    // from the centre starts.
    let start = if band.from_centre { 0.0 } else { -1e6 };
    let mut lines = vec![
        (
            band.centre - band.v * band.half,
            band.u,
            (start, 1e6),
            band.v,
        ),
        (
            band.centre + band.v * band.half,
            band.u,
            (start, 1e6),
            -band.v,
        ),
    ];
    if band.from_centre {
        lines.push((
            band.centre - band.v * band.half,
            band.v,
            (0.0, band.half * 2.0),
            band.u,
        ));
    }
    for (offset, direction, (mut s0, mut s1), facing) in lines {
        // Clipped to the cell.
        for axis in 0..3 {
            if axis == band.face {
                continue;
            }
            let (o, d) = (offset[axis], direction[axis]);
            if d.abs() < 1e-6 {
                if o < lo[axis] || o > hi[axis] {
                    s1 = s0 - 1.0;
                }
                continue;
            }
            let (a, b) = ((lo[axis] - o) / d, (hi[axis] - o) / d);
            s0 = s0.max(a.min(b));
            s1 = s1.min(a.max(b));
        }
        // Outside every hole.
        for (centre, radius) in holes {
            let r = offset - *centre;
            // |r + s d|^2 = R^2
            let (b, c) = (r.dot(direction), r.dot(r) - radius * radius);
            let discriminant = b * b - c;
            if discriminant > 0.0 {
                let root = discriminant.sqrt();
                let (h0, h1) = (-b - root, -b + root);
                if h0 <= s0 && h1 >= s1 {
                    s1 = s0 - 1.0;
                } else if h0 <= s0 && h1 > s0 {
                    s0 = h1;
                } else if h1 >= s1 && h0 < s1 {
                    s1 = h0;
                }
            }
        }
        if s1 - s0 < 1e-4 {
            continue;
        }
        let point = |s: f32, z: f32| {
            let mut p = offset + direction * s;
            p[band.face] = z;
            p
        };
        let normal = frame.vector(facing);
        let quad = [point(s0, z0), point(s1, z0), point(s1, z1), point(s0, z1)];
        let indices = quad.map(|l| builder.vertex(frame.point(l), normal, grain.apply(l)));
        builder.triangle(indices[0], indices[1], indices[2]);
        builder.triangle(indices[0], indices[2], indices[3]);
        if open.0 || z0 > low[band.face] + 1e-4 {
            builder.edge(frame.point(quad[0]), frame.point(quad[1]));
        }
        if open.1 || z1 < high[band.face] - 1e-4 {
            builder.edge(frame.point(quad[3]), frame.point(quad[2]));
        }
    }
}

/// A round section with flats, as a prism of its outline, with its end
/// faces and outlines.
pub(crate) fn flatted_rod(
    builder: &mut Builder,
    frame: &Frame,
    grain: GrainAxes,
    rod: &Rod,
    segments: u16,
) {
    let Some(flats) = rod.flats else {
        return;
    };
    let radius = render::mm(rod.diameter) * 0.5;
    let half = render::mm(flats.across) * 0.5;
    let phi = render::radians(flats.angle);
    let axis = slot_of(rod.axis);
    let [ua, va] = across(rod.axis).map(slot_of);
    let centre = (render::mm(rod.centre[0]), render::mm(rod.centre[1]));
    // Ring angles: even steps, and the ends of each flat.
    let mut angles: Vec<f32> = (0..segments)
        .map(|k| TAU * f32::from(k) / f32::from(segments))
        .collect();
    if half < radius {
        let alpha = (half / radius).acos();
        for base in [phi, phi + PI] {
            for a in [base - alpha, base + alpha] {
                angles.push(a.rem_euclid(TAU));
            }
        }
    }
    angles.sort_by(f32::total_cmp);
    angles.dedup_by(|a, b| (*a - *b).abs() < 1e-5);
    let reach = |theta: f32| {
        let c = (theta - phi).cos().abs();
        if c * radius > half { half / c } else { radius }
    };
    let on_flat = |theta: f32| (theta - phi).cos().abs() * radius > half + 1e-4;
    let local = |theta: f32, along: f32| {
        let mut p = Vec3::ZERO;
        p[axis] = along;
        let r = reach(theta);
        p[ua] = centre.0 + r * theta.cos();
        p[va] = centre.1 + r * theta.sin();
        p
    };
    let radial = |theta: f32| {
        let mut n = Vec3::ZERO;
        n[ua] = theta.cos();
        n[va] = theta.sin();
        n
    };
    let flat_normal = |theta: f32| {
        let sign = (theta - phi).cos().signum();
        let mut n = Vec3::ZERO;
        n[ua] = phi.cos() * sign;
        n[va] = phi.sin() * sign;
        n
    };
    let (start, end) = (render::mm(rod.start), render::mm(rod.end));
    for index in 0..angles.len() {
        let (t0, t1) = (angles[index], angles[(index + 1) % angles.len()]);
        let t1u = if t1 <= t0 { t1 + TAU } else { t1 };
        let middle = (t0 + t1u) * 0.5;
        let flat = on_flat(middle);
        let normal_at = |t: f32| if flat { flat_normal(middle) } else { radial(t) };
        let quad = [(t0, start), (t1, start), (t1, end), (t0, end)].map(|(t, along)| {
            let l = local(t, along);
            builder.vertex(frame.point(l), frame.vector(normal_at(t)), grain.apply(l))
        });
        builder.triangle(quad[0], quad[1], quad[2]);
        builder.triangle(quad[0], quad[2], quad[3]);
        for along in [start, end] {
            builder.edge(frame.point(local(t0, along)), frame.point(local(t1, along)));
        }
        // The edges where a flat meets the round.
        if flat != on_flat((t0 + angles[(index + angles.len() - 1) % angles.len()]) * 0.5)
            || (index == 0 && flat != on_flat(t0 - 1e-3))
        {
            builder.edge(frame.point(local(t0, start)), frame.point(local(t0, end)));
        }
    }
    // End faces.
    for (along, sign) in [(start, -1.0_f32), (end, 1.0)] {
        let mut normal = Vec3::ZERO;
        normal[axis] = sign;
        let world = frame.vector(normal);
        let mut middle = Vec3::ZERO;
        middle[axis] = along;
        middle[ua] = centre.0;
        middle[va] = centre.1;
        let m = builder.vertex(frame.point(middle), world, grain.apply(middle));
        let rim: Vec<u32> = angles
            .iter()
            .map(|t| {
                let l = local(*t, along);
                builder.vertex(frame.point(l), world, grain.apply(l))
            })
            .collect();
        for index in 0..rim.len() {
            if let (Some(&x), Some(&y)) = (rim.get(index), rim.get((index + 1) % rim.len())) {
                builder.triangle(m, x, y);
            }
        }
    }
}
