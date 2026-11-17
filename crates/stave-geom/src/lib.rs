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
//! # What is drawn
//!
//! - **Boards** with their cuts made ([`cut_part`] of a [`solid::Solid`]):
//!   the boundary of the solid's blocks, with holes, round tenons and round
//!   sections. [`part`] draws the plain cut box or drawn box ([`Extent`])
//!   without cuts.
//! - **Round stock** is a cylinder along the length; **turned parts** are
//!   lathe meshes of their profile, with smooth normals except at shoulders
//!   and sharp corners.
//! - **Hardware** is its envelope: a box, or a cylinder standing off the
//!   mount face (a hinge's cup is sunk into it); hinges and slides also
//!   have a member fixed to the carcass ([`hardware_fixed`]). A cup hinge
//!   is drawn as its mechanism ([`hinge`]): the steel cup with its rim and
//!   boss pins, the plate, arm and arm pins, and two links
//!   ([`hardware_links`]).
//!
//! Mirrored parts have left-handed frames. Every triangle is wound so its
//! geometric normal agrees with the outward normal it was built from, so
//! winding is correct for both handednesses without a special case.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod build;
pub mod clearance;
pub mod collide;
pub mod hinge;
pub mod linkage;
mod mesh;
mod mesh_cut;
mod mesh_drill;
mod mesh_prism;
mod piece;
pub mod planar;
mod prism;
pub mod render;
mod sample;
pub mod sequence;
pub mod solid;
pub mod sweep;

use glam::{Mat4, Vec3};
use stave_model::solved::{Envelope, Hardware, Part, Shape, Size3};
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
    let frame = build::Frame::of_part(part);
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
    mesh::cut_part(part, solid, grain)
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

/// The mesh of the member of a piece of hardware on its piece: the whole
/// of a pull, a hinge's cup (with a cup hinge's rim and boss pins), a
/// slide's drawer member.
pub fn hardware(hardware: &Hardware) -> Geometry {
    match hinge::mechanism(hardware) {
        Some(mechanism) => mechanism.cup,
        None => envelope(&hardware.envelope, &hardware.pose),
    }
}

/// The mesh of the member fixed to the carcass (a hinge's plate, with a
/// cup hinge's arm and arm pins; a slide's cabinet member), when the
/// hardware has one.
pub fn hardware_fixed(hardware: &Hardware) -> Option<Geometry> {
    if let Some(mechanism) = hinge::mechanism(hardware) {
        return Some(mechanism.fixed);
    }
    hardware
        .fixed
        .as_ref()
        .map(|member| envelope(&member.envelope, &member.pose))
}

/// The meshes of a cup hinge's two links at rest; `None` for any other
/// hardware.
pub fn hardware_links(hardware: &Hardware) -> Option<[Geometry; 2]> {
    hinge::mechanism(hardware).map(|mechanism| mechanism.links)
}

/// A box or a cylinder (along the local thickness) at a pose.
fn envelope(envelope: &Envelope, pose: &stave_model::solved::Pose) -> Geometry {
    let frame = build::Frame::of_pose(pose);
    match *envelope {
        Envelope::Box { .. } => build::cuboid(
            &frame,
            glam::Vec3::ZERO,
            render::size(envelope.size()),
            build::GrainAxes::LENGTH,
        ),
        Envelope::Cylinder { diameter, length } => {
            // The cylinder's own axis is the local thickness: turn the frame
            // so the lathe's axis (its first axis) is the thickness.
            let radius = render::mm(diameter) * 0.5;
            let axial = frame.thickness_first();
            let profile = [
                build::ProfilePoint::new(0.0, radius),
                build::ProfilePoint::new(render::mm(length), radius),
            ];
            build::lathe(
                &axial,
                [radius, radius],
                &profile,
                build::Corners::Sharp,
                build::GrainAxes::LENGTH,
            )
        }
    }
}

#[cfg(test)]
mod tests;
