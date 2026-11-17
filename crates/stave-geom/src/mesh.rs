//! Meshes of cut solids ([`Solid`]): the boundary of a board's blocks, its
//! holes, round tenons and round sections, and the lathe profile of round
//! and turned parts.
//!
//! # Boards
//!
//! A board's blocks are disjoint, so their union is drawn from a grid: every
//! distinct block coordinate along each axis is a grid line, each grid cell
//! is wood or air, and a face is drawn between a wood cell and an air cell
//! (or the outside). Coplanar faces facing the same way are merged greedily
//! into rectangles, so a flat face is a few large quads with no seams where
//! blocks meet. Outline edges are the grid edges where the surface bends:
//! where one or three of the four cells around the edge are wood, or two
//! diagonal ones; collinear pieces are joined.
//!
//! # Holes
//!
//! Each hole's bounding square (on the two axes across it) is added to the
//! grid, so where the hole opens on a face its square is one face cell. That
//! cell is drawn as a square minus the circle (fans to its four corners),
//! the hole's wall as an inward-facing cylinder, a bottom disc where it
//! stops inside the part, and the opening as an outline circle. When other
//! grid lines split the square (two holes that overlap, a cut edge across a
//! hole), the face is drawn uncut there and the hole is not drawn; the
//! solid itself is unaffected. A hole opens on every face its square
//! reaches through (a notch floor too), and its wall is drawn only where
//! it passes through wood.
//!
//! # Slots turned by an angle
//!
//! Faces are clipped to the outside of the slot and its walls drawn
//! through each wood cell ([`mesh_cut`]).
//!
//! # Round features
//!
//! Round tenons and round sections on a board are lathe cylinders; a
//! round section with flats is drawn as the prism of its outline
//! ([`mesh_cut`]). Round stock and turned parts
//! are lathe meshes of their profile; holes in them are not drawn (what
//! fills them hides them).

use std::f32::consts::TAU;

use glam::Vec3;
use stave_lang::placement::PartAxis;
use stave_lang::units::Length;
use stave_model::solved::Part;

use crate::build::{self, Builder, Frame, GrainAxes, ProfilePoint};
use crate::mesh_cut::{self, Band};
use crate::solid::{Rod, Solid, across};
use crate::{Aabb, Edges, Geometry, Mesh, render};

/// The slot of a part axis in a local vector.
fn slot(axis: PartAxis) -> usize {
    match axis {
        PartAxis::Length => 0,
        PartAxis::Width => 1,
        PartAxis::Thickness => 2,
    }
}

fn mm_nm(nm: i64) -> f32 {
    render::mm(Length::from_nm(nm))
}

/// The mesh of a part with its cuts made.
pub fn cut_part(part: &Part, solid: &Solid, grain: GrainAxes) -> Geometry {
    let frame = Frame::of_part(part);
    let mut pieces = Vec::new();
    if !solid.blocks().is_empty() {
        pieces.push(board(&frame, solid, grain));
    }
    for rod in solid.rods() {
        if rod.flats.is_some() {
            let mut builder = Builder::default();
            mesh_cut::flatted_rod(&mut builder, &frame, grain, rod, segments());
            pieces.push(builder.finish());
        } else {
            pieces.push(rod_mesh(&frame, rod, grain));
        }
    }
    if let Some(profile) = solid.profile() {
        let mut points: Vec<ProfilePoint> = profile
            .stations
            .iter()
            .map(|s| ProfilePoint::new(render::mm(s.at), render::mm(s.diameter) * 0.5))
            .collect();
        let corners = if profile.smooth {
            points = build::smooth_profile(&points);
            build::Corners::ByAngle
        } else {
            build::Corners::Sharp
        };
        let centre = [
            render::mm(part.cut.width) * 0.5,
            render::mm(part.cut.thickness) * 0.5,
        ];
        pieces.push(build::lathe(&frame, centre, &points, corners, grain));
    }
    merge(pieces)
}

/// Joins geometries into one.
pub(crate) fn merge(pieces: Vec<Geometry>) -> Geometry {
    let mut mesh = Mesh::default();
    let mut edges = Edges::default();
    let mut bounds: Option<Aabb> = None;
    for piece in pieces {
        if piece.mesh.vertices.is_empty() {
            continue;
        }
        let offset = u32::try_from(mesh.vertices.len()).unwrap_or(u32::MAX);
        mesh.vertices.extend(piece.mesh.vertices);
        mesh.indices
            .extend(piece.mesh.indices.iter().map(|i| i.saturating_add(offset)));
        edges.segments.extend(piece.edges.segments);
        bounds = Some(bounds.map_or(piece.bounds, |b| b.union(piece.bounds)));
    }
    Geometry {
        mesh,
        edges,
        bounds: bounds.unwrap_or(Aabb {
            min: Vec3::ZERO,
            max: Vec3::ZERO,
        }),
    }
}

/// A round tenon or round section: a cylinder along its axis.
fn rod_mesh(frame: &Frame, rod: &Rod, grain: GrainAxes) -> Geometry {
    let axial = frame.first(rod.axis);
    let radius = render::mm(rod.diameter) * 0.5;
    let profile = [
        ProfilePoint::new(render::mm(rod.start), radius),
        ProfilePoint::new(render::mm(rod.end), radius),
    ];
    let centre = [render::mm(rod.centre[0]), render::mm(rod.centre[1])];
    build::lathe(&axial, centre, &profile, build::Corners::Sharp, grain)
}

/// A hole's square and where it opens, in grid terms.
struct HoleShape {
    rod: Rod,
    /// The square on the two axes across the hole: (low, high) per axis, nm.
    square: [(i64, i64); 2],
}

impl HoleShape {
    fn of(rod: &Rod) -> HoleShape {
        let radius = rod.diameter.nm() / 2;
        let rest = rod.diameter.nm() - radius;
        let square = rod.centre.map(|c| (c.nm() - radius, c.nm() + rest));
        HoleShape { rod: *rod, square }
    }
}

/// The grid of a board.
struct Grid {
    /// Grid lines per local axis, nm, sorted and distinct.
    lines: [Vec<i64>; 3],
    /// Wood or air per cell, x fastest.
    wood: Vec<bool>,
}

impl Grid {
    fn cells(&self, axis: usize) -> usize {
        self.lines[axis].len().saturating_sub(1)
    }

    fn index(&self, cell: [usize; 3]) -> usize {
        cell[0] + self.cells(0) * (cell[1] + self.cells(1) * cell[2])
    }

    /// Whether a cell is wood; cells outside the grid are air.
    fn is_wood(&self, cell: [isize; 3]) -> bool {
        let mut at = [0usize; 3];
        for axis in 0..3 {
            match usize::try_from(cell[axis]) {
                Ok(value) if value < self.cells(axis) => at[axis] = value,
                _ => return false,
            }
        }
        self.wood.get(self.index(at)).copied().unwrap_or(false)
    }

    fn line(&self, axis: usize, index: usize) -> i64 {
        self.lines[axis].get(index).copied().unwrap_or(0)
    }

    fn find(&self, axis: usize, value: i64) -> Option<usize> {
        self.lines[axis].binary_search(&value).ok()
    }
}

fn board(frame: &Frame, solid: &Solid, grain: GrainAxes) -> Geometry {
    let holes: Vec<HoleShape> = solid.holes().iter().map(HoleShape::of).collect();
    let bands: Vec<Band> = solid.slots().iter().map(Band::of).collect();
    let mut lines: [Vec<i64>; 3] = Default::default();
    for block in solid.blocks().iter() {
        for axis in PartAxis::ALL {
            lines[slot(axis)].push(block.min().get(axis).nm());
            lines[slot(axis)].push(block.max().get(axis).nm());
        }
    }
    for hole in &holes {
        for (index, axis) in across(hole.rod.axis).into_iter().enumerate() {
            let (low, high) = hole.square[index];
            lines[slot(axis)].extend([low, high]);
        }
    }
    for axis_lines in &mut lines {
        axis_lines.sort_unstable();
        axis_lines.dedup();
    }
    let mut grid = Grid {
        wood: Vec::new(),
        lines,
    };
    grid.wood = vec![false; grid.cells(0) * grid.cells(1) * grid.cells(2)];
    for block in solid.blocks().iter() {
        let mut range = [(0usize, 0usize); 3];
        for axis in PartAxis::ALL {
            let s = slot(axis);
            let (Some(low), Some(high)) = (
                grid.find(s, block.min().get(axis).nm()),
                grid.find(s, block.max().get(axis).nm()),
            ) else {
                continue;
            };
            range[s] = (low, high);
        }
        for z in range[2].0..range[2].1 {
            for y in range[1].0..range[1].1 {
                for x in range[0].0..range[0].1 {
                    let index = grid.index([x, y, z]);
                    if let Some(cell) = grid.wood.get_mut(index) {
                        *cell = true;
                    }
                }
            }
        }
    }

    let mut builder = Builder::default();
    let local = |x: i64, y: i64, z: i64| Vec3::new(mm_nm(x), mm_nm(y), mm_nm(z));

    // Hole openings: (axis slot, plane index, cell u, cell v) -> hole index.
    let mut openings: Vec<([usize; 4], usize, bool)> = Vec::new();
    let mut drawn_holes = vec![false; holes.len()];
    for (index, hole) in holes.iter().enumerate() {
        let a = slot(hole.rod.axis);
        let [u_axis, v_axis] = across(hole.rod.axis).map(slot);
        let (Some(u0), Some(u1), Some(v0), Some(v1)) = (
            grid.find(u_axis, hole.square[0].0),
            grid.find(u_axis, hole.square[0].1),
            grid.find(v_axis, hole.square[1].0),
            grid.find(v_axis, hole.square[1].1),
        ) else {
            continue;
        };
        if u1 != u0 + 1 || v1 != v0 + 1 {
            continue;
        }
        // Every plane across the hole within its range where its square
        // goes from wood to air is an opening.
        let (start, end) = (hole.rod.start.nm(), hole.rod.end.nm());
        for p in 0..grid.lines[a].len() {
            let plane = grid.line(a, p);
            if plane < start || plane > end {
                continue;
            }
            let pi = isize::try_from(p).unwrap_or(0);
            for high in [false, true] {
                let mut inside = [0isize; 3];
                let mut outside = [0isize; 3];
                inside[a] = if high { pi - 1 } else { pi };
                outside[a] = if high { pi } else { pi - 1 };
                for (axis, cell) in [(u_axis, u0), (v_axis, v0)] {
                    let value = isize::try_from(cell).unwrap_or(0);
                    inside[axis] = value;
                    outside[axis] = value;
                }
                // The inside cell must lie within the hole's range.
                let Ok(inside_index) = usize::try_from(inside[a]) else {
                    continue;
                };
                let (inside_low, inside_high) =
                    (grid.line(a, inside_index), grid.line(a, inside_index + 1));
                if inside_high <= start || inside_low >= end {
                    continue;
                }
                if grid.is_wood(inside) && !grid.is_wood(outside) {
                    openings.push(([a, p, u0, v0], index, high));
                }
            }
        }
        // Its square is one cell on its faces, so it is drawn.
        if let Some(drawn) = drawn_holes.get_mut(index) {
            *drawn = true;
        }
    }

    // Faces, merged per plane.
    for a in 0..3 {
        let (u, v) = ((a + 1) % 3, (a + 2) % 3);
        let (nu, nv) = (grid.cells(u), grid.cells(v));
        for p in 0..grid.lines[a].len() {
            for positive in [false, true] {
                // mask[i + nu*j]: a face to draw here.
                let mut mask = vec![false; nu * nv];
                for j in 0..nv {
                    for i in 0..nu {
                        let mut below = [0isize; 3];
                        let pi = isize::try_from(p).unwrap_or(0);
                        below[a] = pi - 1;
                        below[u] = isize::try_from(i).unwrap_or(0);
                        below[v] = isize::try_from(j).unwrap_or(0);
                        let mut above = below;
                        above[a] = pi;
                        let (wood_below, wood_above) = (grid.is_wood(below), grid.is_wood(above));
                        let face = if positive {
                            wood_below && !wood_above
                        } else {
                            wood_above && !wood_below
                        };
                        let is_opening = openings.iter().any(|(key, _, _)| {
                            key[0] == a && key[1] == p && {
                                let [hu, hv] = cell_uv(key, a);
                                hu == (u, i) && hv == (v, j)
                            }
                        });
                        if face
                            && !is_opening
                            && let Some(m) = mask.get_mut(i + nu * j)
                        {
                            *m = true;
                        }
                    }
                }
                let mut normal = Vec3::ZERO;
                normal[a] = if positive { 1.0 } else { -1.0 };
                let world_normal = frame.vector(normal);
                let w = grid.line(a, p);
                for (i0, j0, i1, j1) in greedy(&mut mask, nu, nv) {
                    let corner = |i: usize, j: usize| {
                        let mut c = [0i64; 3];
                        c[a] = w;
                        c[u] = grid.line(u, i);
                        c[v] = grid.line(v, j);
                        local(c[0], c[1], c[2])
                    };
                    let polygon = vec![
                        corner(i0, j0),
                        corner(i1, j0),
                        corner(i1, j1),
                        corner(i0, j1),
                    ];
                    if mesh_cut::cuts(&bands, a, &polygon) {
                        mesh_cut::face(&mut builder, frame, grain, &bands, a, polygon, normal);
                        continue;
                    }
                    if i1 == i0 + 1 && j1 == j0 + 1 {
                        let quad = [
                            corner(i0, j0),
                            corner(i1, j0),
                            corner(i1, j1),
                            corner(i0, j1),
                        ]
                        .map(|l| builder.vertex(frame.point(l), world_normal, grain.apply(l)));
                        builder.triangle(quad[0], quad[1], quad[2]);
                        builder.triangle(quad[0], quad[2], quad[3]);
                        continue;
                    }
                    // A merged rectangle keeps every grid point on its
                    // border, so neighbouring faces share their vertices
                    // (no T-junctions, which crack when rasterised); it is
                    // fanned from its middle.
                    let mut ring: Vec<(usize, usize)> = Vec::new();
                    ring.extend((i0..i1).map(|i| (i, j0)));
                    ring.extend((j0..j1).map(|j| (i1, j)));
                    ring.extend((i0 + 1..=i1).rev().map(|i| (i, j1)));
                    ring.extend((j0 + 1..=j1).rev().map(|j| (i0, j)));
                    let middle_local = (corner(i0, j0) + corner(i1, j1)) * 0.5;
                    let middle = builder.vertex(
                        frame.point(middle_local),
                        world_normal,
                        grain.apply(middle_local),
                    );
                    let border: Vec<u32> = ring
                        .iter()
                        .map(|&(i, j)| {
                            let l = corner(i, j);
                            builder.vertex(frame.point(l), world_normal, grain.apply(l))
                        })
                        .collect();
                    for index in 0..border.len() {
                        if let (Some(&from), Some(&to)) =
                            (border.get(index), border.get((index + 1) % border.len()))
                        {
                            builder.triangle(middle, from, to);
                        }
                    }
                }
            }
        }
    }

    // Outline edges where the surface bends.
    for a in 0..3 {
        let (u, v) = ((a + 1) % 3, (a + 2) % 3);
        for i in 0..grid.lines[u].len() {
            for j in 0..grid.lines[v].len() {
                let mut run: Option<usize> = None;
                for k in 0..=grid.cells(a) {
                    let feature = k < grid.cells(a) && {
                        let around = |di: isize, dj: isize| {
                            let mut cell = [0isize; 3];
                            cell[a] = isize::try_from(k).unwrap_or(0);
                            cell[u] = isize::try_from(i).unwrap_or(0) + di;
                            cell[v] = isize::try_from(j).unwrap_or(0) + dj;
                            grid.is_wood(cell)
                        };
                        let cells = [around(-1, -1), around(0, -1), around(-1, 0), around(0, 0)];
                        let count = cells.iter().filter(|c| **c).count();
                        count == 1 || count == 3 || (count == 2 && cells[0] == cells[3])
                    };
                    match (feature, run) {
                        (true, None) => run = Some(k),
                        (false, Some(start)) => {
                            let point = |at: usize| {
                                let mut c = [0i64; 3];
                                c[a] = grid.line(a, at);
                                c[u] = grid.line(u, i);
                                c[v] = grid.line(v, j);
                                frame.point(local(c[0], c[1], c[2]))
                            };
                            builder.edge(point(start), point(k));
                            run = None;
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    // Openings, walls and bottoms of the holes.
    for (key, index, high) in &openings {
        let Some(hole) = holes.get(*index) else {
            continue;
        };
        let place = Opening {
            key: *key,
            high: *high,
        };
        opening(&mut builder, frame, grain, hole, place, &grid, &bands);
    }
    for (index, hole) in holes.iter().enumerate() {
        if !drawn_holes.get(index).copied().unwrap_or(false) {
            continue;
        }
        let a = slot(hole.rod.axis);
        let [u_axis, v_axis] = across(hole.rod.axis).map(slot);
        let (Some(u0), Some(v0)) = (
            grid.find(u_axis, hole.square[0].0),
            grid.find(v_axis, hole.square[1].0),
        ) else {
            continue;
        };
        let (start, end) = (hole.rod.start.nm(), hole.rod.end.nm());
        let cell = |k: isize| {
            let mut c = [0isize; 3];
            c[a] = k;
            c[u_axis] = isize::try_from(u0).unwrap_or(0);
            c[v_axis] = isize::try_from(v0).unwrap_or(0);
            c
        };
        for k in 0..grid.cells(a) {
            let (cell_low, cell_high) = (grid.line(a, k), grid.line(a, k + 1));
            let ki = isize::try_from(k).unwrap_or(0);
            let (low, high) = (cell_low.max(start), cell_high.min(end));
            if low >= high || !grid.is_wood(cell(ki)) {
                continue;
            }
            // The hole stops in wood here: a bottom facing into it.
            let stops_low = low == start && (start > cell_low || grid.is_wood(cell(ki - 1)));
            let stops_high = high == end && (end < cell_high || grid.is_wood(cell(ki + 1)));
            wall(
                &mut builder,
                frame,
                grain,
                hole,
                (mm_nm(low), mm_nm(high)),
                (stops_low, stops_high),
                &bands,
            );
        }
    }
    // The walls of slots turned by an angle, through every wood cell.
    for band in &bands {
        let hole_discs: Vec<(Vec3, f32)> = holes
            .iter()
            .filter(|h| slot(h.rod.axis) == band.face)
            .map(|h| {
                (
                    mesh_cut::flat_centre(&h.rod),
                    render::mm(h.rod.diameter) * 0.5,
                )
            })
            .collect();
        for z in 0..grid.cells(2) {
            for y in 0..grid.cells(1) {
                for x in 0..grid.cells(0) {
                    let at = [x, y, z].map(|i| isize::try_from(i).unwrap_or(0));
                    if !grid.is_wood(at) {
                        continue;
                    }
                    let low = local(grid.line(0, x), grid.line(1, y), grid.line(2, z));
                    let high = local(
                        grid.line(0, x + 1),
                        grid.line(1, y + 1),
                        grid.line(2, z + 1),
                    );
                    let mut below = at;
                    below[band.face] -= 1;
                    let mut above = at;
                    above[band.face] += 1;
                    mesh_cut::walls(
                        &mut builder,
                        frame,
                        grain,
                        band,
                        (low, high),
                        &hole_discs,
                        (!grid.is_wood(below), !grid.is_wood(above)),
                    );
                }
            }
        }
    }
    builder.finish()
}

/// The (axis, cell) pairs of an opening key on the face axes `u`, `v` of
/// face axis `a`.
fn cell_uv(key: &[usize; 4], a: usize) -> [(usize, usize); 2] {
    // The key stores the cells on `across(axis)` in PartAxis order.
    let axes = match a {
        0 => [1, 2],
        1 => [0, 2],
        _ => [0, 1],
    };
    let (u, v) = ((a + 1) % 3, (a + 2) % 3);
    let cell_of = |axis: usize| {
        if axis == axes[0] { key[2] } else { key[3] }
    };
    [(u, cell_of(u)), (v, cell_of(v))]
}

/// Greedy merge of a mask into rectangles `(i0, j0, i1, j1)`, exclusive
/// ends. Clears the mask.
fn greedy(mask: &mut [bool], nu: usize, nv: usize) -> Vec<(usize, usize, usize, usize)> {
    let at = |i: usize, j: usize| i + nu * j;
    let mut out = Vec::new();
    for j in 0..nv {
        let mut i = 0;
        while i < nu {
            if !mask.get(at(i, j)).copied().unwrap_or(false) {
                i += 1;
                continue;
            }
            let mut i1 = i + 1;
            while i1 < nu && mask.get(at(i1, j)).copied().unwrap_or(false) {
                i1 += 1;
            }
            let mut j1 = j + 1;
            while j1 < nv && (i..i1).all(|x| mask.get(at(x, j1)).copied().unwrap_or(false)) {
                j1 += 1;
            }
            for y in j..j1 {
                for x in i..i1 {
                    if let Some(m) = mask.get_mut(at(x, y)) {
                        *m = false;
                    }
                }
            }
            out.push((i, j, i1, j1));
            i = i1;
        }
    }
    out
}

/// Local position of a point on a hole's circle at plane `along` (mm).
fn circle_point(hole: &Rod, along: f32, angle: f32, radius: f32) -> Vec3 {
    let [u, v] = across(hole.axis).map(slot);
    let mut point = Vec3::ZERO;
    point[slot(hole.axis)] = along;
    point[u] = render::mm(hole.centre[0]) + radius * angle.cos();
    point[v] = render::mm(hole.centre[1]) + radius * angle.sin();
    point
}

fn segments() -> u16 {
    build::LATHE_SEGMENTS
}

fn angle_of(step: u16) -> f32 {
    TAU * f32::from(step) / f32::from(segments())
}

/// Where a hole opens: (face axis, plane index, cell u, cell v), and
/// which way it faces.
#[derive(Debug, Clone, Copy)]
struct Opening {
    key: [usize; 4],
    high: bool,
}

/// The face cell of a hole's square, minus its circle (and any slot), and
/// the opening's outline.
fn opening(
    builder: &mut Builder,
    frame: &Frame,
    grain: GrainAxes,
    hole: &HoleShape,
    place: Opening,
    grid: &Grid,
    bands: &[Band],
) {
    let rod = &hole.rod;
    let a = place.key[0];
    let along = mm_nm(grid.line(a, place.key[1]));
    let radius = render::mm(rod.diameter) * 0.5;
    let [u, v] = across(rod.axis).map(slot);
    let mut normal = Vec3::ZERO;
    normal[a] = if place.high { 1.0 } else { -1.0 };
    let corner_local = |hi_u: bool, hi_v: bool| {
        let mut point = Vec3::ZERO;
        point[a] = along;
        point[u] = mm_nm(if hi_u {
            hole.square[0].1
        } else {
            hole.square[0].0
        });
        point[v] = mm_nm(if hi_v {
            hole.square[1].1
        } else {
            hole.square[1].0
        });
        point
    };
    let corners = [
        corner_local(true, true),
        corner_local(false, true),
        corner_local(false, false),
        corner_local(true, false),
    ];
    let quarter = segments() / 4;
    let ring: Vec<Vec3> = (0..=segments())
        .map(|step| circle_point(rod, along, angle_of(step), radius))
        .collect();
    let emit = |builder: &mut Builder, polygon: Vec<Vec3>| {
        mesh_cut::face(builder, frame, grain, bands, a, polygon, normal);
    };
    for step in 0..segments() {
        let quadrant = usize::from(step / quarter.max(1)).min(3);
        if let (Some(&a0), Some(&a1), Some(&corner)) = (
            ring.get(usize::from(step)),
            ring.get(usize::from(step) + 1),
            corners.get(quadrant),
        ) {
            emit(builder, vec![a0, a1, corner]);
        }
    }
    for q in 0..4u16 {
        let at = usize::from(q * quarter);
        let before = corners.get(usize::from((q + 3) % 4));
        let after = corners.get(usize::from(q));
        if let (Some(&point), Some(&b), Some(&c)) = (ring.get(at), before, after) {
            emit(builder, vec![point, b, c]);
        }
    }
    for step in 0..segments() {
        let (from, to) = (
            circle_point(rod, along, angle_of(step), radius),
            circle_point(rod, along, angle_of(step + 1), radius),
        );
        if mesh_cut::in_any(bands, a, (from + to) * 0.5) {
            continue;
        }
        builder.edge(frame.point(from), frame.point(to));
    }
}

/// A hole's wall (facing into the hole) over `range` along its axis, less
/// what a slot opens, and a bottom at either end where it stops in wood.
fn wall(
    builder: &mut Builder,
    frame: &Frame,
    grain: GrainAxes,
    hole: &HoleShape,
    range: (f32, f32),
    stops: (bool, bool),
    bands: &[Band],
) {
    let rod = &hole.rod;
    let radius = render::mm(rod.diameter) * 0.5;
    let (start, end) = range;
    let a = slot(rod.axis);
    let [u, v] = across(rod.axis).map(slot);
    let inward = |angle: f32| {
        let mut n = Vec3::ZERO;
        n[u] = -angle.cos();
        n[v] = -angle.sin();
        n
    };
    for step in 0..segments() {
        let (t0, t1) = (angle_of(step), angle_of(step + 1));
        let middle = circle_point(rod, (start + end) * 0.5, (t0 + t1) * 0.5, radius);
        if mesh_cut::in_any(bands, a, middle) {
            continue;
        }
        let mut corner = |along: f32, angle: f32| {
            let l = circle_point(rod, along, angle, radius);
            builder.vertex(frame.point(l), frame.vector(inward(angle)), grain.apply(l))
        };
        let quad = [
            corner(start, t0),
            corner(start, t1),
            corner(end, t1),
            corner(end, t0),
        ];
        builder.triangle(quad[0], quad[1], quad[2]);
        builder.triangle(quad[0], quad[2], quad[3]);
    }
    // A bottom where the hole stops: facing back toward the opening.
    for (along, stop, toward_high) in [(start, stops.0, true), (end, stops.1, false)] {
        if !stop {
            continue;
        }
        let mut normal = Vec3::ZERO;
        normal[a] = if toward_high { 1.0 } else { -1.0 };
        let world_normal = frame.vector(normal);
        let mut centre = circle_point(rod, along, 0.0, 0.0);
        centre[a] = along;
        let middle = builder.vertex(frame.point(centre), world_normal, grain.apply(centre));
        let rim: Vec<u32> = (0..=segments())
            .map(|step| {
                let l = circle_point(rod, along, angle_of(step), radius);
                builder.vertex(frame.point(l), world_normal, grain.apply(l))
            })
            .collect();
        for pair in rim.windows(2) {
            if let &[x, y] = pair {
                builder.triangle(middle, x, y);
            }
        }
    }
}
