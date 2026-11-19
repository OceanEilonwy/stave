//! What covers the thing the user is working on: the nodes between the
//! eye and a region or a part, so a view can draw them see-through (a
//! door in front of the bay behind it).

use glam::Vec3;
use stave_geom::Aabb;
use stave_model::solved::{PartId, RegionId};

use crate::camera::{Orbit, Projection};
use crate::graph::{Node, NodeId, Posed, SceneGraph};
use crate::pick::TOLERANCE;

/// What the user is working on, to see through to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Subject {
    /// A space: an opening, a bay, the box.
    Region(RegionId),
    /// A part.
    Part(PartId),
}

/// How many sight lines per axis are drawn to the subject's box (a grid
/// of 3 × 3 × 3 points: corners, edge middles, face middles, centre).
const SAMPLES: u8 = 3;

/// How far in from its walls the subject's box is sampled, and how far a
/// sight line must pass inside a node's box to count, millimetres: so
/// parts that only touch the subject (the walls of a bay) don't cover it.
const INSET: f32 = 0.5;

/// A box shrunk by `by` on every side (never past its middle).
fn shrunk(bounds: Aabb, by: f32) -> Aabb {
    let half = bounds.size() * 0.5;
    let by = Vec3::splat(by).min(half);
    Aabb {
        min: bounds.min + by,
        max: bounds.max - by,
    }
}

/// Where a segment from `from` to `to` goes into a box, as a fraction of
/// the way (0 when it starts inside); `None` when it misses.
fn entry(bounds: Aabb, from: Vec3, to: Vec3) -> Option<f32> {
    let direction = to - from;
    let mut near = 0.0_f32;
    let mut far = 1.0_f32;
    for axis in 0..3 {
        let (o, d, low, high) = (
            from[axis],
            direction[axis],
            bounds.min[axis],
            bounds.max[axis],
        );
        if d.abs() < 1e-9 {
            if o <= low || o >= high {
                return None;
            }
            continue;
        }
        let (t0, t1) = ((low - o) / d, (high - o) / d);
        let (t0, t1) = if t0 <= t1 { (t0, t1) } else { (t1, t0) };
        near = near.max(t0);
        far = far.min(t1);
        if near >= far {
            return None;
        }
    }
    Some(near)
}

/// The points sight lines are drawn to: a grid over the box.
fn samples(bounds: Aabb) -> Vec<Vec3> {
    let step = |index: u8| f32::from(index) / f32::from(SAMPLES - 1);
    let mut points = Vec::new();
    for i in 0..SAMPLES {
        for j in 0..SAMPLES {
            for k in 0..SAMPLES {
                let t = Vec3::new(step(i), step(j), step(k));
                points.push(bounds.min + bounds.size() * t);
            }
        }
    }
    points
}

impl SceneGraph {
    /// The box the subject fills where a pose puts it: a region's volume
    /// moved with its assembly, or a part's posed box with what it
    /// carries.
    fn subject_bounds(&self, posed: &Posed, subject: Subject) -> Option<Aabb> {
        match subject {
            Subject::Region(region) => self.region_volume(posed, region),
            Subject::Part(part) => self.bounds_of(posed, &self.carried_nodes(part)),
        }
    }

    /// The shown nodes that cover the subject from the camera: those a
    /// sight line from the eye to the subject passes through (by more than
    /// half a millimetre) before it reaches the subject's box. Sight lines
    /// are drawn to a grid of points over the subject, and through a grid
    /// of points over each node. A part standing inside a bay, or a wall the bay only
    /// touches, does not cover it; a door in front of it does, and from
    /// behind the cabinet the back does instead. In a flat view the sight
    /// lines are parallel, along the view. The subject's own nodes (a
    /// part and what it carries) are never listed. In node order; empty
    /// when the subject is unknown or hidden behind nothing.
    pub fn covering(&self, posed: &Posed, camera: &Orbit, subject: Subject) -> Vec<NodeId> {
        let Some(bounds) = self.subject_bounds(posed, subject) else {
            return Vec::new();
        };
        let target = shrunk(bounds, INSET);
        let points = samples(target);
        let reach = (camera.eye() - bounds.centre()).length() + bounds.size().length() * 2.0;
        let eye_for = |point: Vec3| match camera.projection {
            Projection::Perspective => camera.eye(),
            Projection::Flat => point + camera.backward() * reach,
        };
        // Each sight line, and how far along it the subject begins.
        let lines: Vec<(Vec3, Vec3, f32)> = points
            .into_iter()
            .filter_map(|point| {
                let eye = eye_for(point);
                // The eye inside the subject sees it all.
                let begins = entry(bounds, eye, point)?;
                (begins > 0.0).then_some((eye, point, begins))
            })
            .collect();
        let own = |node: &Node| match subject {
            Subject::Part(part) => node.carried_by() == Some(part),
            Subject::Region(_) => false,
        };
        self.node_ids()
            .filter(|id| {
                let (Some(transform), Some(node)) = (posed.shown(*id), self.node(*id)) else {
                    return false;
                };
                if own(node) {
                    return false;
                }
                let solid = shrunk(node.bounds().transformed(transform), INSET);
                if solid.size().min_element() <= 0.0 {
                    return false;
                }
                // A sight line to the subject through the node, or one
                // through the node on to the subject: either way round, so
                // neither a thin node nor a thin subject slips between the
                // samples.
                let to_subject = lines.iter().any(|(eye, point, begins)| {
                    let length = (*point - *eye).length().max(f32::EPSILON);
                    entry(solid, *eye, *point).is_some_and(|at| (at + TOLERANCE / length) < *begins)
                });
                to_subject
                    || samples(solid).into_iter().any(|point| {
                        let eye = eye_for(point);
                        let toward = point - eye;
                        let length = toward.length();
                        if length <= f32::EPSILON {
                            return false;
                        }
                        // On past the point, as far again as the subject
                        // can be.
                        let end = eye + toward * ((length + reach * 2.0) / length);
                        let total = (end - eye).length();
                        entry(bounds, eye, end)
                            .is_some_and(|begins| begins * total > length + TOLERANCE)
                    })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit() -> Aabb {
        Aabb {
            min: Vec3::ZERO,
            max: Vec3::ONE,
        }
    }

    #[test]
    fn a_segment_enters_a_box_part_of_the_way_along() {
        let from = Vec3::new(0.5, 0.5, 3.0);
        let to = Vec3::new(0.5, 0.5, -1.0);
        assert_eq!(entry(unit(), from, to), Some(0.5));
        assert_eq!(entry(unit(), Vec3::splat(0.5), to), Some(0.0));
        assert_eq!(entry(unit(), Vec3::new(3.0, 0.5, 3.0), to), None);
        // Short of the box.
        assert_eq!(entry(unit(), from, Vec3::new(0.5, 0.5, 2.0)), None);
    }

    #[test]
    fn a_box_is_sampled_at_its_corners_middles_and_centre() {
        let points = samples(unit());
        assert_eq!(points.len(), 27);
        assert!(points.contains(&Vec3::ZERO));
        assert!(points.contains(&Vec3::ONE));
        assert!(points.contains(&Vec3::splat(0.5)));
        let tiny = shrunk(unit(), 2.0);
        assert_eq!(tiny.min, Vec3::splat(0.5));
        assert_eq!(tiny.max, Vec3::splat(0.5));
    }
}
