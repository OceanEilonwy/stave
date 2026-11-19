//! Mesh builders: boxes and lathe shapes in a part's frame.

use std::f32::consts::TAU;

use glam::{Vec2, Vec3};
use stave_model::solved::{Part, Pose};

use crate::{Aabb, Edges, Geometry, Mesh, Vertex, render};

/// How many facets a lathe mesh has around its axis.
pub const LATHE_SEGMENTS: u16 = 48;

/// Profile corners sharper than this (between neighbouring facets) get a
/// crease and an outline ring; gentler ones are smoothed.
const CREASE_ANGLE_DEGREES: f32 = 35.0;

/// How many pieces each curved span of a smooth profile is drawn with.
const SMOOTH_STEPS: u16 = 12;

/// Lengths below this (millimetres) count as zero.
const EPSILON: f32 = 1e-4;

/// A local frame in the render frame: an origin and three unit axes (which
/// may be left-handed for mirrored parts).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Frame {
    origin: Vec3,
    axes: [Vec3; 3],
}

impl Frame {
    /// A part's frame.
    pub fn of_part(part: &Part) -> Frame {
        Frame::of_pose(&part.pose)
    }

    /// The frame of a pose.
    pub fn of_pose(pose: &Pose) -> Frame {
        let orientation = pose.orientation;
        Frame {
            origin: render::point(pose.origin),
            axes: [
                render::direction(orientation.length()),
                render::direction(orientation.width()),
                render::direction(orientation.thickness()),
            ],
        }
    }

    /// A part's own frame, of the given handedness: the origin at the
    /// part's corner and its length, width and thickness along `x`, `y`
    /// and `z` (`-z` for a left-handed, mirrored part). A mesh built in it
    /// is placed by [`crate::rest`], which is then a rotation and a
    /// move, never a reflection, so the winding stays right.
    pub(crate) fn own(handedness: crate::Handedness) -> Frame {
        let thickness = match handedness {
            crate::Handedness::Right => Vec3::Z,
            crate::Handedness::Left => Vec3::NEG_Z,
        };
        Frame {
            origin: Vec3::ZERO,
            axes: [Vec3::X, Vec3::Y, thickness],
        }
    }

    /// The matrix taking points in `self` to points in `world`, both
    /// given as frames in the render frame (`self`'s axes at right
    /// angles).
    pub(crate) fn onto(&self, world: &Frame) -> glam::Mat4 {
        // A point at local coordinates p is self.origin + Σ self.axes[i]·p_i
        // here and world.origin + Σ world.axes[i]·p_i there.
        let [a, b, c] = world.axes;
        let to_world = glam::Mat4::from_cols(
            a.extend(0.0),
            b.extend(0.0),
            c.extend(0.0),
            world.origin.extend(1.0),
        );
        let [x, y, z] = self.axes;
        // `self`'s axes are orthonormal: its inverse is its transpose.
        let rotation = glam::Mat3::from_cols(x, y, z).transpose();
        let from_self =
            glam::Mat4::from_mat3(rotation) * glam::Mat4::from_translation(-self.origin);
        to_world * from_self
    }

    /// A frame from an origin and three unit axes.
    pub(crate) fn new(origin: Vec3, axes: [Vec3; 3]) -> Frame {
        Frame { origin, axes }
    }

    /// The same frame with `axis` first and the two axes across it after,
    /// in [`crate::solid::across`] order. The handedness may change; every
    /// builder winds triangles by their normals, so that doesn't matter.
    pub fn first(&self, axis: stave_lang::placement::PartAxis) -> Frame {
        use stave_lang::placement::PartAxis;
        let [l, w, t] = self.axes;
        let axes = match axis {
            PartAxis::Length => [l, w, t],
            PartAxis::Width => [w, l, t],
            PartAxis::Thickness => [t, l, w],
        };
        Frame {
            origin: self.origin,
            axes,
        }
    }

    pub(crate) fn point(&self, local: Vec3) -> Vec3 {
        let [a, b, c] = self.axes;
        self.origin + a * local.x + b * local.y + c * local.z
    }

    /// A render-frame point in this frame's local coordinates (the axes
    /// are unit and at right angles).
    pub(crate) fn local(&self, point: Vec3) -> Vec3 {
        let [a, b, c] = self.axes;
        let offset = point - self.origin;
        Vec3::new(offset.dot(a), offset.dot(b), offset.dot(c))
    }

    pub(crate) fn vector(&self, local: Vec3) -> Vec3 {
        let [a, b, c] = self.axes;
        a * local.x + b * local.y + c * local.z
    }
}

/// Which local axis the grain runs along, for the grain attribute.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GrainAxes {
    along_width: bool,
}

impl GrainAxes {
    /// Grain along the first axis.
    pub const LENGTH: GrainAxes = GrainAxes { along_width: false };
    /// Grain along the second axis.
    pub const WIDTH: GrainAxes = GrainAxes { along_width: true };

    pub(crate) fn apply(self, local: Vec3) -> [f32; 3] {
        if self.along_width {
            [local.y, local.x, local.z]
        } else {
            [local.x, local.y, local.z]
        }
    }
}

/// A mesh under construction.
#[derive(Default)]
pub(crate) struct Builder {
    mesh: Mesh,
    edges: Edges,
    /// Vertices added now are on a face a cut made.
    cut: bool,
}

impl Builder {
    /// Marks the vertices added from now on as on a face a cut made, or
    /// on the stock's own surface.
    pub(crate) fn set_cut(&mut self, cut: bool) {
        self.cut = cut;
    }

    pub(crate) fn vertex(&mut self, position: Vec3, normal: Vec3, grain: [f32; 3]) -> u32 {
        let index = u32::try_from(self.mesh.vertices.len()).unwrap_or(u32::MAX);
        self.mesh.vertices.push(Vertex {
            position: position.to_array(),
            normal: normal.normalize_or_zero().to_array(),
            grain,
            cut: if self.cut { 1.0 } else { 0.0 },
        });
        index
    }

    fn position(&self, index: u32) -> Option<(Vec3, Vec3)> {
        let vertex = self.mesh.vertices.get(usize::try_from(index).ok()?)?;
        Some((Vec3::from(vertex.position), Vec3::from(vertex.normal)))
    }

    /// Adds a triangle, wound so its geometric normal agrees with its
    /// vertices' normals; degenerate triangles are dropped.
    pub(crate) fn triangle(&mut self, a: u32, b: u32, c: u32) {
        let (Some((pa, na)), Some((pb, nb)), Some((pc, nc))) =
            (self.position(a), self.position(b), self.position(c))
        else {
            return;
        };
        let geometric = (pb - pa).cross(pc - pa);
        if geometric.length_squared() < EPSILON * EPSILON {
            return;
        }
        if geometric.dot(na + nb + nc) >= 0.0 {
            self.mesh.indices.extend([a, b, c]);
        } else {
            self.mesh.indices.extend([a, c, b]);
        }
    }

    pub(crate) fn edge(&mut self, from: Vec3, to: Vec3) {
        self.edges.segments.push([from.to_array(), to.to_array()]);
    }

    pub(crate) fn finish(self) -> Geometry {
        let bounds = self.mesh.bounds().unwrap_or(Aabb {
            min: Vec3::ZERO,
            max: Vec3::ZERO,
        });
        Geometry {
            mesh: self.mesh,
            edges: self.edges,
            bounds,
        }
    }
}

/// A box from local `offset` to `offset + size`, with flat faces and its
/// twelve edges.
pub fn cuboid(frame: &Frame, offset: Vec3, size: Vec3, grain: GrainAxes) -> Geometry {
    let mut builder = Builder::default();
    let corner = |bits: [bool; 3]| -> Vec3 {
        let pick = |axis: usize, set: bool| if set { size[axis] } else { 0.0 };
        offset + Vec3::new(pick(0, bits[0]), pick(1, bits[1]), pick(2, bits[2]))
    };
    for axis in 0..3 {
        let (u, v) = ((axis + 1) % 3, (axis + 2) % 3);
        for high in [false, true] {
            let mut local_normal = Vec3::ZERO;
            local_normal[axis] = if high { 1.0 } else { -1.0 };
            let normal = frame.vector(local_normal);
            let quad =
                [(false, false), (true, false), (true, true), (false, true)].map(|(bu, bv)| {
                    let mut bits = [false; 3];
                    bits[axis] = high;
                    bits[u] = bu;
                    bits[v] = bv;
                    let local = corner(bits);
                    builder.vertex(frame.point(local), normal, grain.apply(local))
                });
            builder.triangle(quad[0], quad[1], quad[2]);
            builder.triangle(quad[0], quad[2], quad[3]);
        }
    }
    for axis in 0..3 {
        let (u, v) = ((axis + 1) % 3, (axis + 2) % 3);
        for (bu, bv) in [(false, false), (true, false), (true, true), (false, true)] {
            let mut start = [false; 3];
            start[u] = bu;
            start[v] = bv;
            let mut end = start;
            end[axis] = true;
            builder.edge(frame.point(corner(start)), frame.point(corner(end)));
        }
    }
    builder.finish()
}

/// A point of a lathe profile: a radius at a distance along the axis
/// (millimetres).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ProfilePoint {
    /// Distance along the axis from the frame's origin.
    pub along: f32,
    /// Radius there.
    pub radius: f32,
}

impl ProfilePoint {
    /// A profile point.
    pub fn new(along: f32, radius: f32) -> Self {
        ProfilePoint {
            along,
            radius: radius.max(0.0),
        }
    }

    fn vector(self) -> Vec2 {
        Vec2::new(self.along, self.radius)
    }
}

/// One smooth curve through the stations (a Catmull-Rom spline), split at
/// shoulders (two stations at the same distance); a span between two
/// stations of the same radius stays straight.
pub fn smooth_profile(points: &[ProfilePoint]) -> Vec<ProfilePoint> {
    let mut out: Vec<ProfilePoint> = Vec::new();
    for (index, pair) in points.windows(2).enumerate() {
        let &[from, to] = pair else { continue };
        if out.is_empty() {
            out.push(from);
        }
        let shoulder = (to.along - from.along).abs() < EPSILON;
        let straight = (to.radius - from.radius).abs() < EPSILON;
        if shoulder || straight {
            out.push(to);
            continue;
        }
        // Neighbours across a shoulder don't shape this span.
        let before = index
            .checked_sub(1)
            .and_then(|i| points.get(i))
            .filter(|p| from.along - p.along > EPSILON)
            .map_or(from.vector() * 2.0 - to.vector(), |p| p.vector());
        let after = points
            .get(index + 2)
            .filter(|p| p.along - to.along > EPSILON)
            .map_or(to.vector() * 2.0 - from.vector(), |p| p.vector());
        let (p0, p1, p2, p3) = (before, from.vector(), to.vector(), after);
        for step in 1..=SMOOTH_STEPS {
            let t = f32::from(step) / f32::from(SMOOTH_STEPS);
            let (t2, t3) = (t * t, t * t * t);
            let point = 0.5
                * ((2.0 * p1)
                    + (p2 - p0) * t
                    + (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3) * t2
                    + (3.0 * p1 - p0 - 3.0 * p2 + p3) * t3);
            let previous = out.last().map_or(point.x, |p| p.along);
            // Keep the curve inside the span and moving forward.
            let along = point.x.clamp(previous, to.along);
            out.push(ProfilePoint::new(along, point.y));
        }
    }
    if out.is_empty()
        && let Some(only) = points.first()
    {
        out.push(*only);
    }
    out
}

/// Which profile corners a lathe mesh shades smoothly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Corners {
    /// Every corner is a crease (a profile of straight lines).
    Sharp,
    /// Corners gentler than the crease angle are smoothed (a curve).
    ByAngle,
}

/// A solid of revolution about the frame's first axis, through the point
/// `centre` of its second and third axes. End caps close it where the
/// radius isn't zero. Outline rings mark the ends and sharp corners.
pub fn lathe(
    frame: &Frame,
    centre: [f32; 2],
    profile: &[ProfilePoint],
    corners: Corners,
    grain: GrainAxes,
) -> Geometry {
    lathe_zoned(frame, centre, profile, corners, grain, &CutZones::default())
}

/// Where a lathe mesh's surface is a face a cut made: spans whose middle
/// lies in a zone (millimetres along the axis, ends included), and end
/// caps except at the stock's own ends.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct CutZones {
    /// Ranges along the axis.
    pub zones: Vec<(f32, f32)>,
    /// The stock's ends along the axis: caps there are its own surface.
    pub ends: Option<(f32, f32)>,
}

impl CutZones {
    fn covers(&self, along: f32) -> bool {
        self.zones
            .iter()
            .any(|(low, high)| along >= *low - EPSILON && along <= *high + EPSILON)
    }

    fn cap_is_cut(&self, along: f32) -> bool {
        match self.ends {
            Some((low, high)) => {
                (along - low).abs() > EPSILON
                    && (along - high).abs() > EPSILON
                    && self.covers(along)
            }
            None => false,
        }
    }
}

/// A lathe mesh whose surface is a cut face over `zones`.
pub(crate) fn lathe_zoned(
    frame: &Frame,
    centre: [f32; 2],
    profile: &[ProfilePoint],
    corners: Corners,
    grain: GrainAxes,
    zones: &CutZones,
) -> Geometry {
    let mut builder = Builder::default();
    // Drop repeated points: they make zero-length spans.
    let mut points: Vec<ProfilePoint> = Vec::with_capacity(profile.len());
    for point in profile {
        if points
            .last()
            .is_none_or(|last| (last.vector() - point.vector()).length() > EPSILON)
        {
            points.push(*point);
        }
    }
    let spans: Vec<(ProfilePoint, ProfilePoint, Vec2)> = points
        .windows(2)
        .filter_map(|pair| {
            let &[from, to] = pair else { return None };
            let d = to.vector() - from.vector();
            Some((from, to, Vec2::new(-d.y, d.x).normalize_or_zero()))
        })
        .collect();
    let crease = CREASE_ANGLE_DEGREES.to_radians().cos();
    // The normal at each point: averaged when the corner is gentle, `None`
    // when it is sharp (each span then keeps its own).
    let smooth_at = |index: usize| -> Option<Vec2> {
        let before = spans.get(index.checked_sub(1)?)?.2;
        let after = spans.get(index)?.2;
        (corners == Corners::ByAngle && before.dot(after) >= crease)
            .then(|| (before + after).normalize_or_zero())
    };
    let ring = |around: u16| -> Vec3 {
        let angle = TAU * f32::from(around) / f32::from(LATHE_SEGMENTS);
        Vec3::new(0.0, angle.cos(), angle.sin())
    };
    let centre_local = Vec3::new(0.0, centre[0], centre[1]);
    let local_at = |point: ProfilePoint, around: u16| -> Vec3 {
        centre_local + Vec3::new(point.along, 0.0, 0.0) + ring(around) * point.radius
    };
    let add_ring = |builder: &mut Builder, point: ProfilePoint, normal: Vec2| -> Vec<u32> {
        (0..LATHE_SEGMENTS)
            .map(|around| {
                let local = local_at(point, around);
                let local_normal = Vec3::new(normal.x, 0.0, 0.0) + ring(around) * normal.y;
                builder.vertex(
                    frame.point(local),
                    frame.vector(local_normal),
                    grain.apply(local),
                )
            })
            .collect()
    };
    for (index, (from, to, normal)) in spans.iter().enumerate() {
        let start_normal = smooth_at(index).unwrap_or(*normal);
        let end_normal = smooth_at(index + 1).unwrap_or(*normal);
        builder.set_cut(zones.covers((from.along + to.along) * 0.5));
        let first = add_ring(&mut builder, *from, start_normal);
        let second = add_ring(&mut builder, *to, end_normal);
        for around in 0..first.len() {
            let next = (around + 1) % first.len();
            let (Some(&a), Some(&b), Some(&c), Some(&d)) = (
                first.get(around),
                first.get(next),
                second.get(next),
                second.get(around),
            ) else {
                continue;
            };
            builder.triangle(a, b, c);
            builder.triangle(a, c, d);
        }
    }
    // End caps.
    for (point, outward) in [
        (points.first().copied(), -1.0_f32),
        (points.last().copied(), 1.0_f32),
    ] {
        let Some(point) = point else { continue };
        if point.radius <= EPSILON {
            continue;
        }
        builder.set_cut(zones.cap_is_cut(point.along));
        let normal = frame.vector(Vec3::new(outward, 0.0, 0.0));
        let middle_local = centre_local + Vec3::new(point.along, 0.0, 0.0);
        let middle = builder.vertex(frame.point(middle_local), normal, grain.apply(middle_local));
        let rim: Vec<u32> = (0..LATHE_SEGMENTS)
            .map(|around| {
                let local = local_at(point, around);
                builder.vertex(frame.point(local), normal, grain.apply(local))
            })
            .collect();
        for around in 0..rim.len() {
            if let (Some(&a), Some(&b)) = (rim.get(around), rim.get((around + 1) % rim.len())) {
                builder.triangle(middle, a, b);
            }
        }
    }
    // Outline rings: the ends and every sharp corner.
    for (index, point) in points.iter().enumerate() {
        let is_end = index == 0 || index + 1 == points.len();
        if point.radius <= EPSILON || !(is_end || smooth_at(index).is_none()) {
            continue;
        }
        for around in 0..LATHE_SEGMENTS {
            let from = frame.point(local_at(*point, around));
            let to = frame.point(local_at(*point, (around + 1) % LATHE_SEGMENTS));
            builder.edge(from, to);
        }
    }
    builder.finish()
}
