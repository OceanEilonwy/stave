//! Holes drilled in round stock and turned parts.
//!
//! A lathe mesh is a faceted solid of revolution, so a hole can't be cut
//! from a grid the way a board's are. Instead the hole is cut out of the
//! mesh itself, so that the result is **watertight**: every edge is shared
//! by exactly two triangles, with no T-junctions where cracks would show.
//!
//! - Every surface triangle within the hole's range along its axis is
//!   projected along that axis, and the hole's polygon (the same polygon
//!   its wall follows) is taken out of it exactly: the rest of the
//!   triangle is traced as one or more loops (its own corners outside the
//!   hole, the points where the polygon crosses its edges, the polygon's
//!   corners inside it) and those loops are triangulated without adding a
//!   point. A crossing on an edge shared by two triangles is computed once,
//!   from the edge's ends in a fixed order, so both triangles get exactly
//!   the same point; so is each polygon corner on the surface.
//! - The wall is one flat facet per polygon side, from the hole's bottom
//!   (or the far surface) to the surface, its edge along the surface
//!   passing through exactly the crossings and corners the surface uses.
//! - A bottom disc closes a hole that stops inside the wood, and the
//!   opening is outlined.
//!
//! Wall, bottom and opening rim are cut faces. Holes along the lathe's own
//! axis (into an end) work the same way: the end cap is what is cut.

use std::collections::HashMap;

use glam::{Vec2, Vec3};

use crate::axes::on_part;
use crate::build::{Builder, Frame, GrainAxes, LATHE_SEGMENTS};
use crate::mesh::merge;
use crate::solid::{Rod, Slant, across};
use crate::{Geometry, Vertex, render};

/// Lengths below this (millimetres) count as zero.
const EPSILON: f32 = 1e-4;

/// A position as exact bits, for finding the same point twice.
type Key = [u32; 3];

fn key(point: Vec3) -> Key {
    // `+ 0.0` makes -0.0 and 0.0 the same.
    point.to_array().map(|c| (c + 0.0).to_bits())
}

/// A hole in the part's local frame (millimetres): along `axis` from
/// `start` to `end`, its wall following `polygon` on the plane across it,
/// spanned by `u` and `v`. The three are at right angles; for a hole along
/// one of the part's axes they are the part's axes.
struct Hole {
    /// Unit vectors along the hole's axis and the two across it.
    axis: Vec3,
    u: Vec3,
    v: Vec3,
    /// Its centre on `u`, `v`.
    centre: Vec2,
    /// Its range along the axis.
    start: f32,
    end: f32,
    /// The polygon its wall follows, counter-clockwise on (`u`, `v`).
    polygon: Vec<Vec2>,
    /// `true` for a slanted hole: a surface triangle is cut where the
    /// hole's axis meets its plane within the hole's range (it may be far
    /// bigger than the hole, a board's face), not where its middle is.
    slanted: bool,
}

/// A local unit vector along a part axis.
fn unit(axis: stave_lang::placement::PartAxis) -> Vec3 {
    on_part(axis, 1.0)
}

impl Hole {
    fn of(rod: &Rod) -> Hole {
        let [u, v] = across(rod.axis).map(unit);
        let centre = Vec2::new(render::mm(rod.centre[0]), render::mm(rod.centre[1]));
        let radius = render::mm(rod.diameter) * 0.5;
        Hole {
            axis: unit(rod.axis),
            u,
            v,
            centre,
            start: render::mm(rod.start),
            end: render::mm(rod.end),
            polygon: ring(centre, radius),
            slanted: false,
        }
    }

    /// A slanted hole (grammar, "Slanted members").
    fn of_slant(slant: &Slant) -> Hole {
        let direction = slant.direction().as_vec3().normalize_or_zero();
        let u = direction.any_orthonormal_vector();
        let v = direction.cross(u);
        let entry = Vec3::new(
            render::mm(slant.entry.length),
            render::mm(slant.entry.width),
            render::mm(slant.entry.thickness),
        );
        let centre = Vec2::new(entry.dot(u), entry.dot(v));
        let at = entry.dot(direction);
        Hole {
            axis: direction,
            u,
            v,
            centre,
            start: at + render::mm(slant.start),
            end: at + render::mm(slant.end),
            polygon: ring(centre, render::mm(slant.diameter) * 0.5),
            slanted: true,
        }
    }

    /// How far along the axis a local point is.
    fn along(&self, local: Vec3) -> f32 {
        local.dot(self.axis)
    }

    /// A local point on the plane across the axis.
    fn flat(&self, local: Vec3) -> Vec2 {
        Vec2::new(local.dot(self.u), local.dot(self.v))
    }

    /// The local point at `along` the axis over a point of the plane.
    fn point(&self, flat: Vec2, along: f32) -> Vec3 {
        self.axis * along + self.u * flat.x + self.v * flat.y
    }

    /// Does the hole go through the surface triangle with these local
    /// corners? For a slanted hole: where its axis meets the triangle's
    /// plane, within its range or near enough that its rim does (the rim
    /// reaches `radius × cot` of the angle the axis meets the plane at,
    /// along the axis). For any other hole: where the triangle's middle is.
    fn meets(&self, corners: [Vec3; 3]) -> bool {
        let middle = (corners[0] + corners[1] + corners[2]) / 3.0;
        if !self.slanted {
            return self.within_range(self.along(middle));
        }
        let normal = (corners[1] - corners[0])
            .cross(corners[2] - corners[0])
            .normalize_or_zero();
        let facing = normal.dot(self.axis);
        if facing.abs() < 1e-4 {
            return false;
        }
        let base = self.point(self.centre, 0.0);
        let hit = (normal.dot(corners[0]) - normal.dot(base)) / facing;
        let radius = self
            .polygon
            .first()
            .map_or(0.0, |p| p.distance(self.centre));
        let reach = radius * (1.0 - facing * facing).max(0.0).sqrt() / facing.abs();
        hit > self.start - reach - EPSILON && hit < self.end + reach + EPSILON
    }

    fn within_range(&self, along: f32) -> bool {
        along > self.start - EPSILON && along < self.end + EPSILON
    }

    fn corner(&self, k: usize) -> Vec2 {
        self.polygon
            .get(k % self.polygon.len().max(1))
            .copied()
            .unwrap_or(self.centre)
    }

    fn count(&self) -> usize {
        self.polygon.len()
    }
}

/// The corners of a hole's polygon round a centre.
fn ring(centre: Vec2, radius: f32) -> Vec<Vec2> {
    (0..LATHE_SEGMENTS)
        .map(|step| centre + Vec2::from_angle(angle(step)) * radius)
        .collect()
}

/// The angle of a corner of the hole's polygon. The corners sit half a
/// step off the hole's axes, so none lands exactly on a lathe facet's edge
/// along the part (the meridians lie over the hole's axes).
fn angle(step: u16) -> f32 {
    std::f32::consts::TAU * (f32::from(step) + 0.5) / f32::from(LATHE_SEGMENTS)
}

/// Twice the signed area of a triangle on the plane.
fn cross(a: Vec2, b: Vec2, c: Vec2) -> f32 {
    (b - a).perp_dot(c - a)
}

/// Where two segments on the plane cross, as parameters along each, when
/// they do (strictly inside the first, anywhere on the second).
fn crossing(p: Vec2, q: Vec2, a: Vec2, b: Vec2) -> Option<(f32, f32)> {
    let d = q - p;
    let e = b - a;
    let denominator = d.perp_dot(e);
    if denominator.abs() < 1e-12 {
        return None;
    }
    let t = (a - p).perp_dot(e) / denominator;
    let s = (a - p).perp_dot(d) / denominator;
    (t > 0.0 && t < 1.0 && (0.0..=1.0).contains(&s)).then_some((t, s))
}

/// Points found once and shared by every triangle and wall that uses them.
#[derive(Default)]
struct Shared {
    /// Where polygon side `k` crosses a surface edge (its ends' keys in a
    /// fixed order): the point and how far along the side.
    crossings: HashMap<(Key, Key, usize), (Vec3, f32)>,
    /// Polygon corner `k` on the surface, per side of the part (the
    /// surface facing one way or the other along the axis).
    corners: HashMap<(usize, bool), Vec3>,
}

/// A point of a loop being traced in one triangle: on the plane, in the
/// render frame, and its weights on the triangle's corners.
#[derive(Debug, Clone, Copy)]
struct Point {
    flat: Vec2,
    world: Vec3,
    weights: Vec3,
}

/// One point on a triangle's boundary as it is walked.
#[derive(Debug, Clone, Copy)]
enum Boundary {
    /// One of its own corners (0 to 2), and whether it is outside the hole.
    Corner(usize, bool),
    /// A crossing with polygon side `side`, `along` that side, entering
    /// the hole or leaving it.
    Crossing {
        point: Point,
        side: usize,
        along: f32,
        enter: bool,
    },
}

/// A triangle's corners: indices, positions in the render frame and on the
/// plane, counter-clockwise on the plane.
struct Facet {
    indices: [usize; 3],
    world: [Vec3; 3],
    flat: [Vec2; 3],
    /// The original winding was clockwise on the plane: triangles made
    /// from it are wound back.
    flipped: bool,
}

/// Drills a hole through a lathe mesh (in the render frame) of a part
/// whose frame is `frame`.
pub(crate) fn drill(geometry: &mut Geometry, frame: &Frame, grain: GrainAxes, rod: &Rod) {
    drill_hole(geometry, frame, grain, &Hole::of(rod));
}

/// Drills a slanted hole through the mesh (in the render frame) of a
/// part whose frame is `frame`: a board's or a lathe's alike.
pub(crate) fn drill_slant(geometry: &mut Geometry, frame: &Frame, grain: GrainAxes, slant: &Slant) {
    drill_hole(geometry, frame, grain, &Hole::of_slant(slant));
}

fn drill_hole(geometry: &mut Geometry, frame: &Frame, grain: GrainAxes, hole: &Hole) {
    if hole.slanted {
        split_round(geometry, frame, hole);
    }
    let vertices = &geometry.mesh.vertices;
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
    let reach = hole
        .polygon
        .iter()
        .map(|p| p.distance(hole.centre))
        .fold(0.0, f32::max)
        + EPSILON;
    let (low, high) = (
        hole.centre - Vec2::splat(reach),
        hole.centre + Vec2::splat(reach),
    );

    let mut shared = Shared::default();
    let mut indices: Vec<u32> = Vec::with_capacity(geometry.mesh.indices.len());
    let mut added: Vec<Vertex> = Vec::new();
    let base = u32::try_from(vertices.len()).unwrap_or(u32::MAX);
    let mut changed = false;
    for corners in &triangles {
        let positions = corners.map(|i| local.get(i).copied().unwrap_or(Vec3::ZERO));
        let world = corners.map(|i| {
            vertices
                .get(i)
                .map_or(Vec3::ZERO, |v| Vec3::from(v.position))
        });
        let keep = |indices: &mut Vec<u32>| {
            indices.extend(corners.map(|i| u32::try_from(i).unwrap_or(u32::MAX)));
        };
        let flat = positions.map(|p| hole.flat(p));
        let (min, max) = (
            flat[0].min(flat[1]).min(flat[2]),
            flat[0].max(flat[1]).max(flat[2]),
        );
        let outside_box = max.x < low.x || max.y < low.y || min.x > high.x || min.y > high.y;
        let area = cross(flat[0], flat[1], flat[2]);
        if !hole.meets(positions) || outside_box || area.abs() < EPSILON * EPSILON {
            keep(&mut indices);
            continue;
        }
        let flipped = area < 0.0;
        let facet = if flipped {
            Facet {
                indices: [corners[0], corners[2], corners[1]],
                world: [world[0], world[2], world[1]],
                flat: [flat[0], flat[2], flat[1]],
                flipped,
            }
        } else {
            Facet {
                indices: *corners,
                world,
                flat,
                flipped,
            }
        };
        match cut_facet(hole, &facet, &mut shared) {
            None => keep(&mut indices),
            Some(loops) => {
                changed = true;
                for points in loops {
                    let first = u32::try_from(added.len()).unwrap_or(u32::MAX);
                    for point in &points {
                        added.push(blend(vertices, &facet.indices, point));
                    }
                    let flats: Vec<Vec2> = points.iter().map(|p| p.flat).collect();
                    for [a, b, c] in ear_clip(&flats) {
                        let index = |n: usize| {
                            base.saturating_add(first)
                                .saturating_add(u32::try_from(n).unwrap_or(u32::MAX))
                        };
                        if facet.flipped {
                            indices.extend([index(a), index(c), index(b)]);
                        } else {
                            indices.extend([index(a), index(b), index(c)]);
                        }
                    }
                }
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
        !(hole.within_range(hole.along(middle)) && inside_polygon(&hole.polygon, hole.flat(middle)))
    });

    let surface: Vec<[Vec3; 3]> = triangles
        .iter()
        .map(|corners| corners.map(|i| local.get(i).copied().unwrap_or(Vec3::ZERO)))
        .collect();
    let walls = wall(frame, grain, hole, &surface, &shared);
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

/// Splits each surface triangle the hole goes through with its centre
/// inside (a board's face is far bigger than a hole) into three about the
/// centre, so the hole's polygon crosses their edges as it crosses a
/// lathe's small facets. The new edges are inside the old triangle, so no
/// crack opens beside it.
fn split_round(geometry: &mut Geometry, frame: &Frame, hole: &Hole) {
    let vertices = &geometry.mesh.vertices;
    let at = |index: u32| usize::try_from(index).ok();
    let (triangles, _) = geometry.mesh.indices.as_chunks::<3>();
    let mut indices = Vec::with_capacity(geometry.mesh.indices.len());
    let mut added: Vec<Vertex> = Vec::new();
    let base = vertices.len();
    for &[a, b, c] in triangles {
        let corners = [at(a), at(b), at(c)];
        let [Some(i), Some(j), Some(k)] = corners else {
            indices.extend([a, b, c]);
            continue;
        };
        let (Some(va), Some(vb), Some(vc)) = (vertices.get(i), vertices.get(j), vertices.get(k))
        else {
            indices.extend([a, b, c]);
            continue;
        };
        let world = [va, vb, vc].map(|v| Vec3::from(v.position));
        let local = world.map(|w| frame.local(w));
        let flat = local.map(|p| hole.flat(p));
        let whole = cross(flat[0], flat[1], flat[2]);
        if whole.abs() < EPSILON * EPSILON || !hole.meets(local) {
            indices.extend([a, b, c]);
            continue;
        }
        let centre = hole.centre;
        let weights = Vec3::new(
            cross(centre, flat[1], flat[2]) / whole,
            cross(flat[0], centre, flat[2]) / whole,
            cross(flat[0], flat[1], centre) / whole,
        );
        if weights.min_element() <= 1e-4 {
            indices.extend([a, b, c]);
            continue;
        }
        let point = Point {
            flat: centre,
            world: world[0] * weights.x + world[1] * weights.y + world[2] * weights.z,
            weights,
        };
        let middle = u32::try_from(base + added.len()).unwrap_or(u32::MAX);
        added.push(blend(vertices, &[i, j, k], &point));
        indices.extend([a, b, middle, b, c, middle, c, a, middle]);
    }
    if added.is_empty() {
        return;
    }
    geometry.mesh.vertices.extend(added);
    geometry.mesh.indices = indices;
}

/// Strictly inside a convex counter-clockwise polygon.
fn inside_polygon(polygon: &[Vec2], point: Vec2) -> bool {
    polygon.iter().enumerate().all(|(k, from)| {
        let to = polygon
            .get((k + 1) % polygon.len())
            .copied()
            .unwrap_or(*from);
        cross(*from, to, point) > EPSILON * EPSILON
    })
}

/// At or inside a convex counter-clockwise polygon.
fn on_or_inside(polygon: &[Vec2], point: Vec2) -> bool {
    polygon.iter().enumerate().all(|(k, from)| {
        let to = polygon
            .get((k + 1) % polygon.len())
            .copied()
            .unwrap_or(*from);
        cross(*from, to, point) >= 0.0
    })
}

/// A triangle less the hole: the loops of what is left, or `None` when
/// the hole doesn't touch it (or lies wholly inside it, which a hole
/// through a turned part never does: then it is left whole).
fn cut_facet(hole: &Hole, facet: &Facet, shared: &mut Shared) -> Option<Vec<Vec<Point>>> {
    let corner_point = |i: usize| -> Point {
        let mut weights = Vec3::ZERO;
        weights[i] = 1.0;
        Point {
            flat: facet.flat[i],
            world: facet.world[i],
            weights,
        }
    };
    let outside: [bool; 3] = [0, 1, 2].map(|i| !on_or_inside(&hole.polygon, facet.flat[i]));
    if outside.iter().all(|o| !o) {
        // Wholly inside the hole.
        return Some(Vec::new());
    }
    // The boundary, walked counter-clockwise, with its crossings.
    let mut boundary: Vec<Boundary> = Vec::new();
    for i in 0..3 {
        let j = (i + 1) % 3;
        boundary.push(Boundary::Corner(i, outside[i]));
        let (p, q) = (facet.flat[i], facet.flat[j]);
        // The edge's ends in a fixed order, so the triangle on its other
        // side finds the same point.
        let (ka, kb) = (key(facet.world[i]), key(facet.world[j]));
        let forward = ka <= kb;
        let mut found: Vec<(f32, Boundary)> = Vec::new();
        for side in 0..hole.count() {
            let (a, b) = (hole.corner(side), hole.corner(side + 1));
            let (op, oq, ow, ox) = if forward {
                (p, q, facet.world[i], facet.world[j])
            } else {
                (q, p, facet.world[j], facet.world[i])
            };
            let Some((t_ordered, _)) = crossing(op, oq, a, b) else {
                continue;
            };
            let entry = (if forward { (ka, kb) } else { (kb, ka) }, side);
            let (world, along) = *shared
                .crossings
                .entry((entry.0.0, entry.0.1, entry.1))
                .or_insert_with(|| {
                    let along = crossing(op, oq, a, b).map_or(0.0, |(_, s)| s);
                    (ow + (ox - ow) * t_ordered, along)
                });
            let t = if forward { t_ordered } else { 1.0 - t_ordered };
            let mut weights = Vec3::ZERO;
            weights[i] = 1.0 - t;
            weights[j] = t;
            let flat = p + (q - p) * t;
            let enter = (b - a).perp_dot(q - p) > 0.0;
            found.push((
                t,
                Boundary::Crossing {
                    point: Point {
                        flat,
                        world,
                        weights,
                    },
                    side,
                    along,
                    enter,
                },
            ));
        }
        found.sort_by(|x, y| x.0.total_cmp(&y.0));
        boundary.extend(found.into_iter().map(|(_, b)| b));
    }
    let has_crossing = boundary
        .iter()
        .any(|b| matches!(b, Boundary::Crossing { .. }));
    if !has_crossing {
        // Outside the hole, or the hole inside it: left whole.
        return None;
    }
    // A polygon corner on the surface, shared by both triangles it might
    // touch on this side of the part.
    let mut polygon_corner = |k: usize| -> Option<Point> {
        let c = hole.corner(k);
        let [a, b, d] = facet.flat;
        let whole = cross(a, b, d);
        let weights = Vec3::new(
            cross(c, b, d) / whole,
            cross(a, c, d) / whole,
            cross(a, b, c) / whole,
        );
        if weights.min_element() < -1e-5 {
            return None;
        }
        let world = *shared
            .corners
            .entry((k % hole.count(), facet.flipped))
            .or_insert_with(|| {
                facet.world[0] * weights.x + facet.world[1] * weights.y + facet.world[2] * weights.z
            });
        Some(Point {
            flat: c,
            world,
            weights,
        })
    };
    // Walk: along the triangle outside the hole; at a crossing into it,
    // back along the polygon (clockwise) to where the triangle comes out.
    let count = boundary.len();
    let mut used = vec![false; count];
    let mut loops = Vec::new();
    while let Some(start) = (0..count).find(|&n| {
        !used[n]
            && match boundary.get(n) {
                Some(Boundary::Corner(_, outside)) => *outside,
                Some(Boundary::Crossing { enter, .. }) => !*enter,
                None => false,
            }
    }) {
        let mut points: Vec<Point> = Vec::new();
        let mut at = start;
        let mut guard = 0;
        loop {
            guard += 1;
            if guard > 4 * count + 4 * hole.count() {
                return None;
            }
            used[at] = true;
            match boundary.get(at).copied() {
                Some(Boundary::Corner(i, outside)) => {
                    if outside {
                        points.push(corner_point(i));
                    }
                    at = (at + 1) % count;
                }
                Some(Boundary::Crossing {
                    point,
                    side,
                    along,
                    enter: false,
                }) => {
                    let _ = (side, along);
                    points.push(point);
                    at = (at + 1) % count;
                }
                Some(Boundary::Crossing {
                    point,
                    side,
                    along,
                    enter: true,
                }) => {
                    points.push(point);
                    // Back along the polygon from side `side` at `along`.
                    let mut current = side;
                    let mut limit = along;
                    let mut steps = 0;
                    let next = loop {
                        // The exit on this side nearest below `limit`.
                        let exit = boundary
                            .iter()
                            .enumerate()
                            .filter_map(|(n, b)| match b {
                                Boundary::Crossing {
                                    side, along, enter, ..
                                } if !*enter && *side == current && *along < limit => {
                                    Some((n, *along))
                                }
                                Boundary::Corner(..) | Boundary::Crossing { .. } => None,
                            })
                            .max_by(|x, y| x.1.total_cmp(&y.1));
                        if let Some((n, _)) = exit {
                            break n;
                        }
                        points.push(polygon_corner(current)?);
                        current = (current + hole.count() - 1) % hole.count();
                        limit = f32::INFINITY;
                        steps += 1;
                        if steps > hole.count() {
                            return None;
                        }
                    };
                    at = next;
                }
                None => return None,
            }
            if at == start {
                break;
            }
        }
        loops.push(points);
    }
    Some(loops)
}

/// Triangulates a simple polygon on the plane by cutting off ears; only
/// its own points are used. Counter-clockwise in, counter-clockwise out
/// (a clockwise polygon is turned round first).
fn ear_clip(points: &[Vec2]) -> Vec<[usize; 3]> {
    let mut order: Vec<usize> = (0..points.len()).collect();
    let area: f32 = (0..points.len())
        .map(|i| {
            let a = points.get(i).copied().unwrap_or(Vec2::ZERO);
            let b = points
                .get((i + 1) % points.len())
                .copied()
                .unwrap_or(Vec2::ZERO);
            a.perp_dot(b)
        })
        .sum();
    if area < 0.0 {
        order.reverse();
    }
    let at = |i: usize| points.get(i).copied().unwrap_or(Vec2::ZERO);
    // A point on (or within float noise of) a candidate ear's side
    // counts as inside it: a nearly straight run must not be cut across.
    let size = points.iter().fold(
        (Vec2::splat(f32::INFINITY), Vec2::splat(f32::NEG_INFINITY)),
        |(lo, hi), p| (lo.min(*p), hi.max(*p)),
    );
    let tolerance = (size.1 - size.0).length_squared() * 1e-7;
    let mut out = Vec::new();
    let mut guard = 0;
    while order.len() > 3 && guard < 10_000 {
        guard += 1;
        let n = order.len();
        let ear = (0..n).find(|&k| {
            let (a, b, c) = (order[(k + n - 1) % n], order[k], order[(k + 1) % n]);
            let (pa, pb, pc) = (at(a), at(b), at(c));
            if cross(pa, pb, pc) <= 0.0 {
                return false;
            }
            order.iter().all(|&o| {
                if o == a || o == b || o == c {
                    return true;
                }
                let p = at(o);
                !(cross(pa, pb, p) >= -tolerance
                    && cross(pb, pc, p) >= -tolerance
                    && cross(pc, pa, p) >= -tolerance)
            })
        });
        // A degenerate run (points in a line) has no proper ear: cut the
        // flattest corner off.
        let k = ear.unwrap_or_else(|| {
            (0..n)
                .min_by(|&x, &y| {
                    let flatness = |k: usize| {
                        cross(
                            at(order[(k + n - 1) % n]),
                            at(order[k]),
                            at(order[(k + 1) % n]),
                        )
                        .abs()
                    };
                    flatness(x).total_cmp(&flatness(y))
                })
                .unwrap_or(0)
        });
        let (a, b, c) = (order[(k + n - 1) % n], order[k], order[(k + 1) % n]);
        if cross(at(a), at(b), at(c)) > 0.0 {
            out.push([a, b, c]);
        }
        order.remove(k);
    }
    if let [a, b, c] = order.as_slice()
        && cross(at(*a), at(*b), at(*c)) > 0.0
    {
        out.push([*a, *b, *c]);
    }
    out
}

/// A vertex part way across a triangle, at a point found once.
fn blend(vertices: &[Vertex], corners: &[usize; 3], point: &Point) -> Vertex {
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
    let w = point.weights;
    let mix3 = |x: [f32; 3], y: [f32; 3], z: [f32; 3]| {
        (Vec3::from(x) * w.x + Vec3::from(y) * w.y + Vec3::from(z) * w.z).to_array()
    };
    Vertex {
        position: point.world.to_array(),
        normal: Vec3::from(mix3(a.normal, b.normal, c.normal))
            .normalize_or_zero()
            .to_array(),
        grain: mix3(a.grain, b.grain, c.grain),
        cut: a.cut * w.x + b.cut * w.y + c.cut * w.z,
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
                hole.along(triangle[0]) * wa
                    + hole.along(triangle[1]) * wb
                    + hole.along(triangle[2]) * wc
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

/// The hole's wall, bottom and opening outline, meeting the surface at the
/// points it was cut at.
fn wall(
    frame: &Frame,
    grain: GrainAxes,
    hole: &Hole,
    surface: &[[Vec3; 3]],
    shared: &Shared,
) -> Geometry {
    let mut builder = Builder::default();
    builder.set_cut(true);
    let count = hole.count();
    let spans: Vec<Option<Span>> = hole
        .polygon
        .iter()
        .map(|flat| span(hole, surface, *flat))
        .collect();
    let inward = |flat: Vec2| {
        let direction = (hole.centre - flat).normalize_or_zero();
        frame.vector(hole.u * direction.x + hole.v * direction.y)
    };
    let axial = |world: Vec3| hole.along(frame.local(world));
    // The surface's points along polygon side `k` near the axial value
    // `near`: its corners and crossings, ordered along the side.
    let surface_chain = |k: usize, near: (f32, f32)| -> Option<Vec<(f32, Vec3)>> {
        let side = [false, true].into_iter().find(|side| {
            shared
                .corners
                .get(&(k % count, *side))
                .is_some_and(|w| (axial(*w) - near.0).abs() < 0.5)
        })?;
        let first = *shared.corners.get(&(k % count, side))?;
        let last = *shared.corners.get(&((k + 1) % count, side))?;
        if (axial(last) - near.1).abs() >= 0.5 {
            return None;
        }
        let mut chain = vec![(0.0, first)];
        let mut middle: Vec<(f32, Vec3)> = shared
            .crossings
            .iter()
            .filter(|((_, _, s), (world, _))| {
                *s == k % count && {
                    let a = axial(*world);
                    let expected = near.0 + (near.1 - near.0) * 0.5;
                    (a - expected).abs() < (near.1 - near.0).abs() * 0.5 + 0.5
                }
            })
            .map(|(_, (world, along))| (*along, *world))
            .collect();
        middle.sort_by(|x, y| x.0.total_cmp(&y.0));
        chain.extend(middle);
        chain.push((1.0, last));
        Some(chain)
    };
    for k in 0..count {
        let next = (k + 1) % count;
        let (Some(Some(a)), Some(Some(b))) = (spans.get(k), spans.get(next)) else {
            continue;
        };
        let (pa, pb) = (hole.corner(k), hole.corner(next));
        let straight = |along_a: f32, along_b: f32| {
            vec![
                (0.0, frame.point(hole.point(pa, along_a))),
                (1.0, frame.point(hole.point(pb, along_b))),
            ]
        };
        let low = if a.low_is_bottom && b.low_is_bottom {
            straight(a.low, b.low)
        } else {
            surface_chain(k, (a.low, b.low)).unwrap_or_else(|| straight(a.low, b.low))
        };
        let high = if a.high_is_bottom && b.high_is_bottom {
            straight(a.high, b.high)
        } else {
            surface_chain(k, (a.high, b.high)).unwrap_or_else(|| straight(a.high, b.high))
        };
        // The facet: along the low edge, then back along the high one.
        let outline: Vec<(f32, Vec3)> = low
            .iter()
            .copied()
            .chain(high.iter().rev().copied())
            .collect();
        let flats: Vec<Vec2> = outline
            .iter()
            .map(|(along, world)| Vec2::new(*along * 1000.0, axial(*world)))
            .collect();
        let indices: Vec<u32> = outline
            .iter()
            .map(|(along, world)| {
                let normal = inward(pa).lerp(inward(pb), *along);
                builder.vertex(*world, normal, grain.apply(frame.local(*world)))
            })
            .collect();
        for [x, y, z] in ear_clip(&flats) {
            if let (Some(&i), Some(&j), Some(&l)) = (indices.get(x), indices.get(y), indices.get(z))
            {
                builder.triangle(i, j, l);
            }
        }
        // The opening's outline where the wall meets the surface.
        for (chain, opens) in [
            (&low, !(a.low_is_bottom && b.low_is_bottom)),
            (&high, !(a.high_is_bottom && b.high_is_bottom)),
        ] {
            if opens {
                for pair in chain.windows(2) {
                    if let [(_, from), (_, to)] = pair {
                        builder.edge(*from, *to);
                    }
                }
            }
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
        let normal = if toward_high { hole.axis } else { -hole.axis };
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
