//! Ray picking: bounding boxes first, then triangles. A hit on a part
//! also says which of its six faces the ray went in through.

use glam::Vec3;
use stave_lang::placement::Face;
use stave_model::solved::PartId;

use crate::camera::Ray;
use crate::graph::{Item, NodeId, Posed, SceneGraph};

/// What a ray hit.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hit {
    /// The node.
    pub node: NodeId,
    /// What it draws.
    pub item: Item,
    /// Distance from the ray's origin, millimetres.
    pub distance: f32,
    /// For a part, the face hit: one of the six faces of its drawn box,
    /// named by the world direction it faces as the design places it
    /// (grammar, "Directions, axes and faces"), whatever pose the scene
    /// shows it in. `None` for hardware and joint members.
    pub face: Option<Face>,
}

impl Hit {
    /// The part and face hit, when the hit is on a part.
    pub fn part_face(&self) -> Option<PartFace> {
        match self.item {
            Item::Part(part) => Some(PartFace {
                part,
                face: self.face?,
            }),
            Item::Hardware(_) | Item::Member(_) => None,
        }
    }
}

/// A face of a part: what a click on a part gives the editor (an anchor,
/// `side_l.right`, and its part).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PartFace {
    /// The part.
    pub part: PartId,
    /// Which of its six faces.
    pub face: Face,
}

/// How far apart two distances along a ray may be and still count as the
/// same place, millimetres: well under anything a woodworker measures,
/// well over `f32` rounding across a few metres.
pub(crate) const TOLERANCE: f32 = 0.01;

/// Where a ray crosses a triangle (Möller–Trumbore), either side.
fn triangle_hit(origin: Vec3, direction: Vec3, [a, b, c]: [Vec3; 3]) -> Option<f32> {
    let (edge1, edge2) = (b - a, c - a);
    let p = direction.cross(edge2);
    let determinant = edge1.dot(p);
    if determinant.abs() < 1e-9 {
        return None;
    }
    let inverse = 1.0 / determinant;
    let s = origin - a;
    let u = s.dot(p) * inverse;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = s.cross(edge1);
    let v = direction.dot(q) * inverse;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    let t = edge2.dot(q) * inverse;
    (t >= 0.0).then_some(t)
}

/// The face a render-frame direction points along most, so a normal
/// tipped a little still names the face it is nearest. `None` for a zero
/// or non-finite vector.
fn face_along(vector: Vec3) -> Option<Face> {
    let magnitude = vector.abs();
    if !vector.is_finite() || magnitude.max_element() <= 0.0 {
        return None;
    }
    // Ties go to the earlier axis: x, then y, then z.
    Some(
        if magnitude.x >= magnitude.y && magnitude.x >= magnitude.z {
            if vector.x > 0.0 {
                Face::Right
            } else {
                Face::Left
            }
        } else if magnitude.y >= magnitude.z {
            if vector.y > 0.0 {
                Face::Top
            } else {
                Face::Bottom
            }
        } else if vector.z > 0.0 {
            // +z runs toward the viewer.
            Face::Front
        } else {
            Face::Back
        },
    )
}

/// Where a ray goes into a box and through which face: `None` when it
/// misses, starts inside, or the box lies behind it.
fn entry_face(bounds: crate::Aabb, origin: Vec3, direction: Vec3) -> Option<(f32, Face)> {
    let mut near = f32::NEG_INFINITY;
    let mut far = f32::INFINITY;
    let mut face = None;
    for (index, (low_face, high_face)) in [
        (Face::Left, Face::Right),
        (Face::Bottom, Face::Top),
        (Face::Back, Face::Front),
    ]
    .into_iter()
    .enumerate()
    {
        let (o, d, low, high) = (
            origin[index],
            direction[index],
            bounds.min[index],
            bounds.max[index],
        );
        if d.abs() < 1e-12 {
            if o < low || o > high {
                return None;
            }
            continue;
        }
        // Going up the axis the ray enters through the low face.
        let (t0, t1, entered) = if d > 0.0 {
            ((low - o) / d, (high - o) / d, low_face)
        } else {
            ((high - o) / d, (low - o) / d, high_face)
        };
        if t0 > near {
            near = t0;
            face = Some(entered);
        }
        far = far.min(t1);
    }
    if near <= far && near >= 0.0 {
        face.map(|face| (near, face))
    } else {
        None
    }
}

impl SceneGraph {
    /// The nearest shown node a ray hits, and for a part the face it hits.
    pub fn pick(&self, posed: &Posed, ray: Ray) -> Option<Hit> {
        let mut best: Option<(Hit, Vec3, Vec3, Vec3)> = None;
        for id in self.node_ids() {
            let (Some(transform), Some(node)) = (posed.shown(id), self.node(id)) else {
                continue;
            };
            // Into the node's rest frame, and on into its mesh's own
            // frame; affine maps keep the ray parameter, so distances
            // compare directly.
            let inverse = transform.inverse();
            let origin = inverse.transform_point3(ray.origin);
            let direction = inverse.transform_vector3(ray.direction);
            let own = node.rest.inverse();
            let (local_origin, local_direction) = (
                own.transform_point3(origin),
                own.transform_vector3(direction),
            );
            let Some(entry) = node.local.bounds.ray_entry(local_origin, local_direction) else {
                continue;
            };
            if best.is_some_and(|(hit, ..)| entry > hit.distance) {
                continue;
            }
            let nearest = node
                .local
                .mesh
                .triangles()
                .filter_map(|triangle| {
                    triangle_hit(local_origin, local_direction, triangle).map(|t| (t, triangle))
                })
                .reduce(|a, b| if b.0 < a.0 { b } else { a });
            if let Some((distance, [a, b, c])) = nearest
                && best.is_none_or(|(hit, ..)| distance < hit.distance)
            {
                // The surface's normal in the rest frame: `rest` turns and
                // moves, never reflects.
                let normal = node.rest.transform_vector3((b - a).cross(c - a));
                best = Some((
                    Hit {
                        node: id,
                        item: node.item,
                        distance,
                        face: None,
                    },
                    origin,
                    direction,
                    normal,
                ));
            }
        }
        let (mut hit, origin, direction, normal) = best?;
        hit.face = self.face_hit(hit, origin, direction, normal);
        Some(hit)
    }

    /// The face of a part a ray hit, in the part's rest frame: the face of
    /// its drawn box the ray goes in through, when the surface hit lies
    /// on or inside that box (a dado's floor is still the face it is cut
    /// into); otherwise (the ray starts inside the box, or hits something
    /// a joint adds beyond it, a tenon) the face the surface's normal
    /// points along most, turned toward the ray.
    fn face_hit(&self, hit: Hit, origin: Vec3, direction: Vec3, normal: Vec3) -> Option<Face> {
        let drawn = self.drawn_box(hit.node)?;
        let bounds = stave_geom::render::world_box(drawn);
        if let Some((entry, face)) = entry_face(bounds, origin, direction)
            && hit.distance + TOLERANCE >= entry
        {
            return Some(face);
        }
        let toward = if normal.dot(direction) > 0.0 {
            -normal
        } else {
            normal
        };
        face_along(toward)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Aabb;

    fn unit() -> Aabb {
        Aabb {
            min: Vec3::ZERO,
            max: Vec3::ONE,
        }
    }

    #[test]
    fn a_ray_enters_through_the_face_it_meets_first() {
        let from = |origin: Vec3, direction: Vec3| entry_face(unit(), origin, direction);
        let half = Vec3::splat(0.5);
        assert_eq!(
            from(half + Vec3::Z * 5.0, Vec3::NEG_Z),
            Some((4.5, Face::Front))
        );
        assert_eq!(from(half - Vec3::Z * 5.0, Vec3::Z), Some((4.5, Face::Back)));
        assert_eq!(
            from(half + Vec3::Y * 2.0, Vec3::NEG_Y),
            Some((1.5, Face::Top))
        );
        assert_eq!(from(half - Vec3::X * 2.0, Vec3::X), Some((1.5, Face::Left)));
        // Inside, behind, or beside: no face.
        assert_eq!(from(half, Vec3::X), None);
        assert_eq!(from(half + Vec3::X * 3.0, Vec3::X), None);
        assert_eq!(from(Vec3::new(5.0, 0.5, 5.0), Vec3::NEG_Z), None);
    }

    #[test]
    fn a_normal_names_the_face_it_points_along_most() {
        assert_eq!(face_along(Vec3::new(0.2, -1.0, 0.1)), Some(Face::Bottom));
        assert_eq!(face_along(Vec3::new(0.0, 0.0, 3.0)), Some(Face::Front));
        assert_eq!(face_along(Vec3::new(-2.0, 1.0, 0.0)), Some(Face::Left));
        assert_eq!(face_along(Vec3::ZERO), None);
        assert_eq!(face_along(Vec3::new(f32::NAN, 0.0, 0.0)), None);
    }
}
