//! Ray picking: bounding boxes first, then triangles.

use glam::Vec3;

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
}

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

impl SceneGraph {
    /// The nearest shown node a ray hits.
    pub fn pick(&self, posed: &Posed, ray: Ray) -> Option<Hit> {
        let mut best: Option<Hit> = None;
        for id in self.node_ids() {
            let (Some(transform), Some(node)) = (posed.shown(id), self.node(id)) else {
                continue;
            };
            // Into the node's rest frame; affine maps keep the ray
            // parameter, so distances compare directly.
            let inverse = transform.inverse();
            let origin = inverse.transform_point3(ray.origin);
            let direction = inverse.transform_vector3(ray.direction);
            let Some(entry) = node.geometry.bounds.ray_entry(origin, direction) else {
                continue;
            };
            if best.is_some_and(|hit| entry > hit.distance) {
                continue;
            }
            let nearest = node
                .geometry
                .mesh
                .triangles()
                .filter_map(|triangle| triangle_hit(origin, direction, triangle))
                .reduce(f32::min);
            if let Some(distance) = nearest
                && best.is_none_or(|hit| distance < hit.distance)
            {
                best = Some(Hit {
                    node: id,
                    item: node.item,
                    distance,
                });
            }
        }
        best
    }
}
