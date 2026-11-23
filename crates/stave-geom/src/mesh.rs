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
//! Each hole's square (on the two axes across it, a little larger than the
//! hole) is added to the grid, so where the hole opens on a face its square
//! is one face cell. That
//! cell is drawn as a square minus the circle (fans to its four corners),
//! the hole's wall as an inward-facing cylinder, a bottom disc where it
//! stops inside the part, and the opening as an outline circle. Grid lines
//! from cuts elsewhere on the board (a mortise in line with a row of holes)
//! may split the square into several cells; when every cell of it is the
//! same at each layer along the hole, all wood or all air, the hole is
//! drawn over them as over one, its square's sides fanned through the
//! grid points on them. When a cut edge does cross the square (two holes
//! that overlap, a cut edge across a hole), the face is drawn uncut there
//! and the hole is not drawn; the solid itself is unaffected. A hole opens on every face its square
//! reaches through (a notch floor too), and its wall is drawn only where
//! it passes through wood.
//!
//! # Angled cuts
//!
//! A board whose angled cuts ([`Prism`](crate::solid::Prism)s) all run along
//! one axis, with no holes or round features, is drawn by
//! [`mesh_prism`](crate::mesh_prism): cells and slabs instead of a grid,
//! watertight. Otherwise the grid is drawn and each slanted piece clips the
//! faces it reaches, its walls drawn through each wood cell ([`mesh_cut`]).
//!
//! # Round features
//!
//! Round tenons and round sections on a board are lathe cylinders; a
//! round section with flats is drawn as the prism of its outline
//! ([`mesh_cut`]). Round stock and turned parts
//! are lathe meshes of their profile, drilled where they have holes
//! ([`mesh_drill`](crate::mesh_drill)).
//!
//! # Cut faces
//!
//! Every vertex says whether its face was made by a cut
//! ([`Vertex::cut`](crate::Vertex::cut)): a board's faces off its cut box,
//! hole walls and bottoms, the walls and floors of angled cuts, round tenons and round
//! sections, and the zones of a turned part a round tenon narrowed.

use std::f32::consts::TAU;

use glam::Vec3;
use stave_lang::placement::{PartAxis, PerPartAxis};
use stave_lang::units::Length;
use stave_model::solved::Part;

use crate::axes::{AlongPart, following, on_part};
use crate::build::{self, Builder, Frame, GrainAxes, ProfilePoint};
use crate::mesh_cut::{self, Band};
use crate::solid::{Rod, Solid, across};
use crate::{Aabb, Edges, Geometry, Mesh, render};

fn mm_nm(nm: i64) -> f32 {
    render::mm(Length::from_nm(nm))
}

/// The mesh of a part with its cuts made, built in `frame` (the part's
/// frame in the render frame, or its own frame).
pub fn cut_part(frame: &Frame, part: &Part, solid: &Solid, grain: GrainAxes) -> Geometry {
    // Slanted holes are drilled in the part's own frame, where its
    // coordinates are small, and the result moved into place: drilled in
    // place, far from the origin, the points where a hole crosses a face
    // would come out a little different in floats.
    let own = Frame::own(crate::handedness(part));
    if !solid.slants().is_empty() && own != *frame {
        let local = cut_part(&own, part, solid, grain);
        return moved(local, own.onto(frame));
    }
    let frame = *frame;
    let size = PerPartAxis::from_fn(|axis| part.cut.get(axis).nm());
    let length = render::mm(part.cut.length);
    let mut pieces = Vec::new();
    if !solid.blocks().is_empty() {
        match crate::mesh_prism::shared_axis(solid) {
            Some(axis) => pieces.push(crate::mesh_prism::board(&frame, solid, grain, size, axis)),
            None => pieces.push(board(&frame, solid, grain, size)),
        }
    }
    for rod in solid.rods() {
        if rod.flats.is_some() {
            let mut builder = Builder::default();
            builder.set_cut(true);
            mesh_cut::flatted_rod(&mut builder, &frame, grain, rod, segments());
            pieces.push(builder.finish());
        } else {
            pieces.push(rod_mesh(&frame, rod, grain, length));
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
        let zones = build::CutZones {
            zones: profile
                .narrowed
                .iter()
                .map(|(start, end)| (render::mm(*start), render::mm(*end)))
                .collect(),
            ends: Some((0.0, length)),
        };
        let mut turned = build::lathe_zoned(&frame, centre, &points, corners, grain, &zones);
        for hole in solid.holes() {
            crate::mesh_drill::drill(&mut turned, &frame, grain, hole);
        }
        pieces.push(turned);
    }
    let mut geometry = merge(pieces);
    for slant in solid.slants() {
        crate::mesh_drill::drill_slant(&mut geometry, &frame, grain, slant);
    }
    geometry
}

/// A geometry moved by a transform with no reflection in it.
fn moved(mut geometry: Geometry, transform: glam::Mat4) -> Geometry {
    for vertex in &mut geometry.mesh.vertices {
        vertex.position = transform
            .transform_point3(Vec3::from(vertex.position))
            .to_array();
        vertex.normal = transform
            .transform_vector3(Vec3::from(vertex.normal))
            .normalize_or_zero()
            .to_array();
    }
    for segment in &mut geometry.edges.segments {
        for point in segment.iter_mut() {
            *point = transform.transform_point3(Vec3::from(*point)).to_array();
        }
    }
    geometry.bounds = geometry.bounds.transformed(transform);
    geometry
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

/// A round tenon or round section: a cylinder along its axis, a cut face
/// except where it ends at the part's own end (`length` along the part).
fn rod_mesh(frame: &Frame, rod: &Rod, grain: GrainAxes, length: f32) -> Geometry {
    let axial = frame.first(rod.axis);
    let radius = render::mm(rod.diameter) * 0.5;
    let (start, end) = (render::mm(rod.start), render::mm(rod.end));
    let profile = [
        ProfilePoint::new(start, radius),
        ProfilePoint::new(end, radius),
    ];
    let centre = [render::mm(rod.centre[0]), render::mm(rod.centre[1])];
    let zones = build::CutZones {
        zones: vec![(start, end)],
        ends: Some((0.0, length)),
    };
    build::lathe_zoned(
        &axial,
        centre,
        &profile,
        build::Corners::Sharp,
        grain,
        &zones,
    )
}

/// A hole's square and where it opens, in grid terms.
struct HoleShape {
    rod: Rod,
    /// The square on the two axes across the hole: (low, high) per axis, nm.
    square: [(i64, i64); 2],
}

impl HoleShape {
    /// The square is a little larger than the hole (by an eighth of its
    /// radius each way), so the circle never touches its sides: a point
    /// of the circle on a side would sit in the middle of the neighbouring
    /// face's edge, and the mesh would crack there.
    fn of(rod: &Rod) -> HoleShape {
        let radius = rod.diameter.nm() / 2;
        let rest = rod.diameter.nm().saturating_sub(radius);
        let margin = radius / 8;
        let square = rod.centre.map(|c| {
            (
                c.nm().saturating_sub(radius).saturating_sub(margin),
                c.nm().saturating_add(rest).saturating_add(margin),
            )
        });
        HoleShape { rod: *rod, square }
    }
}

/// The grid of a board.
struct Grid {
    /// Grid lines per local axis, nm, sorted and distinct.
    lines: PerPartAxis<Vec<i64>>,
    /// Wood or air per cell, length fastest.
    wood: Vec<bool>,
}

impl Grid {
    fn cells(&self, axis: PartAxis) -> usize {
        self.lines[axis].len().saturating_sub(1)
    }

    /// Cells in the whole grid.
    fn count(&self) -> usize {
        PartAxis::ALL.into_iter().fold(1, |count: usize, axis| {
            count.saturating_mul(self.cells(axis))
        })
    }

    /// A cell's place in [`Grid::wood`]; `None` outside the grid.
    fn index(&self, cell: PerPartAxis<usize>) -> Option<usize> {
        let [length, width, thickness] = cell.into_array();
        let (cells_length, cells_width) =
            (self.cells(PartAxis::Length), self.cells(PartAxis::Width));
        let row = thickness.checked_mul(cells_width)?.checked_add(width)?;
        row.checked_mul(cells_length)?.checked_add(length)
    }

    /// Whether a cell is wood; cells outside the grid are air.
    fn is_wood(&self, cell: PerPartAxis<isize>) -> bool {
        let Ok(at) = PerPartAxis::try_from_fn(|axis| {
            usize::try_from(cell[axis])
                .ok()
                .filter(|value| *value < self.cells(axis))
                .ok_or(())
        }) else {
            return false;
        };
        self.index(at)
            .and_then(|index| self.wood.get(index))
            .copied()
            .unwrap_or(false)
    }

    fn line(&self, axis: PartAxis, index: usize) -> i64 {
        self.lines[axis].get(index).copied().unwrap_or(0)
    }

    fn find(&self, axis: PartAxis, value: i64) -> Option<usize> {
        self.lines[axis].binary_search(&value).ok()
    }

    /// Every layer along `axis` of the column of cells `u.1` by `v.1`
    /// (cell ranges on the axes `u.0` and `v.0`, ends exclusive) is all
    /// wood or all air.
    fn uniform_column(
        &self,
        axis: PartAxis,
        u: (PartAxis, (usize, usize)),
        v: (PartAxis, (usize, usize)),
    ) -> bool {
        let (u_axis, (u0, u1)) = u;
        let (v_axis, (v0, v1)) = v;
        (0..self.cells(axis)).all(|k| {
            let mut first: PerPartAxis<isize> = PerPartAxis::default();
            first[axis] = signed(k);
            first[u_axis] = signed(u0);
            first[v_axis] = signed(v0);
            let wood = self.is_wood(first);
            (u0..u1).all(|i| {
                (v0..v1).all(|j| {
                    let mut cell = first;
                    cell[u_axis] = signed(i);
                    cell[v_axis] = signed(j);
                    self.is_wood(cell) == wood
                })
            })
        })
    }
}

/// A grid index as a signed cell coordinate, so a neighbour one below the
/// first cell is outside rather than a wrapped number.
fn signed(index: usize) -> isize {
    isize::try_from(index).unwrap_or(isize::MAX)
}

/// The boundary of a board's blocks, with its holes and angled slots.
/// `size` is the cut size in nanometres per part axis: faces on its box
/// are the stock's own surface, every other face was made by a cut.
fn board(frame: &Frame, solid: &Solid, grain: GrainAxes, size: PerPartAxis<i64>) -> Geometry {
    let stock_plane = |axis: PartAxis, at: i64| at == 0 || size[axis] == at;
    let holes: Vec<HoleShape> = solid.holes().iter().map(HoleShape::of).collect();
    let bands: Vec<Band> = solid
        .prisms()
        .iter()
        .flat_map(|prism| {
            prism
                .slanted_pieces()
                .map(move |piece| Band::of(prism, piece))
        })
        .collect();
    let mut lines: PerPartAxis<Vec<i64>> = PerPartAxis::default();
    for block in solid.blocks().iter() {
        for axis in PartAxis::ALL {
            lines[axis].push(block.min().get(axis).nm());
            lines[axis].push(block.max().get(axis).nm());
        }
    }
    for hole in &holes {
        for (axis, (low, high)) in across(hole.rod.axis).into_iter().zip(hole.square) {
            lines[axis].extend([low, high]);
        }
    }
    for axis in PartAxis::ALL {
        lines[axis].sort_unstable();
        lines[axis].dedup();
    }
    let mut grid = Grid {
        wood: Vec::new(),
        lines,
    };
    grid.wood = vec![false; grid.count()];
    for block in solid.blocks().iter() {
        let mut range: PerPartAxis<(usize, usize)> = PerPartAxis::default();
        for axis in PartAxis::ALL {
            let (Some(low), Some(high)) = (
                grid.find(axis, block.min().get(axis).nm()),
                grid.find(axis, block.max().get(axis).nm()),
            ) else {
                continue;
            };
            range[axis] = (low, high);
        }
        let ((x0, x1), (y0, y1), (z0, z1)) = (
            range[PartAxis::Length],
            range[PartAxis::Width],
            range[PartAxis::Thickness],
        );
        for z in z0..z1 {
            for y in y0..y1 {
                for x in x0..x1 {
                    if let Some(cell) = grid
                        .index(PerPartAxis::new(x, y, z))
                        .and_then(|index| grid.wood.get_mut(index))
                    {
                        *cell = true;
                    }
                }
            }
        }
    }

    let mut builder = Builder::default();
    let local = |at: PerPartAxis<i64>| {
        let [x, y, z] = at.into_array().map(mm_nm);
        Vec3::new(x, y, z)
    };

    // Hole openings, with the hole each belongs to.
    let mut openings: Vec<(Opening, usize)> = Vec::new();
    let mut drawn_holes = vec![false; holes.len()];
    for (index, hole) in holes.iter().enumerate() {
        let a = hole.rod.axis;
        let [u_axis, v_axis] = across(a);
        let (Some(u0), Some(u1), Some(v0), Some(v1)) = (
            grid.find(u_axis, hole.square[0].0),
            grid.find(u_axis, hole.square[0].1),
            grid.find(v_axis, hole.square[1].0),
            grid.find(v_axis, hole.square[1].1),
        ) else {
            continue;
        };
        // Grid lines from cuts elsewhere on the board may split its
        // square into several cells. That is harmless when every cell of
        // the square is the same as the others at each layer along the
        // hole, all wood or all air: the hole is drawn over them as over
        // one cell. Otherwise something cuts across it, and it is not drawn.
        let (us, vs) = ((u0, u1), (v0, v1));
        if u1 <= u0 || v1 <= v0 || !grid.uniform_column(a, (u_axis, us), (v_axis, vs)) {
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
            let pi = signed(p);
            for high in [false, true] {
                let mut inside: PerPartAxis<isize> = PerPartAxis::default();
                inside[u_axis] = signed(u0);
                inside[v_axis] = signed(v0);
                let mut outside = inside;
                inside[a] = if high { pi - 1 } else { pi };
                outside[a] = if high { pi } else { pi - 1 };
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
                    let mut at: PerPartAxis<usize> = PerPartAxis::default();
                    at[a] = p;
                    at[u_axis] = u0;
                    at[v_axis] = v0;
                    let mut end: PerPartAxis<usize> = PerPartAxis::default();
                    end[a] = p;
                    end[u_axis] = u1;
                    end[v_axis] = v1;
                    openings.push((
                        Opening {
                            axis: a,
                            at,
                            end,
                            high,
                        },
                        index,
                    ));
                }
            }
        }
        // Its square is one cell on its faces, so it is drawn.
        if let Some(drawn) = drawn_holes.get_mut(index) {
            *drawn = true;
        }
    }

    // Faces, merged per plane.
    for a in PartAxis::ALL {
        let [u, v] = following(a);
        let (nu, nv) = (grid.cells(u), grid.cells(v));
        for p in 0..grid.lines[a].len() {
            for positive in [false, true] {
                // mask[i + nu*j]: a face to draw here.
                let mut mask = vec![false; nu * nv];
                for j in 0..nv {
                    for i in 0..nu {
                        let mut below: PerPartAxis<isize> = PerPartAxis::default();
                        let pi = signed(p);
                        below[a] = pi - 1;
                        below[u] = signed(i);
                        below[v] = signed(j);
                        let mut above = below;
                        above[a] = pi;
                        let (wood_below, wood_above) = (grid.is_wood(below), grid.is_wood(above));
                        let face = if positive {
                            wood_below && !wood_above
                        } else {
                            wood_above && !wood_below
                        };
                        let is_opening = openings.iter().any(|(opening, _)| {
                            opening.axis == a
                                && opening.at[a] == p
                                && (opening.at[u]..opening.end[u]).contains(&i)
                                && (opening.at[v]..opening.end[v]).contains(&j)
                        });
                        if face
                            && !is_opening
                            && let Some(m) = mask.get_mut(i + nu * j)
                        {
                            *m = true;
                        }
                    }
                }
                let normal = on_part(a, if positive { 1.0 } else { -1.0 });
                let world_normal = frame.vector(normal);
                let w = grid.line(a, p);
                builder.set_cut(!stock_plane(a, w));
                for (i0, j0, i1, j1) in greedy(&mut mask, nu, nv) {
                    let corner = |i: usize, j: usize| {
                        let mut c: PerPartAxis<i64> = PerPartAxis::default();
                        c[a] = w;
                        c[u] = grid.line(u, i);
                        c[v] = grid.line(v, j);
                        local(c)
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
                        let [q0, q1, q2, q3] = [
                            corner(i0, j0),
                            corner(i1, j0),
                            corner(i1, j1),
                            corner(i0, j1),
                        ]
                        .map(|l| builder.vertex(frame.point(l), world_normal, grain.apply(l)));
                        builder.triangle(q0, q1, q2);
                        builder.triangle(q0, q2, q3);
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
    for a in PartAxis::ALL {
        let [u, v] = following(a);
        for i in 0..grid.lines[u].len() {
            for j in 0..grid.lines[v].len() {
                let mut run: Option<usize> = None;
                for k in 0..=grid.cells(a) {
                    let feature = k < grid.cells(a) && {
                        let around = |di: isize, dj: isize| {
                            let mut cell: PerPartAxis<isize> = PerPartAxis::default();
                            cell[a] = signed(k);
                            cell[u] = signed(i).saturating_add(di);
                            cell[v] = signed(j).saturating_add(dj);
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
                                let mut c: PerPartAxis<i64> = PerPartAxis::default();
                                c[a] = grid.line(a, at);
                                c[u] = grid.line(u, i);
                                c[v] = grid.line(v, j);
                                frame.point(local(c))
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
    for (place, index) in &openings {
        let Some(hole) = holes.get(*index) else {
            continue;
        };
        let axis = place.axis;
        builder.set_cut(!stock_plane(axis, grid.line(axis, place.at[axis])));
        opening(&mut builder, frame, grain, hole, *place, &grid, &bands);
    }
    builder.set_cut(true);
    for (index, hole) in holes.iter().enumerate() {
        if !drawn_holes.get(index).copied().unwrap_or(false) {
            continue;
        }
        let a = hole.rod.axis;
        let [u_axis, v_axis] = across(a);
        let (Some(u0), Some(v0)) = (
            grid.find(u_axis, hole.square[0].0),
            grid.find(v_axis, hole.square[1].0),
        ) else {
            continue;
        };
        let (start, end) = (hole.rod.start.nm(), hole.rod.end.nm());
        let cell = |k: isize| {
            let mut c: PerPartAxis<isize> = PerPartAxis::default();
            c[a] = k;
            c[u_axis] = signed(u0);
            c[v_axis] = signed(v0);
            c
        };
        for k in 0..grid.cells(a) {
            let (cell_low, cell_high) = (grid.line(a, k), grid.line(a, k + 1));
            let ki = signed(k);
            let (low, high) = (cell_low.max(start), cell_high.min(end));
            if low >= high || !grid.is_wood(cell(ki)) {
                continue;
            }
            // The hole stops in wood here: a bottom facing into it.
            let stops_low = low == start && (start > cell_low || grid.is_wood(cell(ki - 1)));
            let stops_high =
                high == end && (end < cell_high || grid.is_wood(cell(ki.saturating_add(1))));
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
            .filter(|h| h.rod.axis == band.face)
            .map(|h| {
                (
                    mesh_cut::flat_centre(&h.rod),
                    render::mm(h.rod.diameter) * 0.5,
                )
            })
            .collect();
        for z in 0..grid.cells(PartAxis::Thickness) {
            for y in 0..grid.cells(PartAxis::Width) {
                for x in 0..grid.cells(PartAxis::Length) {
                    let cell = PerPartAxis::new(x, y, z);
                    let at = cell.map(signed);
                    if !grid.is_wood(at) {
                        continue;
                    }
                    let low = local(PerPartAxis::from_fn(|axis| grid.line(axis, cell[axis])));
                    let high = local(PerPartAxis::from_fn(|axis| grid.line(axis, cell[axis] + 1)));
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
    let [u, v] = across(hole.axis);
    let mut point = on_part(hole.axis, along);
    point.set_along(u, render::mm(hole.centre[0]) + radius * angle.cos());
    point.set_along(v, render::mm(hole.centre[1]) + radius * angle.sin());
    point
}

fn segments() -> u16 {
    build::LATHE_SEGMENTS
}

/// The angle of a step around a hole; the last step is the first again,
/// exactly, so a ring closes without a crack.
fn angle_of(step: u16) -> f32 {
    TAU * f32::from(step % segments().max(1)) / f32::from(segments())
}

/// Where a hole opens, and which way it faces.
#[derive(Debug, Clone, Copy)]
struct Opening {
    /// The face axis: the hole's own.
    axis: PartAxis,
    /// The grid plane it opens on (along `axis`) and the first cell of
    /// the hole's square (along the two axes across it).
    at: PerPartAxis<usize>,
    /// The cells of the hole's square end before these (along the two
    /// axes across it; along `axis`, the plane again).
    end: PerPartAxis<usize>,
    /// It faces toward the high end of `axis`.
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
    let a = place.axis;
    let along = mm_nm(grid.line(a, place.at[a]));
    let radius = render::mm(rod.diameter) * 0.5;
    let [u, v] = across(rod.axis);
    let normal = on_part(a, if place.high { 1.0 } else { -1.0 });
    let corner_local = |hi_u: bool, hi_v: bool| {
        let mut point = on_part(a, along);
        point.set_along(
            u,
            mm_nm(if hi_u {
                hole.square[0].1
            } else {
                hole.square[0].0
            }),
        );
        point.set_along(
            v,
            mm_nm(if hi_v {
                hole.square[1].1
            } else {
                hole.square[1].0
            }),
        );
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
    // Each side of the square, from the circle's point nearest it: a fan
    // through every grid point on the side, which the neighbouring faces
    // share, so there is no crack where grid lines split the square.
    for q in 0..4u16 {
        let at = usize::from(q * quarter);
        let before = corners.get(usize::from((q + 3) % 4));
        let after = corners.get(usize::from(q));
        let (Some(&point), Some(&b), Some(&c)) = (ring.get(at), before, after) else {
            continue;
        };
        // The side runs along v for q = 0 and 2, along u for 1 and 3.
        let (side_axis, side) = if q % 2 == 0 {
            (v, hole.square[1])
        } else {
            (u, hole.square[0])
        };
        let mut inner: Vec<i64> = grid.lines[side_axis]
            .iter()
            .copied()
            .filter(|&line| line > side.0 && line < side.1)
            .collect();
        // From `before` to `after`: up the side for q = 0 and 3, down it
        // for 1 and 2.
        if q == 1 || q == 2 {
            inner.reverse();
        }
        let mut path = vec![b];
        path.extend(inner.into_iter().map(|line| {
            let mut on_side = b;
            on_side.set_along(side_axis, mm_nm(line));
            on_side
        }));
        path.push(c);
        for pair in path.windows(2) {
            if let &[from, to] = pair {
                emit(builder, vec![point, from, to]);
            }
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
    let a = rod.axis;
    let [u, v] = across(rod.axis);
    let inward = |angle: f32| {
        let mut n = on_part(u, -angle.cos());
        n.set_along(v, -angle.sin());
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
        let [q0, q1, q2, q3] = [
            corner(start, t0),
            corner(start, t1),
            corner(end, t1),
            corner(end, t0),
        ];
        builder.triangle(q0, q1, q2);
        builder.triangle(q0, q2, q3);
    }
    // A bottom where the hole stops: facing back toward the opening.
    for (along, stop, toward_high) in [(start, stops.0, true), (end, stops.1, false)] {
        if !stop {
            continue;
        }
        let normal = on_part(a, if toward_high { 1.0 } else { -1.0 });
        let world_normal = frame.vector(normal);
        let mut centre = circle_point(rod, along, 0.0, 0.0);
        centre.set_along(a, along);
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
