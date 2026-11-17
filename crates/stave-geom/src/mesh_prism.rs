//! Meshes of boards with angled cuts ([`Prism`]s) all along one axis.
//!
//! The plane across that axis is cut into convex **cells** by every line
//! a side lies on: the sides of the blocks and of the prisms' pieces,
//! each extended right across the board. Along the axis, the board is cut
//! into **slabs** at every block face and every prism's ends. Within one
//! slab each cell is wood or air throughout, so the surface is made of:
//!
//! - **walls**: a cell side between wood and air (or the outside), over a
//!   slab;
//! - **caps**: a cell where wood ends from one slab to the next.
//!
//! Every cell is split by a line that crosses it, so a point a line adds
//! on a side is added to both cells that share the side: no point ever
//! lies in the middle of another cell's side, and the mesh is watertight
//! (every edge belongs to exactly two triangles) wherever the solid is a
//! manifold. Cells are fanned from their middle, so a cell with points
//! along a straight side has no zero-area triangles.
//!
//! Points are whole nanometres. Where a line crosses a side between two
//! points, the crossing is worked out once, from the side's two ends in a
//! fixed order, and rounded to the nanometre, so both cells get the same
//! point. A point within [`ON_LINE`] of a line counts as on it, which
//! keeps slivers out. This is drawing only; the solid and its checks are
//! exact.

use std::collections::{BTreeMap, BTreeSet};

use glam::Vec3;
use stave_lang::placement::PartAxis;
use stave_lang::units::Length;

use crate::Geometry;
use crate::build::{Builder, Frame, GrainAxes};
use crate::render;
use crate::solid::{ConvexPolygon, Point2, Prism, Solid, across};

/// A point this close to a line (nanometres) is on it.
const ON_LINE: f64 = 2_000.0;

/// A point of the plane across the prisms' axis, nanometres.
type Flat = [i64; 2];

/// The line `a x + b y = c`, in lowest terms with `(a, b)` pointing to
/// its positive side.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Line {
    a: i128,
    b: i128,
    c: i128,
}

fn gcd(mut x: i128, mut y: i128) -> i128 {
    x = x.abs();
    y = y.abs();
    while y != 0 {
        let rest = x % y;
        x = y;
        y = rest;
    }
    x
}

impl Line {
    /// The line through two distinct points.
    fn through(p: Flat, q: Flat) -> Option<Line> {
        let a = i128::from(q[1]) - i128::from(p[1]);
        let b = i128::from(p[0]) - i128::from(q[0]);
        if a == 0 && b == 0 {
            return None;
        }
        let c = a * i128::from(p[0]) + b * i128::from(p[1]);
        let g = gcd(gcd(a, b), c).max(1);
        let sign = if a < 0 || (a == 0 && b < 0) { -1 } else { 1 };
        Some(Line {
            a: sign * a / g,
            b: sign * b / g,
            c: sign * c / g,
        })
    }

    /// Which side of the line a point is on: `0` within [`ON_LINE`].
    fn side(&self, p: Flat) -> i8 {
        let value = self.a * i128::from(p[0]) + self.b * i128::from(p[1]) - self.c;
        let reach = ON_LINE * norm(self.a, self.b);
        let value_f = wide(value);
        if value_f.abs() <= reach {
            0
        } else if value > 0 {
            1
        } else {
            -1
        }
    }

    fn value(&self, p: Flat) -> f64 {
        wide(self.a * i128::from(p[0]) + self.b * i128::from(p[1]) - self.c)
    }
}

/// An `i128` as the nearest `f64`, by its two halves (no cast).
fn wide(value: i128) -> f64 {
    let high = i64::try_from(value >> 64).unwrap_or(if value < 0 { i64::MIN } else { i64::MAX });
    let low = u64::try_from(value & 0xFFFF_FFFF_FFFF_FFFF).unwrap_or(0);
    let high = render::nm(Length::from_nm(high));
    let low_high = u32::try_from(low >> 32).unwrap_or(0);
    let low_low = u32::try_from(low & 0xFFFF_FFFF).unwrap_or(0);
    high * 18_446_744_073_709_551_616.0 + f64::from(low_high) * 4_294_967_296.0 + f64::from(low_low)
}

fn norm(a: i128, b: i128) -> f64 {
    (wide(a).powi(2) + wide(b).powi(2)).sqrt()
}

/// Where the side `u w` crosses `line`, from its ends in a fixed order,
/// to the nanometre.
fn crossing(u: Flat, w: Flat, line: &Line) -> Option<Flat> {
    let (p, q) = if u <= w { (u, w) } else { (w, u) };
    let (vp, vq) = (line.value(p), line.value(q));
    let t = vp / (vp - vq);
    let at = |from: i64, to: i64| {
        let (from, to) = (
            render::nm(Length::from_nm(from)),
            render::nm(Length::from_nm(to)),
        );
        render::length_from_nm(from + (to - from) * t).map(Length::nm)
    };
    let ([p0, p1], [q0, q1]) = (p, q);
    Some([at(p0, q0)?, at(p1, q1)?])
}

/// Twice the signed area of a ring.
fn twice_area(ring: &[Flat]) -> i128 {
    let mut sum = 0i128;
    for (index, p) in ring.iter().enumerate() {
        if let Some(q) = ring.get((index + 1) % ring.len()) {
            sum += i128::from(p[0]) * i128::from(q[1]) - i128::from(q[0]) * i128::from(p[1]);
        }
    }
    sum
}

/// A ring with repeated neighbours dropped; `None` when it has no area.
fn tidy(mut ring: Vec<Flat>) -> Option<Vec<Flat>> {
    ring.dedup();
    while ring.len() > 1 && ring.first() == ring.last() {
        ring.pop();
    }
    (ring.len() >= 3 && twice_area(&ring) > 0).then_some(ring)
}

/// A convex cell split by a line: the parts on each side, when the line
/// crosses it.
fn split(cell: &[Flat], line: &Line) -> Option<(Vec<Flat>, Vec<Flat>)> {
    let sides: Vec<i8> = cell.iter().map(|p| line.side(*p)).collect();
    if !sides.contains(&1) || !sides.contains(&-1) {
        return None;
    }
    let mut below = Vec::new();
    let mut above = Vec::new();
    let count = cell.len();
    for index in 0..count {
        let (Some(&u), Some(&w), Some(&su), Some(&sw)) = (
            cell.get(index),
            cell.get((index + 1) % count),
            sides.get(index),
            sides.get((index + 1) % count),
        ) else {
            continue;
        };
        if su <= 0 {
            below.push(u);
        }
        if su >= 0 {
            above.push(u);
        }
        if su * sw < 0 {
            let x = crossing(u, w, line)?;
            below.push(x);
            above.push(x);
        }
    }
    Some((tidy(below)?, tidy(above)?))
}

/// The middle of a cell (the average of its points), in nanometres.
fn middle(cell: &[Flat]) -> [f64; 2] {
    let count = f64::from(u32::try_from(cell.len()).unwrap_or(1).max(1));
    let mut sum = [0.0, 0.0];
    for p in cell {
        for (i, total) in sum.iter_mut().enumerate() {
            *total += p.get(i).map_or(0.0, |v| render::nm(Length::from_nm(*v)));
        }
    }
    sum.map(|s| s / count)
}

fn flat(point: Point2) -> Flat {
    [point.first.nm(), point.second.nm()]
}

/// Does a convex piece hold a point strictly inside?
fn piece_holds(piece: &ConvexPolygon, at: [f64; 2]) -> bool {
    let points = piece.points();
    let count = points.len();
    (0..count).all(|i| match (points.get(i), points.get((i + 1) % count)) {
        (Some(p), Some(q)) => {
            let (px, py) = (render::nm(p.first), render::nm(p.second));
            let (qx, qy) = (render::nm(q.first), render::nm(q.second));
            (qx - px) * (at[1] - py) - (qy - py) * (at[0] - px) > 0.0
        }
        _ => false,
    })
}

fn slot(axis: PartAxis) -> usize {
    match axis {
        PartAxis::Length => 0,
        PartAxis::Width => 1,
        PartAxis::Thickness => 2,
    }
}

/// The face axis every prism of a solid shares, when it is a board with
/// no holes or round features and has at least one prism.
pub(crate) fn shared_axis(solid: &Solid) -> Option<PartAxis> {
    if !solid.holes().is_empty() || !solid.rods().is_empty() || solid.profile().is_some() {
        return None;
    }
    let first = solid.prisms().first()?.face_axis();
    solid
        .prisms()
        .iter()
        .all(|prism| prism.face_axis() == first)
        .then_some(first)
}

/// What lies on each side of a wall: which way it faces, as the cell
/// whose wood it bounds (`true`: the cell that runs along the side
/// counter-clockwise).
type WallKind = Option<bool>;

/// The mesh of a board whose prisms all run along `axis` (see
/// [`shared_axis`]). `size` is the cut size in nanometres per local axis.
pub(crate) fn board(
    frame: &Frame,
    solid: &Solid,
    grain: GrainAxes,
    size: [i64; 3],
    axis: PartAxis,
) -> Geometry {
    let [first, second] = across(axis);
    let (fs, ss, cs) = (slot(first), slot(second), slot(axis));
    let extent = [
        size.get(fs).copied().unwrap_or(0),
        size.get(ss).copied().unwrap_or(0),
    ];
    let depth = size.get(cs).copied().unwrap_or(0);
    let prisms: Vec<&Prism> = solid.prisms().iter().collect();

    // Lines, in a fixed order.
    let mut lines: BTreeSet<Line> = BTreeSet::new();
    for block in solid.blocks().iter() {
        let (low, high) = (block.min(), block.max());
        for (index, part_axis) in [first, second].into_iter().enumerate() {
            for value in [low.get(part_axis).nm(), high.get(part_axis).nm()] {
                let (p, q) = if index == 0 {
                    ([value, 0], [value, 1])
                } else {
                    ([0, value], [1, value])
                };
                lines.extend(Line::through(p, q));
            }
        }
    }
    for prism in &prisms {
        let outline = prism.outline().points();
        let sides = outline.iter().zip(outline.iter().cycle().skip(1));
        lines.extend(sides.filter_map(|(p, q)| Line::through(flat(*p), flat(*q))));
        for piece in prism.slanted_pieces() {
            let points = piece.points();
            let sides = points.iter().zip(points.iter().cycle().skip(1));
            lines.extend(sides.filter_map(|(p, q)| Line::through(flat(*p), flat(*q))));
        }
    }

    // Cells.
    let mut cells: Vec<Vec<Flat>> = vec![vec![
        [0, 0],
        [extent[0], 0],
        [extent[0], extent[1]],
        [0, extent[1]],
    ]];
    for line in &lines {
        let mut next = Vec::with_capacity(cells.len() + 4);
        for cell in cells {
            match split(&cell, line) {
                Some((below, above)) => {
                    next.push(below);
                    next.push(above);
                }
                None => next.push(cell),
            }
        }
        cells = next;
    }

    // Slabs.
    let mut levels: BTreeSet<i64> = BTreeSet::from([0, depth]);
    for block in solid.blocks().iter() {
        levels.insert(block.min().get(axis).nm());
        levels.insert(block.max().get(axis).nm());
    }
    for prism in &prisms {
        levels.insert(prism.depth().from().nm().clamp(0, depth));
        levels.insert(prism.depth().to().nm().clamp(0, depth));
    }
    let levels: Vec<i64> = levels.into_iter().collect();
    let slabs: Vec<(i64, i64)> = levels
        .windows(2)
        .filter_map(|w| match w {
            [low, high] if low < high => Some((*low, *high)),
            _ => None,
        })
        .collect();

    // Wood or air, per slab and cell.
    let middles: Vec<[f64; 2]> = cells.iter().map(|cell| middle(cell)).collect();
    let wood: Vec<Vec<bool>> = slabs
        .iter()
        .map(|(low, high)| {
            let z = (render::nm(Length::from_nm(*low)) + render::nm(Length::from_nm(*high))) / 2.0;
            middles
                .iter()
                .map(|at| {
                    let in_block = solid.blocks().iter().any(|block| {
                        let within = |part_axis: PartAxis, value: f64| {
                            value >= render::nm(block.min().get(part_axis))
                                && value <= render::nm(block.max().get(part_axis))
                        };
                        within(first, at[0]) && within(second, at[1]) && within(axis, z)
                    });
                    in_block
                        && !prisms.iter().any(|prism| {
                            z > render::nm(prism.depth().from())
                                && z < render::nm(prism.depth().to())
                                && prism.slanted_pieces().any(|piece| piece_holds(piece, *at))
                        })
                })
                .collect()
        })
        .collect();
    let is_wood = |slab: usize, cell: usize| {
        wood.get(slab)
            .and_then(|row| row.get(cell))
            .copied()
            .unwrap_or(false)
    };

    // The cells on each side.
    let mut sides: BTreeMap<(Flat, Flat), Vec<(usize, bool)>> = BTreeMap::new();
    for (index, cell) in cells.iter().enumerate() {
        let count = cell.len();
        for i in 0..count {
            let (Some(&u), Some(&w)) = (cell.get(i), cell.get((i + 1) % count)) else {
                continue;
            };
            let key = if u <= w { (u, w) } else { (w, u) };
            sides.entry(key).or_default().push((index, u <= w));
        }
    }

    let local = |p: Flat, z: i64| {
        let mut v = Vec3::ZERO;
        v[fs] = render::mm(Length::from_nm(p[0]));
        v[ss] = render::mm(Length::from_nm(p[1]));
        v[cs] = render::mm(Length::from_nm(z));
        v
    };
    let on_edge = |p: Flat, q: Flat| {
        (p[0] == q[0] && (p[0] == 0 || p[0] == extent[0]))
            || (p[1] == q[1] && (p[1] == 0 || p[1] == extent[1]))
    };
    let mut builder = Builder::default();

    // Walls, and what each side is in each slab.
    let mut kinds: BTreeMap<(Flat, Flat), Vec<WallKind>> = BTreeMap::new();
    for (&(p, q), owners) in &sides {
        let mut row = Vec::with_capacity(slabs.len());
        for (s, (z0, z1)) in slabs.iter().enumerate() {
            // `forward` cell: the one that runs p → q counter-clockwise.
            let forward = owners.iter().find(|(_, f)| *f).map(|(c, _)| is_wood(s, *c));
            let backward = owners
                .iter()
                .find(|(_, f)| !*f)
                .map(|(c, _)| is_wood(s, *c));
            let kind = match (forward.unwrap_or(false), backward.unwrap_or(false)) {
                (true, false) => Some(true),
                (false, true) => Some(false),
                (true, true) | (false, false) => None,
            };
            row.push(kind);
            let Some(forward_wood) = kind else {
                continue;
            };
            // Outward: to the right of the way the wood's cell runs.
            let (from, to) = if forward_wood { (p, q) } else { (q, p) };
            let mut normal = Vec3::ZERO;
            let run = [
                render::mm(Length::from_nm(to[0].saturating_sub(from[0]))),
                render::mm(Length::from_nm(to[1].saturating_sub(from[1]))),
            ];
            normal[fs] = run[1];
            normal[ss] = -run[0];
            let world = frame.vector(normal);
            builder.set_cut(!on_edge(p, q));
            let quad = [(p, *z0), (q, *z0), (q, *z1), (p, *z1)].map(|(point, z)| {
                let l = local(point, z);
                builder.vertex(frame.point(l), world, grain.apply(l))
            });
            builder.triangle(quad[0], quad[1], quad[2]);
            builder.triangle(quad[0], quad[2], quad[3]);
        }
        kinds.insert((p, q), row);
    }

    // Caps.
    for (k, level) in levels.iter().enumerate() {
        let below_slab = k.checked_sub(1);
        let above_slab = (k < slabs.len()).then_some(k);
        builder.set_cut(*level != 0 && *level != depth);
        for (index, cell) in cells.iter().enumerate() {
            let below = below_slab.is_some_and(|s| is_wood(s, index));
            let above = above_slab.is_some_and(|s| is_wood(s, index));
            if below == above {
                continue;
            }
            let mut normal = Vec3::ZERO;
            normal[cs] = if below { 1.0 } else { -1.0 };
            let world = frame.vector(normal);
            let at = middles.get(index).copied().unwrap_or([0.0, 0.0]);
            let mut centre = Vec3::ZERO;
            centre[fs] = render::narrow(at[0] / 1e6);
            centre[ss] = render::narrow(at[1] / 1e6);
            centre[cs] = render::mm(Length::from_nm(*level));
            let m = builder.vertex(frame.point(centre), world, grain.apply(centre));
            let ring: Vec<u32> = cell
                .iter()
                .map(|p| {
                    let l = local(*p, *level);
                    builder.vertex(frame.point(l), world, grain.apply(l))
                })
                .collect();
            for i in 0..ring.len() {
                if let (Some(&x), Some(&y)) = (ring.get(i), ring.get((i + 1) % ring.len())) {
                    builder.triangle(m, x, y);
                }
            }
        }
    }

    // Outline edges: where a side's wall starts, stops or turns round
    // from one slab to the next, and where the walls meeting at a point
    // don't run straight on.
    for (&(p, q), row) in &kinds {
        for (k, level) in levels.iter().enumerate() {
            let below = k.checked_sub(1).and_then(|s| row.get(s).copied()).flatten();
            let above = row.get(k).copied().flatten();
            if below != above {
                builder.edge(frame.point(local(p, *level)), frame.point(local(q, *level)));
            }
        }
    }
    for (s, (z0, z1)) in slabs.iter().enumerate() {
        let mut meeting: BTreeMap<Flat, Vec<Flat>> = BTreeMap::new();
        for (&(p, q), row) in &kinds {
            if row.get(s).copied().flatten().is_some() {
                meeting.entry(p).or_default().push(q);
                meeting.entry(q).or_default().push(p);
            }
        }
        for (point, ends) in &meeting {
            let straight = match ends.as_slice() {
                [x, y] => {
                    let d1 = [
                        i128::from(x[0]) - i128::from(point[0]),
                        i128::from(x[1]) - i128::from(point[1]),
                    ];
                    let d2 = [
                        i128::from(y[0]) - i128::from(point[0]),
                        i128::from(y[1]) - i128::from(point[1]),
                    ];
                    d1[0] * d2[1] - d1[1] * d2[0] == 0 && d1[0] * d2[0] + d1[1] * d2[1] < 0
                }
                _ => false,
            };
            if !straight {
                builder.edge(
                    frame.point(local(*point, *z0)),
                    frame.point(local(*point, *z1)),
                );
            }
        }
    }
    builder.finish()
}
