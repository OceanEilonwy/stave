//! Solids in the plane a motion turns in (grammar, "One collision model"):
//! every body as convex pieces with their range along the axis the plane
//! is across, ready for the sweeps and for a check at one pose.
//!
//! A piece is a part's block, the box around a rod, or a member's prism or
//! bar, projected on the plane and shrunk by half the contact tolerance on
//! every edge, so two pieces that only touch never overlap. **Sweeps see
//! cuts**: a part's boxes are axis-aligned at rest, and a round hole whose
//! axis lies in the plane (a cup's bore) is taken out of each box it
//! crosses as its narrowest chord over the range, along the axis, that the
//! other piece shares with it, so no more is taken than the hole takes.
//! A round hole along the axis keeps a piece inside it clear of the
//! blocks it is drilled in ([`Bores`]): at one pose, and all the way round
//! a turn about the plane's origin, which is exact for a hole on the line
//! the body turns about. Other holes and angled cuts are left as wood:
//! more wood than there is, never less. Everything here is `f64`
//! nanometres from exact inputs,
//! converted in [`render`](crate::render).

use glam::{DMat4, DVec2, DVec3};
use stave_lang::placement::Axis;
use stave_lang::units::Length;
use stave_model::mechanism::TurnLine;
use stave_model::solved::{Point, WorldBox};

use crate::collide::{CONTACT_TOLERANCE, WorldRod, WorldSolid, direction_vector, point_vector};
use crate::mechanism::MechanismPath;
use crate::render;

/// A plane across a world axis: `x` and `y` in it, origin on it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Plane {
    pub origin: DVec3,
    pub x: DVec3,
    pub y: DVec3,
    pub axis: Axis,
}

impl Plane {
    /// A mechanism's plane: `x` and `y` along its frame's two directions,
    /// origin on its frame's origin, across the pins' axis.
    pub fn of_mechanism(mechanism: &MechanismPath) -> Plane {
        let (origin, x, y) = mechanism.plane_basis();
        Plane {
            origin,
            x,
            y,
            axis: mechanism.axis(),
        }
    }

    /// The plane across a hinge's pivot line: `x` toward the free side,
    /// `y` toward the side it swings to, origin on the pivot. Turning by a
    /// positive angle is counter-clockwise here.
    pub fn of_pivot(pivot: &TurnLine) -> Plane {
        Plane {
            origin: point_vector(pivot.through),
            x: direction_vector(pivot.free_side),
            y: direction_vector(pivot.swing_toward),
            axis: pivot.axis,
        }
    }

    /// The plane across a world axis through the world's origin.
    pub fn across(axis: Axis) -> Plane {
        let [first, second] = axis.others();
        let unit = |on: Axis| point_vector(Point::default().with(on, Length::from_nm(1)));
        Plane {
            origin: DVec3::ZERO,
            x: unit(first),
            y: unit(second),
            axis,
        }
    }

    /// A world point on the plane.
    pub fn project(&self, point: DVec3) -> DVec2 {
        let relative = point - self.origin;
        DVec2::new(relative.dot(self.x), relative.dot(self.y))
    }

    /// A world point's coordinate along the axis.
    pub fn along(&self, point: DVec3) -> f64 {
        point_vector(Point::default().with(self.axis, Length::from_nm(1))).dot(point)
    }
}

/// A convex polygon, counter-clockwise, with its range along the axis.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Piece {
    pub corners: Vec<DVec2>,
    pub low: f64,
    pub high: f64,
}

impl Piece {
    /// The farthest a corner lies from a point of the plane.
    pub fn radius_about(&self, centre: DVec2) -> f64 {
        self.corners
            .iter()
            .map(|corner| (*corner - centre).length())
            .fold(0.0, f64::max)
    }

    /// How far its range along the axis and another's overlap.
    pub fn shared(&self, other: &Piece) -> f64 {
        self.high.min(other.high) - self.low.max(other.low)
    }

    /// The piece moved in the plane.
    pub fn moved(&self, motion: Motion2) -> Piece {
        Piece {
            corners: self.corners.iter().map(|c| motion.apply(*c)).collect(),
            low: self.low,
            high: self.high,
        }
    }
}

/// A rigid motion in the plane: turn about the plane's origin, then move.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Motion2 {
    /// `(cos, sin)` of the turn.
    pub rotation: DVec2,
    pub offset: DVec2,
}

impl Motion2 {
    /// Staying put.
    pub const STILL: Motion2 = Motion2 {
        rotation: DVec2::X,
        offset: DVec2::ZERO,
    };

    /// A point moved.
    pub fn apply(&self, point: DVec2) -> DVec2 {
        self.rotation.rotate(point) + self.offset
    }
}

/// One element of a solid's wood before it is cut by holes: a box (a block,
/// or the box around a rod) or a member's outline with its corners in the
/// world.
#[derive(Debug, Clone)]
enum Element {
    /// A box; `block` when it is one of the solid's blocks, which its
    /// holes are drilled in (not the box around a rod or a slant).
    Box {
        region: WorldBox,
        block: bool,
    },
    Outline(Vec<DVec3>),
}

impl Element {
    /// One of the solid's blocks, which its holes are drilled in.
    fn is_block(&self) -> bool {
        match self {
            Element::Box { block, .. } => *block,
            Element::Outline(_) => false,
        }
    }
}

fn elements(solid: &WorldSolid) -> Vec<Element> {
    let boxed = |block: bool| move |region: WorldBox| Element::Box { region, block };
    solid
        .boxes()
        .iter()
        .copied()
        .map(boxed(true))
        .chain(
            solid
                .rods()
                .iter()
                .map(WorldRod::bounds)
                .chain(solid.slant_bounds())
                .map(boxed(false)),
        )
        .chain(
            solid
                .prisms()
                .iter()
                .map(|prism| Element::Outline(prism.corners_3d())),
        )
        .collect()
}

/// What a round hole along the plane's axis takes out of the pairs of
/// pieces (grammar, "Sweeps see cuts").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Bores {
    /// Nothing: the pieces will move some other way (a linkage), so such
    /// a hole is left as wood.
    Wood,
    /// The pieces stay where they are (a check at one pose): a piece
    /// inside a hole of the other solid is clear of the blocks the hole
    /// is drilled in.
    Still,
    /// One solid turns about the line through the plane's origin: a
    /// piece that stays inside a hole of the other solid all the way
    /// round is clear of the blocks the hole is drilled in. Exact for a
    /// hole on the line (taken out as itself); for one off it by `e`,
    /// only the round hole `e` narrower that every turn keeps covered is.
    Turning,
}

/// A round hole along the plane's axis, on the plane: its centre and
/// radius there and its range along the axis.
#[derive(Debug, Clone, Copy)]
struct Bore {
    centre: DVec2,
    radius: f64,
    low: f64,
    high: f64,
}

/// The holes of a solid moved by `matrix` that run along the plane's axis.
fn bores(plane: &Plane, solid: &WorldSolid, matrix: &DMat4) -> Vec<Bore> {
    let axis = point_vector(Point::default().with(plane.axis, Length::from_nm(1)));
    solid
        .holes()
        .iter()
        .filter_map(|hole| {
            let [first, second] = hole.axis.others();
            let end = |at: Length| {
                matrix.transform_point3(point_vector(
                    Point::default()
                        .with(hole.axis, at)
                        .with(first, hole.centre[0])
                        .with(second, hole.centre[1]),
                ))
            };
            let (start, finish) = (end(hole.start), end(hole.end));
            let direction = (finish - start).normalize_or_zero();
            // Only a hole along the axis is round on the plane.
            if direction.cross(axis).length() > 1e-9 {
                return None;
            }
            let (a, b) = (plane.along(start), plane.along(finish));
            Some(Bore {
                centre: plane.project(start),
                radius: render::nm(hole.diameter) / 2.0,
                low: a.min(b),
                high: a.max(b),
            })
        })
        .collect()
}

/// Whether `piece` lies inside one of `bores` over the range along the
/// axis it shares with `block` (a block of the solid they are drilled
/// in), at one pose or all the way round a turn as `rule` says. Touching
/// the hole's side, within the contact tolerance, is inside.
fn inside(piece: &Piece, block: &Piece, bores: &[Bore], rule: Bores) -> bool {
    let tolerance = render::nm(CONTACT_TOLERANCE);
    let (low, high) = (piece.low.max(block.low), piece.high.min(block.high));
    bores.iter().any(|bore| {
        let along = low >= bore.low - tolerance && high <= bore.high + tolerance;
        // The piece's corners are shrunk by half the tolerance.
        let room = bore.radius + tolerance / 2.0;
        along
            && match rule {
                Bores::Wood => false,
                Bores::Still => piece.radius_about(bore.centre) <= room,
                Bores::Turning => piece.radius_about(DVec2::ZERO) + bore.centre.length() <= room,
            }
    })
}

/// The eight corners of a box, world nanometres.
pub(crate) fn corners_of(region: &WorldBox) -> [DVec3; 8] {
    let (a, b) = (point_vector(region.min()), point_vector(region.max()));
    [
        DVec3::new(a.x, a.y, a.z),
        DVec3::new(b.x, a.y, a.z),
        DVec3::new(a.x, b.y, a.z),
        DVec3::new(b.x, b.y, a.z),
        DVec3::new(a.x, a.y, b.z),
        DVec3::new(b.x, a.y, b.z),
        DVec3::new(a.x, b.y, b.z),
        DVec3::new(b.x, b.y, b.z),
    ]
}

/// A box less the holes whose axis lies in the plane, each as its
/// narrowest chord over `range` along `axis` (rounded toward the hole, so
/// never more than the hole takes away).
fn cut_by_holes(
    region: WorldBox,
    holes: &[WorldRod],
    axis: Axis,
    range: (f64, f64),
) -> Vec<WorldBox> {
    let mut pieces = vec![region];
    for hole in holes.iter().filter(|hole| hole.axis != axis) {
        let Some(third) = hole.axis.third(axis) else {
            continue;
        };
        let [first, second] = hole.axis.others();
        let centre = |on: Axis| {
            if on == first {
                render::nm(hole.centre[0])
            } else if on == second {
                render::nm(hole.centre[1])
            } else {
                0.0
            }
        };
        let (centre_along, centre_third) = (centre(axis), centre(third));
        let radius = render::nm(hole.diameter) / 2.0;
        let farthest = (range.0 - centre_along)
            .abs()
            .max((range.1 - centre_along).abs());
        if farthest >= radius {
            continue;
        }
        let half = (radius * radius - farthest * farthest).sqrt();
        let (Some(inner), Some(outer)) = (
            render::length_from_nm((centre_third - half).ceil()),
            render::length_from_nm((centre_third + half).floor()),
        ) else {
            continue;
        };
        if inner >= outer {
            continue;
        }
        let mut next = Vec::new();
        for piece in pieces {
            let (low, high) = (piece.min().get(third), piece.max().get(third));
            let (start, end) = (piece.min().get(hole.axis), piece.max().get(hole.axis));
            let (cut_low, cut_high) = (inner.max(low), outer.min(high));
            let (cut_start, cut_end) = (hole.start.max(start), hole.end.min(end));
            if cut_low >= cut_high || cut_start >= cut_end {
                next.push(piece);
                continue;
            }
            let part = |a: Point, b: Point| WorldBox::new(a, b);
            if low < cut_low {
                next.push(part(piece.min(), piece.max().with(third, cut_low)));
            }
            if cut_high < high {
                next.push(part(piece.min().with(third, cut_high), piece.max()));
            }
            let strip_min = piece.min().with(third, cut_low);
            let strip_max = piece.max().with(third, cut_high);
            if start < cut_start {
                next.push(part(strip_min, strip_max.with(hole.axis, cut_start)));
            }
            if cut_end < end {
                next.push(part(strip_min.with(hole.axis, cut_end), strip_max));
            }
        }
        pieces = next;
    }
    pieces
}

/// A convex piece of world corners moved by `matrix`, on the plane.
fn piece(plane: &Plane, corners: &[DVec3], matrix: &DMat4) -> Option<Piece> {
    let moved: Vec<DVec3> = corners
        .iter()
        .map(|c| matrix.transform_point3(*c))
        .collect();
    let flat = hull(moved.iter().map(|c| plane.project(*c)).collect());
    let along = moved.iter().map(|c| plane.along(*c));
    let low = along.clone().fold(f64::INFINITY, f64::min);
    let high = along.fold(f64::NEG_INFINITY, f64::max);
    Some(Piece {
        corners: shrink(&flat, render::nm(CONTACT_TOLERANCE) / 2.0)?,
        low,
        high,
    })
}

/// The range along the plane's axis of an element moved by `matrix`.
fn element_range(plane: &Plane, element: &Element, matrix: &DMat4) -> (f64, f64) {
    let boxed;
    let corners: &[DVec3] = match element {
        Element::Box { region, .. } => {
            boxed = corners_of(region);
            &boxed
        }
        Element::Outline(corners) => corners,
    };
    corners
        .iter()
        .map(|c| plane.along(matrix.transform_point3(*c)))
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), v| {
            (lo.min(v), hi.max(v))
        })
}

/// An element's pieces moved by `matrix`, its boxes less its solid's holes
/// over `range` (world, along the axis, after the move).
fn element_pieces(
    plane: &Plane,
    element: &Element,
    holes: &[WorldRod],
    matrix: &DMat4,
    range: (f64, f64),
) -> Vec<Piece> {
    match element {
        Element::Box { region, .. } => {
            // The range in the solid's own place: a motion in the plane
            // only shifts what lies along the axis.
            let shift = plane.along(matrix.transform_point3(DVec3::ZERO));
            let own = (range.0 - shift, range.1 - shift);
            cut_by_holes(*region, holes, plane.axis, own)
                .iter()
                .filter_map(|part| piece(plane, &corners_of(part), matrix))
                .collect()
        }
        Element::Outline(corners) => piece(plane, corners, matrix).into_iter().collect(),
    }
}

/// Every pair of pieces of `a` (moved by `ma`) and `b` (moved by `mb`) that
/// share more than the contact tolerance along the axis, each solid's
/// boxes less its holes over the range the other piece covers, and
/// without the pairs of a piece and a block that a round hole along the
/// axis, drilled in the block, keeps apart as `rule` says.
pub(crate) fn piece_pairs(
    plane: &Plane,
    a: &WorldSolid,
    ma: &DMat4,
    b: &WorldSolid,
    mb: &DMat4,
    rule: Bores,
) -> Vec<(Piece, Piece)> {
    let tolerance = render::nm(CONTACT_TOLERANCE);
    let (ea, eb) = (elements(a), elements(b));
    let (bores_a, bores_b) = match rule {
        Bores::Wood => (Vec::new(), Vec::new()),
        Bores::Still | Bores::Turning => (bores(plane, a, ma), bores(plane, b, mb)),
    };
    let mut out = Vec::new();
    for x in &ea {
        let rx = element_range(plane, x, ma);
        for y in &eb {
            let ry = element_range(plane, y, mb);
            let shared = (rx.0.max(ry.0), rx.1.min(ry.1));
            if shared.1 - shared.0 <= tolerance {
                continue;
            }
            let px = element_pieces(plane, x, a.holes(), ma, shared);
            let py = element_pieces(plane, y, b.holes(), mb, shared);
            for p in &px {
                for q in &py {
                    if p.shared(q) <= tolerance
                        || (y.is_block() && inside(p, q, &bores_b, rule))
                        || (x.is_block() && inside(q, p, &bores_a, rule))
                    {
                        continue;
                    }
                    out.push((p.clone(), q.clone()));
                }
            }
        }
    }
    out
}

/// The convex hull of points, counter-clockwise (monotone chain).
pub(crate) fn hull(mut points: Vec<DVec2>) -> Vec<DVec2> {
    points.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));
    points.dedup_by(|a, b| (*a - *b).length_squared() < 1e-6);
    if points.len() < 3 {
        return points;
    }
    let cross = |o: DVec2, a: DVec2, b: DVec2| (a - o).perp_dot(b - o);
    let mut lower: Vec<DVec2> = Vec::new();
    for p in &points {
        while let [.., before, last] = lower.as_slice()
            && cross(*before, *last, *p) <= 0.0
        {
            lower.pop();
        }
        lower.push(*p);
    }
    let mut upper: Vec<DVec2> = Vec::new();
    for p in points.iter().rev() {
        while let [.., before, last] = upper.as_slice()
            && cross(*before, *last, *p) <= 0.0
        {
            upper.pop();
        }
        upper.push(*p);
    }
    lower.pop();
    upper.pop();
    lower.extend(upper);
    lower
}

/// The edges of a polygon, each from a corner to the next.
fn edges(polygon: &[DVec2]) -> impl Iterator<Item = (DVec2, DVec2)> + '_ {
    polygon
        .iter()
        .zip(polygon.iter().cycle().skip(1))
        .map(|(a, b)| (*a, *b))
}

/// A convex polygon moved `inset` inward on every edge; `None` when
/// nothing is left.
pub(crate) fn shrink(corners: &[DVec2], inset: f64) -> Option<Vec<DVec2>> {
    if corners.len() < 3 {
        return None;
    }
    // Each edge as a line moved inward (left of the edge, for CCW).
    let lines: Vec<(DVec2, DVec2)> = edges(corners)
        .map(|(a, b)| {
            let direction = (b - a).normalize_or_zero();
            (a + direction.perp() * inset, direction)
        })
        .collect();
    let mut out = Vec::with_capacity(lines.len());
    let previous = lines.iter().cycle().skip(lines.len() - 1);
    for ((p, d), (q, e)) in previous.zip(lines.iter()) {
        let denominator = d.perp_dot(*e);
        if denominator.abs() < 1e-12 {
            return None;
        }
        let t = (*q - *p).perp_dot(*e) / denominator;
        out.push(*p + *d * t);
    }
    // Still counter-clockwise with a positive area?
    let area: f64 = edges(&out).map(|(a, b)| a.perp_dot(b)).sum();
    (area > 0.0).then_some(out)
}

/// The shadow of a polygon on a direction.
fn shadow(points: &[DVec2], normal: DVec2) -> (f64, f64) {
    points
        .iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), p| {
            let v = p.dot(normal);
            (lo.min(v), hi.max(v))
        })
}

/// Do two convex polygons overlap (the separating axis test)?
pub(crate) fn intersect(a: &[DVec2], b: &[DVec2]) -> bool {
    [a, b].into_iter().all(|polygon| {
        edges(polygon).all(|(from, to)| {
            let normal = (to - from).perp();
            let (a_lo, a_hi) = shadow(a, normal);
            let (b_lo, b_hi) = shadow(b, normal);
            a_hi > b_lo && b_hi > a_lo
        })
    })
}

/// The distance from a point to a segment.
fn point_segment(p: DVec2, a: DVec2, b: DVec2) -> f64 {
    let ab = b - a;
    let length2 = ab.length_squared();
    let t = if length2 > 0.0 {
        ((p - a).dot(ab) / length2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (a + ab * t - p).length()
}

/// The distance between two convex polygons that don't overlap.
pub(crate) fn polygon_distance(a: &[DVec2], b: &[DVec2]) -> f64 {
    let mut best = f64::INFINITY;
    for (polygon, other) in [(a, b), (b, a)] {
        for p in polygon {
            for (from, to) in edges(other) {
                best = best.min(point_segment(*p, from, to));
            }
        }
    }
    best
}

/// How far two overlapping convex polygons would have to move apart: the
/// smallest overlap of their shadows over every edge normal, with that
/// normal.
pub(crate) fn overlap_depth(a: &[DVec2], b: &[DVec2]) -> (f64, DVec2) {
    let mut best = (f64::INFINITY, DVec2::X);
    for polygon in [a, b] {
        for (from, to) in edges(polygon) {
            let normal = (to - from).perp().normalize_or_zero();
            let (a_lo, a_hi) = shadow(a, normal);
            let (b_lo, b_hi) = shadow(b, normal);
            let overlap = a_hi.min(b_hi) - a_lo.max(b_lo);
            if overlap < best.0 {
                best = (overlap, normal);
            }
        }
    }
    (best.0.max(0.0), best.1)
}

/// How deep two pieces overlap, as they would without the half tolerance
/// each was shrunk by: the smaller of the depth in the plane and the
/// range they share along the axis; `None` when they don't overlap.
pub(crate) fn piece_depth(a: &Piece, b: &Piece) -> Option<(f64, DVec2)> {
    if !intersect(&a.corners, &b.corners) {
        return None;
    }
    let (depth, normal) = overlap_depth(&a.corners, &b.corners);
    let tolerance = render::nm(CONTACT_TOLERANCE);
    Some(((depth + tolerance).min(a.shared(b)), normal))
}
