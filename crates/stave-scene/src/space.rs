//! Picking the space between parts: the region a ray looks into, and the
//! two parts that face each other across the gap it passes through.
//!
//! Both work on what the scene shows. A region stays where the design
//! puts it (motions don't move it); a facing pair is found among the
//! shown parts that are where the design puts them (a part a scene
//! swings open or pulls apart no longer faces anything), and its clear
//! distance is exact, measured between drawn boxes.

use glam::Vec3;
use stave_lang::placement::{Axis, Face};
use stave_lang::units::Length;
use stave_model::solved::{PartId, Point, RegionId, WorldBox};

use crate::Aabb;
use crate::camera::Ray;
use crate::graph::{Item, Posed, SceneGraph};
use crate::pick::{Hit, PartFace, TOLERANCE};

/// A region a ray looks into.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RegionHit {
    /// The region.
    pub region: RegionId,
    /// Where the ray goes into it, millimetres from the ray's origin (0
    /// when the ray starts inside).
    pub distance: f32,
}

/// What is under the pointer: a node (a part, with the face hit), or the
/// empty space of a region.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Target {
    /// A node: nothing but empty space outside any region lies in front
    /// of it, or it stands inside the region the ray looks into.
    Node(Hit),
    /// A region the ray passes through empty space inside, ending at its
    /// wall or beyond.
    Region(RegionHit),
}

/// Two parts that face each other across a gap along one axis, with
/// nothing between them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Facing {
    /// The axis the gap runs along.
    pub axis: Axis,
    /// The part on the low side (the left, the bottom or the front), with
    /// its face toward the gap (`right`, `top` or `back`).
    pub first: PartFace,
    /// The part on the high side, with its face toward the gap (`left`,
    /// `bottom` or `front`).
    pub second: PartFace,
    /// The clear distance between the two faces, exact.
    pub clear: Length,
}

/// Where a ray is inside a box: the distances it goes in and comes out,
/// the first never below 0. `None` when it misses or the box is behind
/// it.
fn span(bounds: Aabb, origin: Vec3, direction: Vec3) -> Option<(f32, f32)> {
    let mut near = 0.0_f32;
    let mut far = f32::INFINITY;
    for axis in 0..3 {
        let (o, d, low, high) = (
            origin[axis],
            direction[axis],
            bounds.min[axis],
            bounds.max[axis],
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
    far.is_finite().then_some((near, far))
}

/// `true` when a point lies inside a box by more than [`TOLERANCE`] on
/// every axis: inside the space, not on its walls.
fn well_inside(bounds: Aabb, point: Vec3) -> bool {
    (0..3).all(|axis| {
        point[axis] > bounds.min[axis] + TOLERANCE && point[axis] < bounds.max[axis] - TOLERANCE
    })
}

/// The product of a box's sizes, to say which of two regions is the
/// smaller.
fn volume(bounds: Aabb) -> f32 {
    let size = bounds.size();
    size.x * size.y * size.z
}

/// `true` when `low < value < high`.
fn strictly_between(value: Length, low: Length, high: Length) -> bool {
    low < value && value < high
}

/// The faces of the low and the high part across a gap along an axis.
fn facing_faces(axis: Axis) -> (Face, Face) {
    match axis {
        Axis::Up => (Face::Top, Face::Bottom),
        Axis::Across => (Face::Right, Face::Left),
        // Deep runs toward the back: the low part is in front.
        Axis::Deep => (Face::Back, Face::Front),
    }
}

impl SceneGraph {
    /// The region a ray looks into: of the regions it goes into through
    /// empty space (before any shown node, by more than a hundredth of a
    /// millimetre, so a part in front of a region, or in its front face,
    /// wins), the innermost one under the pointer. That is the one the ray
    /// is still inside where its run through empty space ends (at the
    /// node it hits, or where it leaves the last region), and of those the
    /// smallest; ties go to the lower id. Regions with no room inside (no
    /// size on some axis) are never picked.
    pub fn pick_region(&self, posed: &Posed, ray: Ray) -> Option<RegionHit> {
        let stop = self.pick(posed, ray).map(|hit| hit.distance);
        self.region_before(ray, stop)
    }

    /// What a ray points at: the node it hits, unless it first passes
    /// through empty space inside a region and the node is that region's
    /// wall or lies beyond it, when it is the region (see
    /// [`SceneGraph::pick_region`]). A node that stands inside the region
    /// (a rail across a bay) is what the pointer is on.
    pub fn pick_target(&self, posed: &Posed, ray: Ray) -> Option<Target> {
        let hit = self.pick(posed, ray);
        let region = self.region_before(ray, hit.map(|hit| hit.distance));
        match (hit, region) {
            (Some(hit), Some(region)) => {
                let point = ray.origin + ray.direction * hit.distance;
                let inside = self
                    .region(region.region)
                    .is_some_and(|node| well_inside(node.volume, point));
                Some(if inside {
                    Target::Node(hit)
                } else {
                    Target::Region(region)
                })
            }
            (Some(hit), None) => Some(Target::Node(hit)),
            (None, Some(region)) => Some(Target::Region(region)),
            (None, None) => None,
        }
    }

    /// The region a ray looks into before `stop` (where it hits a node).
    fn region_before(&self, ray: Ray, stop: Option<f32>) -> Option<RegionHit> {
        let candidates: Vec<(RegionId, f32, f32, f32)> = self
            .regions()
            .iter()
            .filter(|region| region.volume.size().min_element() > TOLERANCE)
            .filter_map(|region| {
                let (near, far) = span(region.volume, ray.origin, ray.direction)?;
                if stop.is_some_and(|stop| near + TOLERANCE >= stop) {
                    return None;
                }
                let end = stop.map_or(far, |stop| far.min(stop));
                Some((region.id, near, end, volume(region.volume)))
            })
            .collect();
        let last = candidates
            .iter()
            .map(|(_, _, end, _)| *end)
            .reduce(f32::max)?;
        candidates
            .into_iter()
            .filter(|(_, _, end, _)| *end + TOLERANCE >= last)
            .reduce(|best, next| {
                if next.3 < best.3 || (next.3 == best.3 && next.0 < best.0) {
                    next
                } else {
                    best
                }
            })
            .map(|(region, distance, _, _)| RegionHit { region, distance })
    }

    /// The two parts facing each other across the gap a ray passes
    /// through: the gap at the middle of the ray's run through empty
    /// space inside the box around everything shown (from where it goes
    /// into that box to the first node it hits, or to where it leaves).
    /// `None` when the ray hits a node before it reaches any empty space,
    /// or no pair faces each other there (see [`SceneGraph::facing_at`]).
    pub fn facing(&self, posed: &Posed, ray: Ray) -> Option<Facing> {
        let bounds = self.bounds(posed)?;
        let (near, far) = span(bounds, ray.origin, ray.direction)?;
        let end = match self.pick(posed, ray) {
            Some(hit) => far.min(hit.distance),
            None => far,
        };
        if near + TOLERANCE >= end {
            return None;
        }
        let middle = ray.origin + ray.direction * ((near + end) * 0.5);
        self.facing_at(posed, stave_geom::render::point_from_render(middle)?)
    }

    /// The two parts facing each other across the gap a point lies in,
    /// with nothing between them: along each axis, the nearest shown part
    /// on either side of the point whose drawn box spans the point on the
    /// other two axes; of the axes with a part on both sides, the one with
    /// the narrowest clear distance (ties in the order up, across, deep).
    /// Only parts the scene shows where the design puts them count.
    /// `None` when the point is inside a part or no axis has a pair.
    pub fn facing_at(&self, posed: &Posed, point: Point) -> Option<Facing> {
        let boxes: Vec<(PartId, WorldBox)> = self
            .node_ids()
            .filter(|id| posed.shown(*id) == Some(glam::Mat4::IDENTITY))
            .filter_map(|id| match self.node(id)?.item {
                Item::Part(part) => Some((part, self.drawn_box(id)?)),
                Item::Hardware(_) | Item::Member(_) => None,
            })
            .collect();
        let inside = |bounds: &WorldBox, axis: Axis| {
            strictly_between(
                point.get(axis),
                bounds.min().get(axis),
                bounds.max().get(axis),
            )
        };
        if boxes
            .iter()
            .any(|(_, bounds)| Axis::ALL.into_iter().all(|axis| inside(bounds, axis)))
        {
            return None;
        }
        let mut best: Option<Facing> = None;
        for axis in Axis::ALL {
            let across = axis.others();
            let spanning = boxes
                .iter()
                .filter(|(_, bounds)| across.into_iter().all(|other| inside(bounds, other)));
            let value = point.get(axis);
            let mut low: Option<(PartId, Length)> = None;
            let mut high: Option<(PartId, Length)> = None;
            for (part, bounds) in spanning {
                let (start, end) = (bounds.min().get(axis), bounds.max().get(axis));
                if end <= value && low.is_none_or(|(_, face)| end > face) {
                    low = Some((*part, end));
                }
                if start >= value && high.is_none_or(|(_, face)| start < face) {
                    high = Some((*part, start));
                }
            }
            let (Some((first, from)), Some((second, to))) = (low, high) else {
                continue;
            };
            let Some(clear) = to.checked_sub(from) else {
                continue;
            };
            if best.is_some_and(|best| best.clear <= clear) {
                continue;
            }
            let (first_face, second_face) = facing_faces(axis);
            best = Some(Facing {
                axis,
                first: PartFace {
                    part: first,
                    face: first_face,
                },
                second: PartFace {
                    part: second,
                    face: second_face,
                },
                clear,
            });
        }
        best
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_ray_spans_a_box_from_where_it_enters_to_where_it_leaves() {
        let bounds = Aabb {
            min: Vec3::ZERO,
            max: Vec3::splat(2.0),
        };
        assert_eq!(
            span(bounds, Vec3::new(1.0, 1.0, 5.0), Vec3::NEG_Z),
            Some((3.0, 5.0))
        );
        assert_eq!(
            span(bounds, Vec3::splat(1.0), Vec3::X),
            Some((0.0, 1.0)),
            "a ray starting inside goes in at 0"
        );
        assert_eq!(span(bounds, Vec3::new(5.0, 1.0, 5.0), Vec3::NEG_Z), None);
        assert!(well_inside(bounds, Vec3::splat(1.0)));
        assert!(!well_inside(bounds, Vec3::new(1.0, 1.0, 2.0)));
    }
}
