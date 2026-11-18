//! How much wood a cut leaves round it (grammar, "Walls"), and whether a
//! cut stays within its part.
//!
//! Every cut is taken as the shapes it removes, in its part's own frame:
//! a `drill` is a cylinder per hole, a `pocket` or `notch` a box, a
//! `tenon` the boxes round it, a `spigot` or `round` the whole section
//! over its length, a `slot` along one of the part's axes a box. The
//! **wall** round a cut is the least distance from those shapes to a face
//! of the part it isn't meant to open onto, or to the shapes of another
//! cut of the same part.
//!
//! - A cut always opens onto the face it is cut from, and a `through` cut
//!   onto the opposite one too; those are never walls. A **closed** cut
//!   (`drill`, `pocket`) is held to every other face, so one that runs out
//!   of a face it wasn't aimed at has no wall there. Any other cut opens
//!   wherever it reaches a face (a groove runs out of the ends of the
//!   board), so only the faces it stays clear of are walls.
//! - On round stock and turned parts the faces across the length are the
//!   lathe profile: a hole into the side of a leg is measured from the
//!   bottom of the hole to the round surface (its mouth breaks the surface
//!   by design), a hole into the end from its side to the surface, using
//!   the narrowest diameter along the hole.
//! - Exact where the shapes allow it: between boxes, from a shape to a
//!   flat face, and between parallel holes in line, the wall is a
//!   difference of exact lengths. Where it runs on a slant (a hole past the
//!   corner of a box, two holes side by side at an angle) it is a square
//!   root rounded down to the nanometre; between two holes square to each
//!   other it is found by projecting from one to the other until the
//!   distance stops shrinking (worked in `f64`, the geometry boundary) and
//!   rounded to the nanometre.
//! - A cut that lies wholly inside another (a stub tenon's mortise in
//!   the panel groove it sits in) takes no wood of its own there, so the
//!   two are not walls to each other.
//! - Not measured yet (they take no part, either as the cut measured or as
//!   another cut): dovetails, outlines and slots turned at an angle, and
//!   the cuts between the boards of a part made of boards.
//!
//! A drill is one hole, centred on its place, as the solid makes it.
//!
//! Work is in half nanometres (`i128`), so the radius of a hole whose
//! diameter is an odd number of nanometres is still exact.

use stave_lang::placement::PartAxis;
use stave_lang::units::{Angle, Length};
use stave_model::solved::{Cut, CutKind, Depth, End, LocalFace, Part, Shape, Size3, SolvedDesign};

use crate::render;
use crate::solid::{
    Local, Solid, across, centred_low, depth_range, entering_axes, face_extent, hole_centre,
    place_centre, slot_run,
};

/// Where the least wood round a cut is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Toward {
    /// A flat face of the part.
    Face(LocalFace),
    /// The round surface of round stock or a turned part.
    Surface,
    /// Another cut of the part, by its index in [`Part::cuts`].
    Cut(usize),
}

/// The wood left round a cut.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Wall {
    /// The least wood, `0` when the cut reaches the face or the other cut.
    pub least: Length,
    /// How far it runs past the face or into the other cut; `None` when it
    /// stays clear or only touches.
    pub past: Option<Length>,
    /// Where the least wood is.
    pub toward: Toward,
}

/// A depth that doesn't stop inside the part: a length at least as deep
/// as the part is there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TooDeep {
    /// The cut's depth.
    pub depth: Length,
    /// How deep the part is there, along the cut.
    pub there: Length,
    /// The part's axis the depth runs along.
    pub axis: PartAxis,
    /// `true` when the part is round there (a hole into the side of a leg).
    pub round: bool,
}

/// A tenon or round tenon bigger than the part it is cut on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Oversize {
    /// The tenon's size.
    pub size: Length,
    /// The part's size there.
    pub part: Length,
    /// The part's axis compared; `None` for a round tenon's diameter.
    pub axis: Option<PartAxis>,
}

/// A shape a cut removes, in half nanometres in the part's frame
/// (coordinates indexed by [`slot`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Volume {
    /// A box.
    Box { min: [i128; 3], max: [i128; 3] },
    /// A cylinder along a part axis: its centre on the two axes
    /// [`across`] it, its radius, and where it starts and ends.
    Cylinder {
        axis: PartAxis,
        centre: [i128; 2],
        radius: i128,
        start: i128,
        end: i128,
    },
}

/// What a cut removes and how it may open.
#[derive(Debug, Clone, PartialEq, Eq)]
struct CutShape {
    volumes: Vec<Volume>,
    closed: bool,
    entry: LocalFace,
    through: bool,
}

impl CutShape {
    /// `true` when every shape of this cut lies inside a shape of
    /// `other`: the cut takes no wood of its own there (a stub tenon's
    /// mortise inside the panel groove it sits in), so the two aren't
    /// walls to each other.
    fn within(&self, other: &CutShape) -> bool {
        self.volumes.iter().all(|volume| {
            let (min, max) = volume.bounds();
            other.volumes.iter().any(|outer| match outer {
                Volume::Box {
                    min: outer_min,
                    max: outer_max,
                } => PartAxis::ALL.iter().all(|axis| {
                    get(outer_min, *axis) <= get(&min, *axis)
                        && get(&max, *axis) <= get(outer_max, *axis)
                }),
                Volume::Cylinder { .. } => outer == volume,
            })
        })
    }
}

/// The walls round the cuts of one part.
#[derive(Debug, Clone)]
pub struct Walls<'a> {
    part: &'a Part,
    shapes: Vec<CutShape>,
    /// The lathe profile of a round or turned part.
    profile: Option<&'a crate::solid::Profile>,
}

/// A part axis's index in a coordinate array.
fn slot(axis: PartAxis) -> usize {
    match axis {
        PartAxis::Length => 0,
        PartAxis::Width => 1,
        PartAxis::Thickness => 2,
    }
}

/// A length in half nanometres.
fn half(length: Length) -> i128 {
    2 * i128::from(length.nm())
}

/// A length from half nanometres, rounded down (never below zero).
fn whole(half_nm: i128) -> Length {
    let nm = half_nm.max(0).div_euclid(2);
    Length::from_nm(i64::try_from(nm).unwrap_or(i64::MAX))
}

/// A length from half nanometres, rounded up.
fn whole_up(half_nm: i128) -> Length {
    let nm = (half_nm.max(0) + 1).div_euclid(2);
    Length::from_nm(i64::try_from(nm).unwrap_or(i64::MAX))
}

/// The integer square root, rounded down.
fn root(value: i128) -> i128 {
    let root = u128::try_from(value).unwrap_or(0).isqrt();
    i128::try_from(root).unwrap_or(i128::MAX)
}

/// The signed gap between two intervals: positive when apart, zero when
/// they touch, negative (minus the overlap) when they overlap.
fn gap(a: (i128, i128), b: (i128, i128)) -> i128 {
    (b.0 - a.1).max(a.0 - b.1)
}

/// Gaps along independent directions as one signed distance: the
/// straight-line distance over the directions that are apart, or, when
/// none is, the least overlap.
fn combine(gaps: &[i128]) -> i128 {
    let apart: i128 = gaps.iter().filter(|g| **g > 0).map(|g| g * g).sum();
    if apart > 0 {
        root(apart)
    } else {
        gaps.iter().copied().max().unwrap_or(0)
    }
}

/// The signed distance from a disc to a rectangle in a plane.
fn disc_to_rectangle(centre: [i128; 2], radius: i128, low: [i128; 2], high: [i128; 2]) -> i128 {
    let dx = (low[0] - centre[0]).max(centre[0] - high[0]);
    let dy = (low[1] - centre[1]).max(centre[1] - high[1]);
    if dx > 0 && dy > 0 {
        root(dx * dx + dy * dy) - radius
    } else {
        dx.max(dy) - radius
    }
}

impl Volume {
    /// Its bounds on every axis.
    fn bounds(&self) -> ([i128; 3], [i128; 3]) {
        match *self {
            Volume::Box { min, max } => (min, max),
            Volume::Cylinder {
                axis,
                centre,
                radius,
                start,
                end,
            } => {
                let mut min = [0; 3];
                let mut max = [0; 3];
                for (index, other) in across(axis).into_iter().enumerate() {
                    let c = centre.get(index).copied().unwrap_or(0);
                    set(&mut min, other, c - radius);
                    set(&mut max, other, c + radius);
                }
                set(&mut min, axis, start);
                set(&mut max, axis, end);
                (min, max)
            }
        }
    }

    /// The signed distance to another shape.
    fn distance(&self, other: &Volume) -> i128 {
        match (*self, *other) {
            (Volume::Box { .. }, Volume::Box { .. }) => {
                let (a0, a1) = self.bounds();
                let (b0, b1) = other.bounds();
                combine(&PartAxis::ALL.map(|axis| {
                    gap(
                        (get(&a0, axis), get(&a1, axis)),
                        (get(&b0, axis), get(&b1, axis)),
                    )
                }))
            }
            (Volume::Box { min, max }, cylinder @ Volume::Cylinder { .. })
            | (cylinder @ Volume::Cylinder { .. }, Volume::Box { min, max }) => {
                box_to_cylinder(min, max, &cylinder)
            }
            (
                Volume::Cylinder {
                    axis: a_axis,
                    centre: a_centre,
                    radius: a_radius,
                    start: a_start,
                    end: a_end,
                },
                Volume::Cylinder {
                    axis: b_axis,
                    centre: b_centre,
                    radius: b_radius,
                    start: b_start,
                    end: b_end,
                },
            ) if a_axis == b_axis => {
                let along = gap((a_start, a_end), (b_start, b_end));
                let dx = a_centre[0] - b_centre[0];
                let dy = a_centre[1] - b_centre[1];
                let apart = root(dx * dx + dy * dy) - a_radius - b_radius;
                combine(&[along, apart])
            }
            (Volume::Cylinder { .. }, Volume::Cylinder { .. }) => square_cylinders(self, other),
        }
    }
}

fn get(point: &[i128; 3], axis: PartAxis) -> i128 {
    point.get(slot(axis)).copied().unwrap_or(0)
}

fn set(point: &mut [i128; 3], axis: PartAxis, value: i128) {
    if let Some(entry) = point.get_mut(slot(axis)) {
        *entry = value;
    }
}

/// The signed distance from a box to a cylinder: along the cylinder's
/// axis and across it are independent, so the distance is the two
/// combined.
fn box_to_cylinder(min: [i128; 3], max: [i128; 3], cylinder: &Volume) -> i128 {
    let Volume::Cylinder {
        axis,
        centre,
        radius,
        start,
        end,
    } = *cylinder
    else {
        return 0;
    };
    let along = gap((start, end), (get(&min, axis), get(&max, axis)));
    let [u, v] = across(axis);
    let apart = disc_to_rectangle(
        centre,
        radius,
        [get(&min, u), get(&min, v)],
        [get(&max, u), get(&max, v)],
    );
    combine(&[along, apart])
}

/// The distance between two cylinders square to each other, by
/// alternating projections (each projection moves a point onto the
/// nearest point of the other shape; for two convex shapes apart the pair
/// closes on the nearest points). When they meet, the boxes round them
/// give how far.
fn square_cylinders(a: &Volume, b: &Volume) -> i128 {
    let as_f64 = |value: i128| {
        i64::try_from(value)
            .map(|v| render::nm(Length::from_nm(v)))
            .unwrap_or(f64::MAX)
    };
    let project = |shape: &Volume, point: [f64; 3]| -> [f64; 3] {
        let Volume::Cylinder {
            axis,
            centre,
            radius,
            start,
            end,
        } = *shape
        else {
            return point;
        };
        let mut out = point;
        let along = slot(axis);
        if let Some(value) = out.get_mut(along) {
            *value = value.clamp(as_f64(start), as_f64(end));
        }
        let [u, v] = across(axis).map(slot);
        let (cu, cv) = (as_f64(centre[0]), as_f64(centre[1]));
        let (pu, pv) = (
            point.get(u).copied().unwrap_or(0.0),
            point.get(v).copied().unwrap_or(0.0),
        );
        let (du, dv) = (pu - cu, pv - cv);
        let length = du.hypot(dv);
        let r = as_f64(radius);
        if length > r && length > 0.0 {
            if let Some(value) = out.get_mut(u) {
                *value = cu + du * r / length;
            }
            if let Some(value) = out.get_mut(v) {
                *value = cv + dv * r / length;
            }
        }
        out
    };
    let (a0, a1) = a.bounds();
    let mut p = [0.0; 3];
    for axis in PartAxis::ALL {
        if let Some(value) = p.get_mut(slot(axis)) {
            *value = (as_f64(get(&a0, axis)) + as_f64(get(&a1, axis))) / 2.0;
        }
    }
    let distance = |p: [f64; 3], q: [f64; 3]| {
        let d: f64 = p.iter().zip(&q).map(|(x, y)| (x - y) * (x - y)).sum();
        d.sqrt()
    };
    let mut best = f64::MAX;
    for _ in 0..4000 {
        let q = project(b, p);
        p = project(a, q);
        let d = distance(p, q);
        if best - d < 1e-3 {
            best = best.min(d);
            break;
        }
        best = d;
    }
    if best < 1.0 {
        let (b0, b1) = b.bounds();
        let boxes = combine(&PartAxis::ALL.map(|axis| {
            gap(
                (get(&a0, axis), get(&a1, axis)),
                (get(&b0, axis), get(&b1, axis)),
            )
        }));
        return boxes.min(0);
    }
    render::integer_from_f64(best.floor()).map_or(0, i128::from)
}

/// The shapes one cut removes, in half nanometres.
fn shape_of(part: &Part, cut: &Cut, design: &SolvedDesign, round: bool) -> CutShape {
    let size = part.cut;
    let face = cut.face;
    let mut shape = CutShape {
        volumes: Vec::new(),
        closed: false,
        entry: face,
        through: false,
    };
    let full = |size: Size3| ([0; 3], PartAxis::ALL.map(|axis| half(size.get(axis))));
    let block = |min: Local, max: Local| Volume::Box {
        min: PartAxis::ALL.map(|axis| half(min.get(axis))),
        max: PartAxis::ALL.map(|axis| half(max.get(axis))),
    };
    let ranged = |depth: Depth| depth_range(size, face, depth);
    match &cut.kind {
        CutKind::Drill { diameter, depth } => {
            shape.closed = true;
            shape.through = *depth == Depth::Through;
            let (start, end) = ranged(*depth);
            shape.volumes.push(Volume::Cylinder {
                axis: face.axis,
                centre: hole_centre(part, cut).map(half),
                radius: i128::from(diameter.nm()),
                start: half(start),
                end: half(end),
            });
            return shape;
        }
        CutKind::Spigot { length, .. } => {
            let (start, end) = ranged(Depth::Length(*length));
            let (mut min, mut max) = full(size);
            set(&mut min, PartAxis::Length, half(start));
            set(&mut max, PartAxis::Length, half(end));
            shape.volumes.push(Volume::Box { min, max });
            return shape;
        }
        CutKind::Notch { .. }
        | CutKind::Slot { .. }
        | CutKind::Pocket { .. }
        | CutKind::Tenon { .. }
        | CutKind::Round { .. }
        | CutKind::DovetailSocket { .. }
        | CutKind::DovetailTail { .. }
        | CutKind::Outline { .. } => {}
    }
    if round {
        // Not made on round parts yet (the checker warns).
        return shape;
    }
    match &cut.kind {
        CutKind::Notch { depth } => {
            shape.through = *depth == Depth::Through;
            let mut min = Local::default();
            let mut max = size;
            for axis in across(face.axis) {
                let (low, high) = face_extent(part, &cut.place, axis);
                min = min.with(axis, low);
                max = max.with(axis, high);
            }
            let (low, high) = ranged(*depth);
            shape
                .volumes
                .push(block(min.with(face.axis, low), max.with(face.axis, high)));
        }
        CutKind::Pocket {
            width,
            thickness,
            depth,
        } => {
            shape.closed = true;
            shape.through = *depth == Depth::Through;
            let [first, second] = across(face.axis);
            let (along_width, along_thickness) =
                entering_axes(part, &cut.place, design, face.axis).unwrap_or((first, second));
            let middle = place_centre(part, &cut.place, face.axis);
            let mut min = Local::default();
            let mut max = size;
            for (axis, extent) in [(along_width, *width), (along_thickness, *thickness)] {
                let low = centred_low(middle.get(axis), extent);
                min = min.with(axis, low);
                max = max.with(axis, low.checked_add(extent).unwrap_or(low));
            }
            let (low, high) = ranged(*depth);
            shape
                .volumes
                .push(block(min.with(face.axis, low), max.with(face.axis, high)));
        }
        CutKind::Tenon {
            length,
            width,
            thickness,
        } => {
            let (width_axis, thickness_axis) = tenon_axes(face.axis);
            let (start, end) = ranged(Depth::Length(*length));
            let middle = place_centre(part, &cut.place, face.axis);
            let span = |axis: PartAxis, extent: Length| {
                let full = size.get(axis);
                let low = centred_low(middle.get(axis), extent).max(Length::ZERO);
                (low, low.checked_add(extent).unwrap_or(full).min(full))
            };
            let (w0, w1) = span(width_axis, *width);
            let (t0, t1) = span(thickness_axis, *thickness);
            let zone = |min: Local, max: Local| {
                let (min, max) = (min.with(face.axis, start), max.with(face.axis, end));
                PartAxis::ALL
                    .iter()
                    .all(|axis| min.get(*axis) < max.get(*axis))
                    .then(|| block(min, max))
            };
            let zero = Local::default();
            shape.volumes.extend(
                [
                    zone(zero, size.with(width_axis, w0)),
                    zone(zero.with(width_axis, w1), size),
                    zone(
                        zero.with(width_axis, w0),
                        size.with(width_axis, w1).with(thickness_axis, t0),
                    ),
                    zone(
                        zero.with(width_axis, w0).with(thickness_axis, t1),
                        size.with(width_axis, w1),
                    ),
                ]
                .into_iter()
                .flatten(),
            );
        }
        CutKind::Round { .. } => {
            let (start, end) = face_extent(part, &cut.place, PartAxis::Length);
            let (mut min, mut max) = full(size);
            set(&mut min, PartAxis::Length, half(start));
            set(&mut max, PartAxis::Length, half(end));
            shape.volumes.push(Volume::Box { min, max });
        }
        CutKind::Slot {
            width,
            depth,
            angle,
            from_centre,
        } => {
            shape.through = *depth == Depth::Through;
            let quarter = Angle::from_deg(90).microdeg();
            if angle.microdeg().rem_euclid(quarter) != 0 {
                return shape;
            }
            let turns = angle.microdeg().div_euclid(quarter).rem_euclid(4);
            let Some((run, sign)) = slot_run(part, &cut.place, design, face.axis, turns) else {
                return shape;
            };
            let [first, second] = across(face.axis);
            let wide = if run == first { second } else { first };
            let middle = place_centre(part, &cut.place, face.axis);
            let mut min = Local::default();
            let mut max = size;
            let low = centred_low(middle.get(wide), *width);
            min = min.with(wide, low);
            max = max.with(wide, low.checked_add(*width).unwrap_or(low));
            if *from_centre {
                if sign {
                    min = min.with(run, middle.get(run));
                } else {
                    max = max.with(run, middle.get(run));
                }
            }
            let (low, high) = ranged(*depth);
            shape
                .volumes
                .push(block(min.with(face.axis, low), max.with(face.axis, high)));
        }
        CutKind::DovetailSocket { .. } | CutKind::DovetailTail { .. } | CutKind::Outline { .. } => {
        }
        CutKind::Drill { .. } | CutKind::Spigot { .. } => {}
    }
    shape
}

/// The axes of the part a tenon's width and thickness lie across, for a
/// tenon cut from a face square to `face_axis` (as the solid cuts it).
fn tenon_axes(face_axis: PartAxis) -> (PartAxis, PartAxis) {
    match face_axis {
        PartAxis::Length => (PartAxis::Width, PartAxis::Thickness),
        PartAxis::Width => (PartAxis::Length, PartAxis::Thickness),
        PartAxis::Thickness => (PartAxis::Length, PartAxis::Width),
    }
}

/// The face at the other end of an axis.
fn opposite(face: LocalFace) -> LocalFace {
    LocalFace {
        axis: face.axis,
        end: match face.end {
            End::Start => End::End,
            End::End => End::Start,
        },
    }
}

impl<'a> Walls<'a> {
    /// The cuts of a part, ready to measure. `solid` is the part's solid
    /// (for the lathe profile of a round or turned part).
    pub fn of(part: &'a Part, design: &SolvedDesign, solid: &'a Solid) -> Walls<'a> {
        let round = !matches!(part.shape, Shape::Board);
        let shapes = part
            .cuts
            .iter()
            .map(|cut| shape_of(part, cut, design, round))
            .collect();
        Walls {
            part,
            shapes,
            profile: if round { solid.profile() } else { None },
        }
    }

    /// `true` for a closed cut (`drill`, `pocket`): it opens only onto the
    /// face it is cut from, and the opposite one when it goes `through`.
    pub fn is_closed(&self, index: usize) -> bool {
        self.shapes.get(index).is_some_and(|shape| shape.closed)
    }

    /// `true` when the cut can be measured (see the module's notes).
    pub fn is_measured(&self, index: usize) -> bool {
        self.shapes
            .get(index)
            .is_some_and(|shape| !shape.volumes.is_empty())
    }

    /// The narrowest diameter of the lathe profile over a stretch of the
    /// length, in half nanometres as a radius (the diameter in nm).
    fn radius_over(&self, low: i128, high: i128) -> Option<i128> {
        let profile = self.profile?;
        let to_length = |value: i128| whole(value);
        let mut least = profile
            .diameter_at(to_length(low))
            .min(profile.diameter_at(to_length(high)));
        for station in &profile.stations {
            let at = half(station.at);
            if at > low && at < high {
                least = least.min(station.diameter);
            }
        }
        Some(i128::from(least.nm()))
    }

    /// The signed distance from a shape to the round surface (see the
    /// module's notes), when the part is round.
    fn to_surface(&self, volume: &Volume, entry: LocalFace) -> Option<i128> {
        let Volume::Cylinder {
            axis,
            centre,
            radius,
            start,
            end,
        } = *volume
        else {
            return None;
        };
        let lathe = |axis: PartAxis| half(self.part.cut.get(axis)) / 2;
        let [u, v] = across(axis);
        let off = |index: usize, axis: PartAxis| {
            (centre.get(index).copied().unwrap_or(0) - lathe(axis)).abs()
        };
        match axis {
            PartAxis::Length => {
                let r = self.radius_over(start, end)?;
                let (du, dv) = (off(0, u), off(1, v));
                Some(r - root(du * du + dv * dv) - radius)
            }
            PartAxis::Width | PartAxis::Thickness => {
                // `across` a radial axis puts the length first.
                let length_centre = centre.first().copied().unwrap_or(0);
                let r = self.radius_over(length_centre - radius, length_centre + radius)?;
                let other = v;
                let far = match entry.end {
                    End::Start => end,
                    End::End => start,
                };
                let along = (far - lathe(axis)).abs();
                let aside = off(1, other) + radius;
                Some(r - root(along * along + aside * aside))
            }
        }
    }

    /// The signed distance from a cut's shapes to a flat face.
    fn to_face(shape: &CutShape, face: LocalFace, size: Size3) -> Option<i128> {
        shape
            .volumes
            .iter()
            .map(|volume| {
                let (min, max) = volume.bounds();
                match face.end {
                    End::Start => get(&min, face.axis),
                    End::End => half(size.get(face.axis)) - get(&max, face.axis),
                }
            })
            .min()
    }

    /// The wood left round one cut: to every face it isn't meant to open
    /// onto and to every other cut for which `skip` is `false`. `None` when
    /// the cut isn't measured.
    pub fn wall(&self, index: usize, skip: impl Fn(usize) -> bool) -> Option<Wall> {
        let shape = self.shapes.get(index)?;
        if shape.volumes.is_empty() {
            return None;
        }
        let size = self.part.cut;
        let mut opens = vec![shape.entry];
        if shape.through {
            opens.push(opposite(shape.entry));
        }
        let mut best: Option<(i128, Toward)> = None;
        let mut consider = |signed: i128, toward: Toward| {
            if best.is_none_or(|(least, _)| signed < least) {
                best = Some((signed, toward));
            }
        };
        let round = self.profile.is_some();
        for axis in PartAxis::ALL {
            if round && axis != PartAxis::Length {
                continue;
            }
            for end in [End::Start, End::End] {
                let face = LocalFace { axis, end };
                if opens.contains(&face) {
                    continue;
                }
                let Some(signed) = Self::to_face(shape, face, size) else {
                    continue;
                };
                if !shape.closed && signed <= 0 {
                    continue;
                }
                consider(signed, Toward::Face(face));
            }
        }
        let radial_entry = shape.entry.axis != PartAxis::Length;
        if round && shape.closed && !(radial_entry && shape.through) {
            let signed = shape
                .volumes
                .iter()
                .filter_map(|volume| self.to_surface(volume, shape.entry))
                .min();
            if let Some(signed) = signed {
                consider(signed, Toward::Surface);
            }
        }
        for (other_index, other) in self.shapes.iter().enumerate() {
            if other_index == index
                || other.volumes.is_empty()
                || skip(other_index)
                || shape.within(other)
                || other.within(shape)
            {
                continue;
            }
            let signed = shape
                .volumes
                .iter()
                .flat_map(|a| other.volumes.iter().map(|b| a.distance(b)))
                .min();
            if let Some(signed) = signed {
                consider(signed, Toward::Cut(other_index));
            }
        }
        let (signed, toward) = best?;
        Some(Wall {
            least: whole(signed),
            past: (signed < 0).then(|| whole_up(-signed)),
            toward,
        })
    }

    /// A depth given as a length that doesn't stop inside the part: the
    /// part's size along the cut there (for a hole into the side of a round
    /// part, from the face it is drilled from to the far side of the
    /// round, at its narrowest along the hole).
    pub fn too_deep(&self, index: usize) -> Option<TooDeep> {
        let cut = self.part.cuts.get(index)?;
        let depth = match &cut.kind {
            CutKind::Notch { depth }
            | CutKind::Slot { depth, .. }
            | CutKind::Pocket { depth, .. }
            | CutKind::Drill { depth, .. }
            | CutKind::Outline { depth, .. } => *depth,
            CutKind::Tenon { .. }
            | CutKind::Spigot { .. }
            | CutKind::Round { .. }
            | CutKind::DovetailSocket { .. }
            | CutKind::DovetailTail { .. } => return None,
        };
        let Depth::Length(depth) = depth else {
            return None;
        };
        let axis = cut.face.axis;
        let size = self.part.cut.get(axis);
        let round = self.profile.is_some() && axis != PartAxis::Length;
        let there = if round {
            let volume = self.shapes.get(index)?.volumes.first()?;
            let Volume::Cylinder { centre, radius, .. } = *volume else {
                return None;
            };
            let length_centre = centre.first().copied().unwrap_or(0);
            let r = self.radius_over(length_centre - radius, length_centre + radius)?;
            whole(half(size) / 2 + r)
        } else {
            size
        };
        (depth >= there).then_some(TooDeep {
            depth,
            there,
            axis,
            round,
        })
    }

    /// A tenon or round tenon bigger than the part across its end (or, for
    /// a tongue, along its edge): cuts only remove wood.
    pub fn oversize(&self, index: usize) -> Option<Oversize> {
        let cut = self.part.cuts.get(index)?;
        let size = self.part.cut;
        match &cut.kind {
            CutKind::Tenon {
                width, thickness, ..
            } => {
                let (width_axis, thickness_axis) = tenon_axes(cut.face.axis);
                [(width_axis, *width), (thickness_axis, *thickness)]
                    .into_iter()
                    .find(|(axis, tenon)| *tenon > size.get(*axis))
                    .map(|(axis, tenon)| Oversize {
                        size: tenon,
                        part: size.get(axis),
                        axis: Some(axis),
                    })
            }
            CutKind::Spigot { diameter, .. } => {
                let end = match &self.part.shape {
                    Shape::Board => size.width.min(size.thickness),
                    Shape::Round { diameter } => *diameter,
                    Shape::Turned { stations, .. } => {
                        let station = match cut.face.end {
                            End::Start => stations.first(),
                            End::End => stations.last(),
                        };
                        station.map(|s| s.diameter)?
                    }
                };
                (*diameter > end).then_some(Oversize {
                    size: *diameter,
                    part: end,
                    axis: None,
                })
            }
            CutKind::Notch { .. }
            | CutKind::Slot { .. }
            | CutKind::Pocket { .. }
            | CutKind::Round { .. }
            | CutKind::Drill { .. }
            | CutKind::DovetailSocket { .. }
            | CutKind::DovetailTail { .. }
            | CutKind::Outline { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mm(value: i128) -> i128 {
        2_000_000 * value
    }

    fn cylinder(axis: PartAxis, centre: [i128; 2], diameter: i128, range: (i128, i128)) -> Volume {
        Volume::Cylinder {
            axis,
            centre: centre.map(mm),
            radius: mm(diameter) / 2,
            start: mm(range.0),
            end: mm(range.1),
        }
    }

    #[test]
    fn boxes_apart_along_one_axis_are_exactly_that_far_apart() {
        let a = Volume::Box {
            min: [0, 0, 0].map(mm),
            max: [10, 10, 10].map(mm),
        };
        let b = Volume::Box {
            min: [13, 2, 2].map(mm),
            max: [20, 5, 5].map(mm),
        };
        assert_eq!(a.distance(&b), mm(3));
        let touching = Volume::Box {
            min: [10, 0, 0].map(mm),
            max: [20, 10, 10].map(mm),
        };
        assert_eq!(a.distance(&touching), 0);
        let into = Volume::Box {
            min: [8, 0, 0].map(mm),
            max: [20, 10, 10].map(mm),
        };
        assert_eq!(a.distance(&into), -mm(2));
    }

    #[test]
    fn a_hole_beside_a_box_is_measured_from_its_side() {
        let hole = cylinder(PartAxis::Thickness, [20, 10], 8, (0, 15));
        let dado = Volume::Box {
            min: [0, 15, 0].map(mm),
            max: [100, 34, 9].map(mm),
        };
        // The hole's side is at 14mm across; the dado starts at 15mm.
        assert_eq!(hole.distance(&dado), mm(1));
    }

    #[test]
    fn parallel_holes_in_line_are_exact() {
        let a = cylinder(PartAxis::Thickness, [20, 10], 8, (0, 15));
        let b = cylinder(PartAxis::Thickness, [32, 10], 8, (0, 15));
        assert_eq!(a.distance(&b), mm(4));
    }

    #[test]
    fn holes_square_to_each_other_are_measured_on_the_slant() {
        // Two 22mm holes into a 45mm leg from adjacent faces, 15mm deep,
        // their centres 22mm apart along the leg (edge to edge along it):
        // they are clear of each other by about 3.6mm, on the slant
        // between their bottom corners.
        let a = cylinder(PartAxis::Width, [150, 22], 22, (0, 15));
        let b = cylinder(PartAxis::Thickness, [172, 22], 22, (0, 15));
        let found = a.distance(&b);
        assert!(found > mm(3) && found < mm(4), "{found}");
        // At the same height they run into each other.
        let c = cylinder(PartAxis::Thickness, [150, 22], 22, (0, 15));
        assert!(a.distance(&c) < 0);
    }
}
