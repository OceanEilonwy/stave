//! Meshes for the parts and hardware of a [`SolvedDesign`]: what the viewer
//! draws and what the glTF export writes.
//!
//! This is the geometry boundary of Stave: everything above it is exact
//! ([`Length`], nanometres), everything here is `f32` millimetres. The one
//! conversion lives in [`render`]; nothing else turns an exact value into a
//! float.
//!
//! # Render frame
//!
//! Meshes are in the **render frame**: millimetres, right-handed, with
//!
//! | Axis | World direction |
//! |---|---|
//! | `+x` | `right` (`across`) |
//! | `+y` | `up` |
//! | `+z` | `front` (toward the viewer: minus `deep`) |
//!
//! This is glTF's convention (in millimetres instead of metres) and the
//! right-handed basis [`Orientation::is_reflection`] measures against, so a
//! part's frame maps straight across.
//!
//! A viewer draws parts from meshes in their **own frame** instead
//! ([`cut_part_local`], [`part_local`]), placed by [`rest`]: the mesh
//! then depends only on the part's cuts, size, grain and [`Handedness`],
//! so parts alike in those share one mesh and a part that moves keeps
//! its mesh.
//!
//! # What is drawn
//!
//! - **Boards** with their cuts made ([`cut_part`] of a [`solid::Solid`]):
//!   the boundary of the solid's blocks, with holes, round tenons and round
//!   sections. [`part`] draws the plain cut box or drawn box ([`Extent`])
//!   without cuts.
//! - **Round stock** is a cylinder along the length; **turned parts** are
//!   lathe meshes of their profile, with smooth normals except at shoulders
//!   and sharp corners.
//! - **Hardware** is its members ([`body`]): exactly the shapes its kind
//!   declares (boxes, cylinders, prisms and bars), the same shapes the
//!   checks see. Nothing is drawn that is not declared, so nothing drawn
//!   goes unchecked (grammar, "One collision model").
//!
//! Mirrored parts have left-handed frames. Every triangle is wound so its
//! geometric normal agrees with the outward normal it was built from, so
//! winding is correct for both handednesses without a special case.

#![forbid(unsafe_code)]
#![warn(clippy::redundant_clone, clippy::needless_pass_by_value)]
#![warn(missing_docs)]

mod axes;
mod build;
pub mod clearance;
pub mod collide;
pub mod glb;
pub mod line;
pub mod mechanism;
mod mesh;
mod mesh_cut;
mod mesh_drill;
mod mesh_prism;
mod piece;
pub mod planar;
mod plane;
mod prism;
pub mod render;
mod sample;
pub mod sequence;
pub mod solid;
pub mod sweep;
pub mod wall;
pub mod whole;

use glam::{Mat4, Vec3};
use stave_lang::placement::Axis;
use stave_lang::units::Length;
use stave_model::solved::{Body, MemberShape, Part, Point, Shape, Size3};
use stave_schema::words::Grain;

pub use build::LATHE_SEGMENTS;

/// One vertex of a mesh.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Vertex {
    /// Position in the render frame, millimetres.
    pub position: [f32; 3],
    /// Unit outward normal.
    pub normal: [f32; 3],
    /// Position in the part's own frame, millimetres, permuted so `x` runs
    /// along the grain, then the other face axis, then the thickness. A
    /// wood shader draws grain from it; hardware leaves it at zero.
    pub grain: [f32; 3],
    /// 1 on a face a cut made (the walls and bottom of a hole, mortise,
    /// dado, rabbet or groove, a tenon's cheeks and shoulders, a round
    /// tenon), 0 on the stock's own surface, so a viewer can tint the
    /// joinery.
    pub cut: f32,
}

/// A triangle mesh. Triangles are counter-clockwise seen from outside.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Mesh {
    /// The vertices.
    pub vertices: Vec<Vertex>,
    /// Three indices per triangle.
    pub indices: Vec<u32>,
}

impl Mesh {
    /// The number of triangles.
    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }

    /// The triangles as vertex positions.
    pub fn triangles(&self) -> impl Iterator<Item = [Vec3; 3]> + '_ {
        let (triangles, _) = self.indices.as_chunks::<3>();
        triangles.iter().filter_map(|&[a, b, c]| {
            let corner = |index: u32| {
                usize::try_from(index)
                    .ok()
                    .and_then(|index| self.vertices.get(index))
                    .map(|vertex| Vec3::from(vertex.position))
            };
            Some([corner(a)?, corner(b)?, corner(c)?])
        })
    }

    /// The box around every vertex; `None` for an empty mesh.
    pub fn bounds(&self) -> Option<Aabb> {
        Aabb::around(
            self.vertices
                .iter()
                .map(|vertex| Vec3::from(vertex.position)),
        )
    }

    /// The enclosed volume in cubic millimetres: positive when the
    /// triangles face outward.
    pub fn signed_volume(&self) -> f32 {
        self.triangles()
            .map(|[a, b, c]| a.dot(b.cross(c)) / 6.0)
            .sum()
    }
}

/// An axis-aligned box in the render frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Aabb {
    /// The low corner.
    pub min: Vec3,
    /// The high corner.
    pub max: Vec3,
}

impl Aabb {
    /// The box around some points; `None` when there are none.
    pub fn around(points: impl IntoIterator<Item = Vec3>) -> Option<Aabb> {
        let mut points = points.into_iter();
        let first = points.next()?;
        Some(points.fold(
            Aabb {
                min: first,
                max: first,
            },
            |bounds, point| Aabb {
                min: bounds.min.min(point),
                max: bounds.max.max(point),
            },
        ))
    }

    /// The box around both.
    pub fn union(self, other: Aabb) -> Aabb {
        Aabb {
            min: self.min.min(other.min),
            max: self.max.max(other.max),
        }
    }

    /// The middle.
    pub fn centre(&self) -> Vec3 {
        (self.min + self.max) * 0.5
    }

    /// The size along each axis.
    pub fn size(&self) -> Vec3 {
        self.max - self.min
    }

    /// The eight corners.
    pub fn corners(&self) -> [Vec3; 8] {
        let (a, b) = (self.min, self.max);
        [
            Vec3::new(a.x, a.y, a.z),
            Vec3::new(b.x, a.y, a.z),
            Vec3::new(a.x, b.y, a.z),
            Vec3::new(b.x, b.y, a.z),
            Vec3::new(a.x, a.y, b.z),
            Vec3::new(b.x, a.y, b.z),
            Vec3::new(a.x, b.y, b.z),
            Vec3::new(b.x, b.y, b.z),
        ]
    }

    /// The box around this box moved by a transform.
    pub fn transformed(&self, transform: Mat4) -> Aabb {
        let corners = self
            .corners()
            .map(|corner| transform.transform_point3(corner));
        Aabb::around(corners).unwrap_or(*self)
    }

    /// Where a ray enters the box: the distance along `direction` (in units
    /// of its length), or `None` if it misses. A ray starting inside hits
    /// at 0.
    pub fn ray_entry(&self, origin: Vec3, direction: Vec3) -> Option<f32> {
        let mut near = 0.0_f32;
        let mut far = f32::INFINITY;
        for axis in 0..3 {
            let (o, d, low, high) = (
                origin[axis],
                direction[axis],
                self.min[axis],
                self.max[axis],
            );
            if d.abs() < 1e-12 {
                if o < low || o > high {
                    return None;
                }
                continue;
            }
            let (t0, t1) = ((low - o) / d, (high - o) / d);
            let (t0, t1) = if t0 <= t1 { (t0, t1) } else { (t1, t0) };
            near = near.max(t0);
            far = far.min(t1);
            if near > far {
                return None;
            }
        }
        Some(near)
    }
}

/// Feature edges: the lines a viewer outlines a shape with (box edges,
/// shoulders and ends of turned parts).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Edges {
    /// Line segments, two points each, in the render frame.
    pub segments: Vec<[[f32; 3]; 2]>,
}

/// A part or envelope ready to draw.
#[derive(Debug, Clone, PartialEq)]
pub struct Geometry {
    /// The surface.
    pub mesh: Mesh,
    /// The outline.
    pub edges: Edges,
    /// The box around the mesh.
    pub bounds: Aabb,
}

/// Which of a part's two boxes to build a board from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Extent {
    /// The part as cut from stock, with what joints add (a tenon, the
    /// tongue in a dado). [`part`] draws the cut box; a scene graph built
    /// with this extent draws every part with its cuts made
    /// ([`cut_part`]), so tenons sit in their mortises.
    #[default]
    Cut,
    /// The drawn box: the sizes as written, so parts meet face to face.
    Drawn,
}

/// The mesh of a part.
///
/// Boards are boxes of the chosen [`Extent`]; round and turned parts are
/// lathe meshes along the part's length, centred on its width and
/// thickness. A turned part's profile is drawn over its drawn length (the
/// stations); with [`Extent::Cut`], what a joint adds to either end is
/// drawn as a cylinder of the end station's diameter.
pub fn part(part: &Part, extent: Extent) -> Geometry {
    part_in(&build::Frame::of_part(part), part, extent)
}

/// [`part`] in the part's own frame ([`Handedness`]), to be placed by
/// [`rest`]: two parts alike in shape, size, grain and handedness get the
/// same mesh wherever they are.
pub fn part_local(part: &Part, extent: Extent) -> Geometry {
    part_in(&build::Frame::own(handedness(part)), part, extent)
}

/// Which way round a part's own frame is: right-handed like the render
/// frame, or left-handed (a mirrored part, [`Orientation::is_reflection`]).
///
/// A mesh in the part's own frame ([`part_local`], [`cut_part_local`]) is
/// built in a frame of the same handedness, so [`rest`] only turns and
/// moves it and never reflects it (which would turn its triangles inside
/// out).
///
/// [`Orientation::is_reflection`]: stave_model::solved::Orientation::is_reflection
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Handedness {
    /// Length, width and thickness turn like right, up and front.
    Right,
    /// A mirror image: they turn the other way.
    Left,
}

/// The handedness of a part's frame.
pub fn handedness(part: &Part) -> Handedness {
    if part.pose.orientation.is_reflection() {
        Handedness::Left
    } else {
        Handedness::Right
    }
}

/// Where a part's own-frame mesh ([`part_local`], [`cut_part_local`]) sits
/// at rest: the transform from its own frame to the render frame. It is a
/// rotation by quarter turns and a move, never a reflection.
pub fn rest(part: &Part) -> Mat4 {
    build::Frame::own(handedness(part)).onto(&build::Frame::of_part(part))
}

fn part_in(frame: &build::Frame, part: &Part, extent: Extent) -> Geometry {
    let frame = *frame;
    let grain = match part.grain {
        Grain::Length => build::GrainAxes::LENGTH,
        Grain::Width => build::GrainAxes::WIDTH,
    };
    match &part.shape {
        Shape::Board => {
            let (offset, size) = match extent {
                Extent::Cut => (Size3::default(), part.cut),
                Extent::Drawn => (part.drawn_offset, part.drawn),
            };
            build::cuboid(&frame, render::size(offset), render::size(size), grain)
        }
        Shape::Round { diameter } => {
            let (start, length) = match extent {
                Extent::Cut => (0.0, render::mm(part.cut.length)),
                Extent::Drawn => (
                    render::mm(part.drawn_offset.length),
                    render::mm(part.drawn.length),
                ),
            };
            let radius = render::mm(*diameter) * 0.5;
            let profile = [
                build::ProfilePoint::new(start, radius),
                build::ProfilePoint::new(start + length, radius),
            ];
            build::lathe(
                &frame,
                centre_of(part),
                &profile,
                build::Corners::Sharp,
                grain,
            )
        }
        Shape::Turned {
            stations, smooth, ..
        } => {
            let start = render::mm(part.drawn_offset.length);
            let mut profile: Vec<build::ProfilePoint> = stations
                .iter()
                .map(|station| {
                    build::ProfilePoint::new(
                        start + render::mm(station.at),
                        render::mm(station.diameter) * 0.5,
                    )
                })
                .collect();
            let corners = if *smooth {
                profile = build::smooth_profile(&profile);
                build::Corners::ByAngle
            } else {
                build::Corners::Sharp
            };
            if extent == Extent::Cut {
                extend_ends(&mut profile, render::mm(part.cut.length));
            }
            build::lathe(&frame, centre_of(part), &profile, corners, grain)
        }
    }
}

/// The mesh of a part with its cuts made: tenons, mortises, dados,
/// rabbets, grooves, holes and round tenons show. `solid` is
/// [`solid::Solid::of`] the same part.
pub fn cut_part(part: &Part, solid: &solid::Solid) -> Geometry {
    let grain = match part.grain {
        Grain::Length => build::GrainAxes::LENGTH,
        Grain::Width => build::GrainAxes::WIDTH,
    };
    mesh::cut_part(&build::Frame::of_part(part), part, solid, grain)
}

/// [`cut_part`] in the part's own frame, to be placed by [`rest`].
///
/// The cuts of a [`solid::Solid`] are already in the part's own frame
/// (they are measured from its corner along its length, width and
/// thickness), so this mesh depends only on the solid, the part's cut
/// size, its grain and its [`Handedness`], never on where the part is:
/// two parts alike in those get the same mesh, and a part that only moved
/// keeps its mesh.
pub fn cut_part_local(part: &Part, solid: &solid::Solid) -> Geometry {
    let grain = match part.grain {
        Grain::Length => build::GrainAxes::LENGTH,
        Grain::Width => build::GrainAxes::WIDTH,
    };
    mesh::cut_part(&build::Frame::own(handedness(part)), part, solid, grain)
}

/// Lengthens a profile to cover `0 ..= length` with cylinders of its end
/// radii.
fn extend_ends(profile: &mut Vec<build::ProfilePoint>, length: f32) {
    if let Some(first) = profile.first().copied()
        && first.along > 0.0
    {
        profile.insert(0, build::ProfilePoint::new(0.0, first.radius));
    }
    if let Some(last) = profile.last().copied()
        && last.along < length
    {
        profile.push(build::ProfilePoint::new(length, last.radius));
    }
}

/// The lathe axis of a part: the middle of its width and thickness.
fn centre_of(part: &Part) -> [f32; 2] {
    [
        render::mm(part.cut.width) * 0.5,
        render::mm(part.cut.thickness) * 0.5,
    ]
}

/// The mesh of a member of a piece of hardware at rest: its shapes, each a
/// box, a cylinder, or an outline extruded along its axis.
pub fn body(body: &Body) -> Geometry {
    mesh::merge(body.shapes().iter().map(member_shape).collect())
}

/// The render-frame point of a world point given on an axis and the two
/// others (in [`Axis::others`] order).
fn axis_point(axis: Axis, at: Length, [a, b]: [Length; 2]) -> glam::Vec3 {
    let [first, second] = axis.others();
    render::point(
        Point::default()
            .with(axis, at)
            .with(first, a)
            .with(second, b),
    )
}

fn member_shape(shape: &MemberShape) -> Geometry {
    match shape {
        MemberShape::Box(region) => {
            let frame = build::Frame::new(
                render::point(region.min()),
                [Axis::Across, Axis::Up, Axis::Deep].map(render::axis),
            );
            let size = glam::Vec3::new(
                render::mm(region.size(Axis::Across)),
                render::mm(region.size(Axis::Up)),
                render::mm(region.size(Axis::Deep)),
            );
            build::cuboid(&frame, glam::Vec3::ZERO, size, build::GrainAxes::LENGTH)
        }
        MemberShape::Cylinder {
            axis,
            centre,
            diameter,
            start,
            end,
        } => {
            let [first, second] = axis.others();
            let frame = build::Frame::new(
                axis_point(*axis, *start, *centre),
                [*axis, first, second].map(render::axis),
            );
            let radius = render::mm(*diameter) * 0.5;
            let length = render::mm(Length::from_nm(end.nm().saturating_sub(start.nm())));
            build::lathe(
                &frame,
                [0.0, 0.0],
                &[
                    build::ProfilePoint::new(0.0, radius),
                    build::ProfilePoint::new(length, radius),
                ],
                build::Corners::Sharp,
                build::GrainAxes::LENGTH,
            )
        }
        MemberShape::Slanted { diameter, .. } => {
            let Some((from, to)) = shape.slanted_ends() else {
                return build::Builder::default().finish();
            };
            let (start, end) = (render::point(from), render::point(to));
            slanted_cylinder(start, end, render::mm(*diameter) * 0.5)
        }
        MemberShape::Prism {
            axis,
            points,
            start,
            end,
        } => extrude(*axis, points, *start, *end),
        MemberShape::Bar {
            axis,
            from,
            to,
            thickness,
            start,
            end,
        } => {
            let bar = collide::WorldPrism::bar(*axis, *from, *to, *thickness, *start, *end);
            extrude(*axis, &bar.corners, *start, *end)
        }
    }
}

/// A cylinder between two render-frame points: a lathe in a frame whose
/// first axis runs from one to the other.
pub(crate) fn slanted_cylinder(start: glam::Vec3, end: glam::Vec3, radius: f32) -> Geometry {
    let along = end - start;
    let length = along.length();
    let first = along.normalize_or_zero();
    if length <= 0.0 || first == glam::Vec3::ZERO {
        return build::Builder::default().finish();
    }
    let second = first.any_orthonormal_vector();
    let third = first.cross(second);
    let frame = build::Frame::new(start, [first, second, third]);
    build::lathe(
        &frame,
        [0.0, 0.0],
        &[
            build::ProfilePoint::new(0.0, radius),
            build::ProfilePoint::new(length, radius),
        ],
        build::Corners::Sharp,
        build::GrainAxes::LENGTH,
    )
}

/// A convex outline on the plane across `axis`, extruded from `start` to
/// `end`, with flat faces and its edges.
fn extrude(axis: Axis, points: &[[Length; 2]], start: Length, end: Length) -> Geometry {
    let mut builder = build::Builder::default();
    let low: Vec<glam::Vec3> = points.iter().map(|p| axis_point(axis, start, *p)).collect();
    let high: Vec<glam::Vec3> = points.iter().map(|p| axis_point(axis, end, *p)).collect();
    let middle = low.iter().chain(&high).copied().sum::<glam::Vec3>()
        / f32::from(u16::try_from(low.len() * 2).unwrap_or(u16::MAX).max(1));
    let along = render::axis(axis);
    let grain = [0.0; 3];
    // Ends.
    for (ring, normal) in [(&low, -along), (&high, along)] {
        let indices: Vec<u32> = ring
            .iter()
            .map(|p| builder.vertex(*p, normal, grain))
            .collect();
        if let Some((first, rest)) = indices.split_first() {
            for pair in rest.windows(2) {
                if let [b, c] = pair {
                    builder.triangle(*first, *b, *c);
                }
            }
        }
    }
    // Sides, each facing away from the middle.
    let next = low.iter().zip(&high).cycle().skip(1);
    for ((a, b), (c, d)) in low.iter().zip(&high).zip(next) {
        let face_middle = (*a + *b + *c + *d) * 0.25;
        let edge = *c - *a;
        let mut normal = edge.cross(along).normalize_or_zero();
        if normal.dot(face_middle - middle) < 0.0 {
            normal = -normal;
        }
        let quad = [*a, *c, *d, *b].map(|p| builder.vertex(p, normal, grain));
        builder.triangle(quad[0], quad[1], quad[2]);
        builder.triangle(quad[0], quad[2], quad[3]);
        builder.edge(*a, *c);
        builder.edge(*b, *d);
        builder.edge(*a, *b);
    }
    builder.finish()
}

#[cfg(test)]
mod tests;
