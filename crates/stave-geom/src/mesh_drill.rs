//! Holes drilled in round stock and turned parts.
//!
//! A lathe mesh is a faceted solid of revolution, so a hole can't be cut
//! from a grid the way a board's are. Instead the hole is cut out of the
//! mesh itself:
//!
//! - Every triangle of the surface within the hole's range along its axis
//!   is projected along that axis and the hole's circle (as the same
//!   polygon its wall uses) is subtracted from it exactly: the triangle
//!   minus a convex polygon is a few convex pieces, each fanned. Positions,
//!   normals and grain are interpolated on the triangle, so the opening
//!   lies on the drawn facets.
//! - The wall is cast along the hole's axis at each corner of the polygon,
//!   from where the line enters the drawn surface to the bottom of the
//!   hole (or out the other side), so it meets the opening exactly.
//! - A bottom disc closes a hole that stops inside the wood, and the
//!   opening is outlined.
//!
//! Wall, bottom and opening rim are cut faces. Holes along the lathe's own
//! axis (into an end) work the same way: the end cap is what is clipped.

use glam::{Vec2, Vec3};

use crate::build::{Builder, Frame, GrainAxes, LATHE_SEGMENTS};
use crate::mesh::merge;
use crate::solid::{Rod, across};
use crate::{Geometry, Vertex, render};

/// Lengths below this (millimetres) count as zero.
const EPSILON: f32 = 1e-4;

/// The slot of a part axis in a local vector.
fn slot(axis: stave_lang::placement::PartAxis) -> usize {
    use stave_lang::placement::PartAxis;
    match axis {
        PartAxis::Length => 0,
        PartAxis::Width => 1,
        PartAxis::Thickness => 2,
    }
}

/// A hole in the part's local frame.
struct Hole {
    /// Local slot of the hole's axis, and of the two across it.
    axis: usize,
    u: usize,
    v: usize,
    /// Its centre on `u`, `v`.
    centre: Vec2,
    radius: f32,
    /// Its range along the axis.
    start: f32,
    end: f32,
    /// The polygon its wall follows, counter-clockwise on (`u`, `v`).
    polygon: Vec<Vec2>,
}

impl Hole {
    fn of(rod: &Rod) -> Hole {
        let [u, v] = across(rod.axis).map(slot);
        let centre = Vec2::new(render::mm(rod.centre[0]), render::mm(rod.centre[1]));
        let radius = render::mm(rod.diameter) * 0.5;
        let polygon = (0..LATHE_SEGMENTS)
            .map(|step| centre + Vec2::from_angle(angle(step)) * radius)
            .collect();
        Hole {
            axis: slot(rod.axis),
            u,
            v,
            centre,
            radius,
            start: render::mm(rod.start),
            end: render::mm(rod.end),
            polygon,
        }
    }

    /// A local point on the plane across the axis.
    fn flat(&self, local: Vec3) -> Vec2 {
        Vec2::new(local[self.u], local[self.v])
    }

    /// The local point at `along` the axis over a point of the plane.
    fn point(&self, flat: Vec2, along: f32) -> Vec3 {
        let mut local = Vec3::ZERO;
        local[self.axis] = along;
        local[self.u] = flat.x;
        local[self.v] = flat.y;
        local
    }

    fn within_range(&self, along: f32) -> bool {
        along > self.start - EPSILON && along < self.end + EPSILON
    }
}

fn angle(step: u16) -> f32 {
    std::f32::consts::TAU * f32::from(step) / f32::from(LATHE_SEGMENTS)
}

/// Twice the signed area of a triangle on the plane.
fn cross(a: Vec2, b: Vec2, c: Vec2) -> f32 {
    (b - a).perp_dot(c - a)
}

/// A corner of a clipped piece: where it is on the plane and its weights
/// on the triangle's three corners.
#[derive(Debug, Clone, Copy)]
struct Corner {
    flat: Vec2,
    weights: Vec3,
}

/// The part of a convex polygon on one side of the line through `from`
/// and `to`: the left side (`keep_left`) or the right.
fn clip(polygon: &[Corner], from: Vec2, to: Vec2, keep_left: bool) -> Vec<Corner> {
    let side = |corner: &Corner| {
        let value = cross(from, to, corner.flat);
        if keep_left { value } else { -value }
    };
    let mut out = Vec::with_capacity(polygon.len() + 2);
    for (index, current) in polygon.iter().enumerate() {
        let Some(next) = polygon.get((index + 1) % polygon.len()) else {
            continue;
        };
        let (a, b) = (side(current), side(next));
        if a >= 0.0 {
            out.push(*current);
        }
        if (a >= 0.0) != (b >= 0.0) {
            let t = a / (a - b);
            out.push(Corner {
                flat: current.flat + (next.flat - current.flat) * t,
                weights: current.weights + (next.weights - current.weights) * t,
            });
        }
    }
    out
}

/// Twice a polygon's area, absolute.
fn area(polygon: &[Corner]) -> f32 {
    let Some(first) = polygon.first() else {
        return 0.0;
    };
    polygon
        .windows(2)
        .map(|pair| match pair {
            [b, c] => cross(first.flat, b.flat, c.flat),
            _ => 0.0,
        })
        .sum::<f32>()
        .abs()
}

/// Drills a hole through a lathe mesh (in the render frame) of a part
/// whose frame is `frame`.
pub(crate) fn drill(geometry: &mut Geometry, frame: &Frame, grain: GrainAxes, rod: &Rod) {
    let hole = Hole::of(rod);
    let vertices = geometry.mesh.vertices.clone();
    let local: Vec<Vec3> = vertices
        .iter()
        .map(|vertex| frame.local(Vec3::from(vertex.position)))
        .collect();
    let at = |index: u32| usize::try_from(index).ok();
    let (triangles, _) = geometry.mesh.indices.as_chunks::<3>();
    let triangles: Vec<[usize; 3]> = triangles
        .iter()
        .filter_map(|&[a, b, c]| Some([at(a)?, at(b)?, at(c)?]))
        .filter(|corners| corners.iter().all(|i| *i < local.len()))
        .collect();
    let reach = hole.radius + EPSILON;
    let (low, high) = (
        hole.centre - Vec2::splat(reach),
        hole.centre + Vec2::splat(reach),
    );

    let mut indices: Vec<u32> = Vec::with_capacity(geometry.mesh.indices.len());
    let mut added: Vec<Vertex> = Vec::new();
    let base = u32::try_from(vertices.len()).unwrap_or(u32::MAX);
    let mut changed = false;
    for corners in &triangles {
        let positions = corners.map(|i| local.get(i).copied().unwrap_or(Vec3::ZERO));
        let keep = |indices: &mut Vec<u32>| {
            indices.extend(corners.map(|i| u32::try_from(i).unwrap_or(u32::MAX)));
        };
        let middle = (positions[0] + positions[1] + positions[2]) / 3.0;
        let flat = positions.map(|p| hole.flat(p));
        let (min, max) = (
            flat[0].min(flat[1]).min(flat[2]),
            flat[0].max(flat[1]).max(flat[2]),
        );
        let outside_box = max.x < low.x || max.y < low.y || min.x > high.x || min.y > high.y;
        if !hole.within_range(middle[hole.axis])
            || outside_box
            || cross(flat[0], flat[1], flat[2]).abs() < EPSILON * EPSILON
        {
            keep(&mut indices);
            continue;
        }
        let edges: Vec<(Vec2, Vec2)> = hole
            .polygon
            .iter()
            .enumerate()
            .filter_map(|(i, from)| Some((*from, *hole.polygon.get((i + 1) % hole.polygon.len())?)))
            .collect();
        let left = |(from, to): &(Vec2, Vec2), point: Vec2| cross(*from, *to, point) >= 0.0;
        if flat.iter().all(|p| edges.iter().all(|edge| left(edge, *p))) {
            changed = true;
            continue;
        }
        if edges
            .iter()
            .any(|edge| flat.iter().all(|p| !left(edge, *p)))
        {
            keep(&mut indices);
            continue;
        }
        changed = true;
        let mut rest: Vec<Corner> = vec![
            Corner {
                flat: flat[0],
                weights: Vec3::X,
            },
            Corner {
                flat: flat[1],
                weights: Vec3::Y,
            },
            Corner {
                flat: flat[2],
                weights: Vec3::Z,
            },
        ];
        for (from, to) in &edges {
            let outside = clip(&rest, *from, *to, false);
            if outside.len() >= 3 && area(&outside) > EPSILON * EPSILON {
                let first = u32::try_from(added.len()).unwrap_or(u32::MAX);
                for corner in &outside {
                    added.push(blend(&vertices, corners, corner.weights));
                }
                for k in 1..outside.len().saturating_sub(1) {
                    let index = |n: usize| {
                        base.saturating_add(first)
                            .saturating_add(u32::try_from(n).unwrap_or(u32::MAX))
                    };
                    indices.extend([index(0), index(k), index(k + 1)]);
                }
            }
            rest = clip(&rest, *from, *to, true);
            if rest.len() < 3 {
                break;
            }
        }
    }
    if !changed {
        return;
    }
    geometry.mesh.vertices.extend(added);
    geometry.mesh.indices = indices;
    // Outlines that crossed the opening go.
    geometry.edges.segments.retain(|[from, to]| {
        let middle = frame.local((Vec3::from(*from) + Vec3::from(*to)) * 0.5);
        !(hole.within_range(middle[hole.axis])
            && hole.flat(middle).distance(hole.centre) < hole.radius - EPSILON)
    });

    let surface: Vec<[Vec3; 3]> = triangles
        .iter()
        .map(|corners| corners.map(|i| local.get(i).copied().unwrap_or(Vec3::ZERO)))
        .collect();
    let walls = wall(frame, grain, &hole, &surface);
    let drilled = std::mem::replace(
        geometry,
        Geometry {
            mesh: crate::Mesh::default(),
            edges: crate::Edges::default(),
            bounds: geometry.bounds,
        },
    );
    *geometry = merge(vec![drilled, walls]);
}

/// A vertex part way across a triangle.
fn blend(vertices: &[Vertex], corners: &[usize; 3], weights: Vec3) -> Vertex {
    let get = |n: usize| {
        corners
            .get(n)
            .and_then(|i| vertices.get(*i))
            .copied()
            .unwrap_or(Vertex {
                position: [0.0; 3],
                normal: [0.0; 3],
                grain: [0.0; 3],
                cut: 0.0,
            })
    };
    let (a, b, c) = (get(0), get(1), get(2));
    let mix3 = |x: [f32; 3], y: [f32; 3], z: [f32; 3]| {
        (Vec3::from(x) * weights.x + Vec3::from(y) * weights.y + Vec3::from(z) * weights.z)
            .to_array()
    };
    Vertex {
        position: mix3(a.position, b.position, c.position),
        normal: Vec3::from(mix3(a.normal, b.normal, c.normal))
            .normalize_or_zero()
            .to_array(),
        grain: mix3(a.grain, b.grain, c.grain),
        cut: a.cut * weights.x + b.cut * weights.y + c.cut * weights.z,
    }
}

/// Where the line along the hole's axis through a point of the plane is
/// inside the drawn surface: the stretch of it, clipped to the hole's
/// range, that overlaps the range most. `None` when the line misses the
/// wood within the range.
fn span(hole: &Hole, surface: &[[Vec3; 3]], flat: Vec2) -> Option<Span> {
    let mut hits: Vec<f32> = surface
        .iter()
        .filter_map(|triangle| {
            let [a, b, c] = triangle.map(|p| hole.flat(p));
            let whole = cross(a, b, c);
            if whole.abs() < EPSILON * EPSILON {
                return None;
            }
            let (wa, wb, wc) = (
                cross(flat, b, c) / whole,
                cross(a, flat, c) / whole,
                cross(a, b, flat) / whole,
            );
            (wa >= 0.0 && wb >= 0.0 && wc >= 0.0).then(|| {
                triangle[0][hole.axis] * wa
                    + triangle[1][hole.axis] * wb
                    + triangle[2][hole.axis] * wc
            })
        })
        .collect();
    hits.sort_by(f32::total_cmp);
    hits.dedup_by(|a, b| (*a - *b).abs() < 1e-3);
    let (pairs, _) = hits.as_chunks::<2>();
    pairs
        .iter()
        .filter_map(|&[enter, leave]| {
            let (low, high) = (enter.max(hole.start), leave.min(hole.end));
            (high - low > EPSILON).then_some(Span {
                low,
                high,
                low_is_bottom: hole.start > enter + EPSILON,
                high_is_bottom: hole.end < leave - EPSILON,
            })
        })
        .max_by(|x, y| (x.high - x.low).total_cmp(&(y.high - y.low)))
}

/// A stretch of a hole's wall along its axis, and whether each end is the
/// hole's bottom (inside the wood) rather than where it opens.
#[derive(Debug, Clone, Copy)]
struct Span {
    low: f32,
    high: f32,
    low_is_bottom: bool,
    high_is_bottom: bool,
}

/// The hole's wall, bottom and opening outline, cast against the surface.
fn wall(frame: &Frame, grain: GrainAxes, hole: &Hole, surface: &[[Vec3; 3]]) -> Geometry {
    let mut builder = Builder::default();
    builder.set_cut(true);
    let spans: Vec<Option<Span>> = hole
        .polygon
        .iter()
        .map(|flat| span(hole, surface, *flat))
        .collect();
    let count = hole.polygon.len();
    let inward = |flat: Vec2| {
        let direction = (hole.centre - flat).normalize_or_zero();
        let mut normal = Vec3::ZERO;
        normal[hole.u] = direction.x;
        normal[hole.v] = direction.y;
        frame.vector(normal)
    };
    for k in 0..count {
        let next = (k + 1) % count;
        let (Some(Some(a)), Some(Some(b)), Some(pa), Some(pb)) = (
            spans.get(k),
            spans.get(next),
            hole.polygon.get(k),
            hole.polygon.get(next),
        ) else {
            continue;
        };
        let mut corner = |flat: Vec2, along: f32| {
            let local = hole.point(flat, along);
            builder.vertex(frame.point(local), inward(flat), grain.apply(local))
        };
        let quad = [
            corner(*pa, a.low),
            corner(*pb, b.low),
            corner(*pb, b.high),
            corner(*pa, a.high),
        ];
        builder.triangle(quad[0], quad[1], quad[2]);
        builder.triangle(quad[0], quad[2], quad[3]);
        // The opening's outline where the wall meets the surface.
        if !a.low_is_bottom && !b.low_is_bottom {
            builder.edge(
                frame.point(hole.point(*pa, a.low)),
                frame.point(hole.point(*pb, b.low)),
            );
        }
        if !a.high_is_bottom && !b.high_is_bottom {
            builder.edge(
                frame.point(hole.point(*pa, a.high)),
                frame.point(hole.point(*pb, b.high)),
            );
        }
    }
    // A bottom where the hole stops in the wood, facing the opening.
    let all = |test: fn(&Span) -> bool| spans.iter().all(|s| s.as_ref().is_some_and(test));
    for (along, stops, toward_high) in [
        (hole.start, all(|s| s.low_is_bottom), true),
        (hole.end, all(|s| s.high_is_bottom), false),
    ] {
        if !stops {
            continue;
        }
        let mut normal = Vec3::ZERO;
        normal[hole.axis] = if toward_high { 1.0 } else { -1.0 };
        let world_normal = frame.vector(normal);
        let centre_local = hole.point(hole.centre, along);
        let middle = builder.vertex(
            frame.point(centre_local),
            world_normal,
            grain.apply(centre_local),
        );
        let rim: Vec<u32> = hole
            .polygon
            .iter()
            .chain(hole.polygon.first())
            .map(|flat| {
                let local = hole.point(*flat, along);
                builder.vertex(frame.point(local), world_normal, grain.apply(local))
            })
            .collect();
        for pair in rim.windows(2) {
            if let &[x, y] = pair {
                builder.triangle(middle, x, y);
            }
        }
    }
    builder.finish()
}
