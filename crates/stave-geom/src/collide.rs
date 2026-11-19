//! Collision tests between cut solids in world coordinates: where wood
//! passes through wood, and whether two parts touch.
//!
//! Two surfaces **touch** when they are within [`CONTACT_TOLERANCE`]
//! (grammar, "Fit and contact"); wood that goes deeper than that into
//! other wood is an [`Overlap`]. The depth of an overlap is how far one
//! solid would have to move to clear the other: for two boxes, the
//! smallest side of the box they share.
//!
//! Boxes are exact: every part is axis-aligned at rest and after a slide's
//! travel, so a board's blocks are whole-nanometre world boxes and box
//! against box is integer arithmetic. Round shapes (rods, holes, a
//! member's cylinders) and a member's prisms and bars ([`WorldPrism`],
//! tested in their plane) are tested in `f64` nanometres, converted in
//! [`render`](crate::render). A hole clears what lies inside it: a rod
//! inside a hole of the other part (a round tenon in its hole, a rung in
//! its socket, a knob's screw) is cut back to where the hole doesn't hold
//! it, and a block or prism lying wholly inside a hole is cut back the
//! same way (a cup hinge's knuckle and links in the cup's bore).
//!
//! Angled cuts ([`WorldCut`], the slanted pieces of a part's
//! [`Prism`](crate::solid::Prism)s) are taken out of the wood two boxes
//! share before it is measured ([`crate::piece`]): what is left is convex
//! pieces, each as deep as its narrowest width (or its length along the
//! cut), worked out in `f64` nanometres from exact inputs. A tail that
//! fills its socket leaves nothing, and touching is never overlap. Rods are
//! tested against the boxes with the angled cuts left in (more wood than
//! there is), and so is a part's contact with others where no box of it
//! meets a cut.
//!
//! A solid turned by a hinge is no longer axis-aligned. When both solids
//! turn about one world axis (or only move), [`overlaps_placed`] tests
//! their pieces in the plane across it, each part's boxes less the holes
//! whose axis lies in that plane ([`crate::plane`], grammar, "Sweeps see
//! cuts"). Otherwise it tests oriented boxes with the separating axis
//! test: each block, rod and prism (as the box around it) of one solid
//! against those of the other. Angled cuts, and holes in the second case,
//! are left out there, which only ever finds more wood than there is.

use glam::{DMat4, DVec3};
use stave_lang::placement::{Axis, Direction, PartAxis};
use stave_lang::units::{Angle, Length};
use stave_model::mechanism::MemberIndex;
use stave_model::mechanism::TurnLine;
use stave_model::solved::{Body, MemberShape, Part, Point, Pose, PoseValue, WorldBox};

use crate::mechanism::MechanismPath;
use stave_model::mechanism::MotionShape;

pub use crate::piece::WorldCut;
use crate::piece::{self, Piece};
use crate::render;
use crate::solid::{Rod, Solid, across};

/// Surfaces this close touch; anything deeper overlaps (0.05mm).
pub const CONTACT_TOLERANCE: Length = Length::from_nm(50_000);

/// A cylinder along a world axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorldRod {
    /// The axis it runs along.
    pub axis: Axis,
    /// Its centre on the two other axes, in [`Axis::others`] order.
    pub centre: [Length; 2],
    /// Its diameter.
    pub diameter: Length,
    /// Its low end along `axis`.
    pub start: Length,
    /// Its high end along `axis`.
    pub end: Length,
}

impl WorldRod {
    /// The box around it.
    pub fn bounds(&self) -> WorldBox {
        let [first, second] = self.axis.others();
        let half = self.diameter.nm().div_euclid(2);
        let rest = self.diameter.nm().saturating_sub(half);
        let point = |axis_value: Length, a: i64, b: i64| {
            Point::default()
                .with(self.axis, axis_value)
                .with(
                    first,
                    Length::from_nm(self.centre[0].nm().saturating_add(a)),
                )
                .with(
                    second,
                    Length::from_nm(self.centre[1].nm().saturating_add(b)),
                )
        };
        WorldBox::new(point(self.start, -half, -half), point(self.end, rest, rest))
    }

    fn radius(&self) -> f64 {
        render::nm(self.diameter) / 2.0
    }

    /// The centre's coordinate along one of the other axes.
    fn centre_on(&self, axis: Axis) -> Option<f64> {
        let [first, second] = self.axis.others();
        if axis == first {
            Some(render::nm(self.centre[0]))
        } else if axis == second {
            Some(render::nm(self.centre[1]))
        } else {
            None
        }
    }

    fn translated(&self, by: Point) -> WorldRod {
        let [first, second] = self.axis.others();
        let shift = |value: Length, axis: Axis| {
            Length::from_nm(value.nm().saturating_add(by.get(axis).nm()))
        };
        WorldRod {
            axis: self.axis,
            centre: [shift(self.centre[0], first), shift(self.centre[1], second)],
            diameter: self.diameter,
            start: shift(self.start, self.axis),
            end: shift(self.end, self.axis),
        }
    }
}

/// A convex outline on the plane across a world axis, extruded along it:
/// a hardware member's prism or bar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorldPrism {
    /// The axis it is extruded along.
    pub axis: Axis,
    /// Its corners in order around it, each on the two other axes in
    /// [`Axis::others`] order.
    pub corners: Vec<[Length; 2]>,
    /// Its low end along `axis`.
    pub start: Length,
    /// Its high end along `axis`.
    pub end: Length,
}

impl WorldPrism {
    /// A bar between two centres, `thickness` wide, reaching half its
    /// thickness past each, as its four corners (rounded to the
    /// nanometre).
    pub(crate) fn bar(
        axis: Axis,
        from: [Length; 2],
        to: [Length; 2],
        thickness: Length,
        start: Length,
        end: Length,
    ) -> WorldPrism {
        let point = |[a, b]: [Length; 2]| glam::DVec2::new(render::nm(a), render::nm(b));
        let (from, to) = (point(from), point(to));
        let direction = (to - from).normalize_or_zero();
        let half = render::nm(thickness) / 2.0;
        let across = direction.perp() * half;
        let (a, b) = (from - direction * half, to + direction * half);
        let corners = [a - across, b - across, b + across, a + across]
            .iter()
            .filter_map(|corner| {
                Some([
                    render::length_from_nm(corner.x)?,
                    render::length_from_nm(corner.y)?,
                ])
            })
            .collect();
        WorldPrism {
            axis,
            corners,
            start,
            end,
        }
    }

    /// The outline on its plane, counter-clockwise, `f64` nanometres.
    fn outline(&self) -> Vec<glam::DVec2> {
        crate::plane::hull(
            self.corners
                .iter()
                .map(|[a, b]| glam::DVec2::new(render::nm(*a), render::nm(*b)))
                .collect(),
        )
    }

    /// Its corners at both ends, world nanometres.
    pub(crate) fn corners_3d(&self) -> Vec<DVec3> {
        let [first, second] = self.axis.others();
        [self.start, self.end]
            .iter()
            .flat_map(|at| {
                self.corners.iter().map(move |[a, b]| {
                    point_vector(
                        Point::default()
                            .with(self.axis, *at)
                            .with(first, *a)
                            .with(second, *b),
                    )
                })
            })
            .collect()
    }

    /// The box around it.
    pub fn bounds(&self) -> Option<WorldBox> {
        let [first, second] = self.axis.others();
        let range = |index: usize| {
            let values = self.corners.iter().filter_map(|c| c.get(index).copied());
            Some((values.clone().min()?, values.max()?))
        };
        let ((a_low, a_high), (b_low, b_high)) = (range(0)?, range(1)?);
        Some(WorldBox::new(
            Point::default()
                .with(self.axis, self.start)
                .with(first, a_low)
                .with(second, b_low),
            Point::default()
                .with(self.axis, self.end)
                .with(first, a_high)
                .with(second, b_high),
        ))
    }

    fn translated(&self, by: Point) -> WorldPrism {
        let [first, second] = self.axis.others();
        let shift = |value: Length, axis: Axis| {
            Length::from_nm(value.nm().saturating_add(by.get(axis).nm()))
        };
        WorldPrism {
            axis: self.axis,
            corners: self
                .corners
                .iter()
                .map(|[a, b]| [shift(*a, first), shift(*b, second)])
                .collect(),
            start: shift(self.start, self.axis),
            end: shift(self.end, self.axis),
        }
    }
}

/// A solid in world coordinates: boxes, rods and prisms of wood or steel,
/// less the holes drilled in it and the angled cuts.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WorldSolid {
    boxes: Vec<WorldBox>,
    rods: Vec<WorldRod>,
    prisms: Vec<WorldPrism>,
    holes: Vec<WorldRod>,
    cuts: Vec<WorldCut>,
}

impl WorldSolid {
    /// A solid of boxes only.
    pub fn from_boxes(boxes: Vec<WorldBox>) -> WorldSolid {
        WorldSolid {
            boxes,
            ..WorldSolid::default()
        }
    }

    /// A solid of one prism, for tests.
    #[cfg(test)]
    pub(crate) fn with_prism(prism: WorldPrism) -> WorldSolid {
        WorldSolid {
            prisms: vec![prism],
            ..WorldSolid::default()
        }
    }

    /// The same solid with a hole drilled in it, for tests.
    #[cfg(test)]
    pub(crate) fn with_hole(mut self, hole: WorldRod) -> WorldSolid {
        self.holes.push(hole);
        self
    }

    /// A hardware member's shapes, at rest.
    pub fn of_body(body: &Body) -> WorldSolid {
        let mut solid = WorldSolid::default();
        for shape in body.shapes() {
            match shape {
                MemberShape::Box(region) => solid.boxes.push(*region),
                MemberShape::Cylinder {
                    axis,
                    centre,
                    diameter,
                    start,
                    end,
                } => solid.rods.push(WorldRod {
                    axis: *axis,
                    centre: *centre,
                    diameter: *diameter,
                    start: *start,
                    end: *end,
                }),
                MemberShape::Prism {
                    axis,
                    points,
                    start,
                    end,
                } => solid.prisms.push(WorldPrism {
                    axis: *axis,
                    corners: points.clone(),
                    start: *start,
                    end: *end,
                }),
                MemberShape::Bar {
                    axis,
                    from,
                    to,
                    thickness,
                    start,
                    end,
                } => solid
                    .prisms
                    .push(WorldPrism::bar(*axis, *from, *to, *thickness, *start, *end)),
            }
        }
        solid
    }

    /// A part's cut solid, placed.
    pub fn of_part(part: &Part, solid: &Solid) -> WorldSolid {
        let pose = &part.pose;
        let boxes = solid
            .blocks()
            .iter()
            .map(|block| pose.world_box_at(block.min(), size_between(block.min(), block.max())))
            .collect();
        let centre = [
            Length::from_nm(part.cut.width.nm().div_euclid(2)),
            Length::from_nm(part.cut.thickness.nm().div_euclid(2)),
        ];
        let rods = solid
            .all_rods(centre)
            .iter()
            .map(|rod| world_rod(pose, rod))
            .collect();
        let holes = solid
            .holes()
            .iter()
            .map(|rod| world_rod(pose, rod))
            .collect();
        let cuts = solid
            .prisms()
            .iter()
            .flat_map(|prism| {
                let [first, second] = across(prism.face_axis());
                let (axis, a) = world_coordinate(pose, prism.face_axis(), prism.depth().from());
                let (_, b) = world_coordinate(pose, prism.face_axis(), prism.depth().to());
                let [o1, _] = axis.others();
                prism.slanted_pieces().map(move |piece| WorldCut {
                    axis,
                    corners: piece
                        .points()
                        .iter()
                        .map(|p| {
                            let (w1, c1) = world_coordinate(pose, first, p.first);
                            let (_, c2) = world_coordinate(pose, second, p.second);
                            if w1 == o1 { [c1, c2] } else { [c2, c1] }
                        })
                        .collect(),
                    start: a.min(b),
                    end: a.max(b),
                })
            })
            .collect();
        WorldSolid {
            boxes,
            rods,
            prisms: Vec::new(),
            holes,
            cuts,
        }
    }

    /// The box around all of it; `None` when it has no wood.
    pub fn bounds(&self) -> Option<WorldBox> {
        self.boxes
            .iter()
            .copied()
            .chain(self.rods.iter().map(WorldRod::bounds))
            .chain(self.prisms.iter().filter_map(WorldPrism::bounds))
            .reduce(|a, b| union(&a, &b))
    }

    /// This solid with another's wood added to it (a part with the pins it
    /// carries): the other's boxes, rods and prisms join this one's, and
    /// this one's holes and angled cuts stay as they are.
    pub fn joined(mut self, other: &WorldSolid) -> WorldSolid {
        self.boxes.extend(other.boxes.iter().copied());
        self.rods.extend(other.rods.iter().copied());
        self.prisms.extend(other.prisms.iter().cloned());
        self
    }

    /// The same solid moved by an exact offset.
    pub fn translated(&self, by: Point) -> WorldSolid {
        WorldSolid {
            boxes: self.boxes.iter().map(|b| translate_box(b, by)).collect(),
            rods: self.rods.iter().map(|r| r.translated(by)).collect(),
            prisms: self.prisms.iter().map(|p| p.translated(by)).collect(),
            holes: self.holes.iter().map(|r| r.translated(by)).collect(),
            cuts: self.cuts.iter().map(|c| c.translated(by)).collect(),
        }
    }

    /// The boxes of wood.
    pub fn boxes(&self) -> &[WorldBox] {
        &self.boxes
    }

    /// The rods of wood.
    pub fn rods(&self) -> &[WorldRod] {
        &self.rods
    }

    /// The prisms (a member's prisms and bars).
    pub fn prisms(&self) -> &[WorldPrism] {
        &self.prisms
    }

    /// The holes.
    pub fn holes(&self) -> &[WorldRod] {
        &self.holes
    }

    /// The angled cuts' slanted pieces: wood the boxes still hold but the
    /// solid does not.
    pub fn cuts(&self) -> &[WorldCut] {
        &self.cuts
    }

    /// The cuts of this solid whose box shares a volume with `region`, as
    /// pieces.
    pub(crate) fn cuts_meeting(&self, region: &WorldBox) -> Vec<Piece> {
        self.cuts
            .iter()
            .filter(|cut| cut.meets(region))
            .map(WorldCut::piece)
            .collect()
    }
}

fn size_between(
    min: stave_model::solved::Size3,
    max: stave_model::solved::Size3,
) -> stave_model::solved::Size3 {
    let mut size = stave_model::solved::Size3::default();
    for axis in PartAxis::ALL {
        size = size.with(
            axis,
            Length::from_nm(max.get(axis).nm().saturating_sub(min.get(axis).nm())),
        );
    }
    size
}

/// A world coordinate of a local one along a part axis.
fn world_coordinate(pose: &Pose, axis: PartAxis, local: Length) -> (Axis, Length) {
    let direction = pose.orientation.direction(axis);
    let world = direction.axis();
    let origin = pose.origin.get(world).nm();
    let value = if direction.is_positive() {
        origin.saturating_add(local.nm())
    } else {
        origin.saturating_sub(local.nm())
    };
    (world, Length::from_nm(value))
}

fn world_rod(pose: &Pose, rod: &Rod) -> WorldRod {
    let (axis, a) = world_coordinate(pose, rod.axis, rod.start);
    let (_, b) = world_coordinate(pose, rod.axis, rod.end);
    let [first, second] = across(rod.axis);
    let (w1, c1) = world_coordinate(pose, first, rod.centre[0]);
    let (_, c2) = world_coordinate(pose, second, rod.centre[1]);
    let [o1, _] = axis.others();
    let centre = if w1 == o1 { [c1, c2] } else { [c2, c1] };
    WorldRod {
        axis,
        centre,
        diameter: rod.diameter,
        start: a.min(b),
        end: a.max(b),
    }
}

pub(crate) fn translate_box(b: &WorldBox, by: Point) -> WorldBox {
    let shift = |p: Point| {
        let mut out = p;
        for axis in Axis::ALL {
            out = out.with(
                axis,
                Length::from_nm(p.get(axis).nm().saturating_add(by.get(axis).nm())),
            );
        }
        out
    };
    WorldBox::new(shift(b.min()), shift(b.max()))
}

/// The box around two boxes.
pub fn union(a: &WorldBox, b: &WorldBox) -> WorldBox {
    let mut min = a.min();
    let mut max = a.max();
    for axis in Axis::ALL {
        min = min.with(axis, min.get(axis).min(b.min().get(axis)));
        max = max.with(axis, max.get(axis).max(b.max().get(axis)));
    }
    WorldBox::new(min, max)
}

/// The box two boxes share, when it has a volume.
pub fn intersection(a: &WorldBox, b: &WorldBox) -> Option<WorldBox> {
    let mut min = a.min();
    let mut max = a.max();
    for axis in Axis::ALL {
        min = min.with(axis, min.get(axis).max(b.min().get(axis)));
        max = max.with(axis, max.get(axis).min(b.max().get(axis)));
        if min.get(axis) >= max.get(axis) {
            return None;
        }
    }
    Some(WorldBox::new(min, max))
}

/// Do two boxes overlap by more than the contact tolerance along every
/// axis (the broad phase's test)?
pub fn boxes_overlap(a: &WorldBox, b: &WorldBox) -> bool {
    Axis::ALL.into_iter().all(|axis| {
        let low = a.min().get(axis).max(b.min().get(axis));
        let high = a.max().get(axis).min(b.max().get(axis));
        high.nm().saturating_sub(low.nm()) > CONTACT_TOLERANCE.nm()
    })
}

/// Wood of one solid inside wood of another.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Overlap {
    /// How far one would have to move to clear the other.
    pub depth: Length,
    /// The world axis along which that move is shortest.
    pub axis: Axis,
    /// The box around the shared wood.
    pub region: WorldBox,
}

/// The smallest side of a box, and its axis.
fn thinnest(region: &WorldBox) -> (Length, Axis) {
    Axis::ALL
        .into_iter()
        .map(|axis| (region.size(axis), axis))
        .min_by_key(|(size, axis)| (*size, *axis))
        .unwrap_or((Length::ZERO, Axis::Up))
}

/// A piece of a rod left to test, with how deep it can at most go when a
/// hole only partly holds it.
#[derive(Debug, Clone, Copy)]
struct RodPiece {
    rod: WorldRod,
    cap: Option<f64>,
}

/// A rod cut back to where `holes` don't hold it.
fn clip(rod: &WorldRod, holes: &[WorldRod]) -> Vec<RodPiece> {
    let mut pieces = vec![RodPiece {
        rod: *rod,
        cap: None,
    }];
    let tolerance = render::nm(CONTACT_TOLERANCE);
    for hole in holes.iter().filter(|h| h.axis == rod.axis) {
        let (Some(a), Some(b)) = (
            rod.centre_on(rod.axis.others()[0]),
            rod.centre_on(rod.axis.others()[1]),
        ) else {
            continue;
        };
        let (Some(c), Some(d)) = (
            hole.centre_on(rod.axis.others()[0]),
            hole.centre_on(rod.axis.others()[1]),
        ) else {
            continue;
        };
        let offset = ((a - c).powi(2) + (b - d).powi(2)).sqrt();
        let held = offset + rod.radius() <= hole.radius() + tolerance;
        let protrudes = offset + rod.radius() - hole.radius();
        if !held && offset >= rod.radius() + hole.radius() {
            continue;
        }
        let mut next = Vec::new();
        for piece in pieces {
            let (s, e) = (piece.rod.start, piece.rod.end);
            let (hs, he) = (hole.start.max(s), hole.end.min(e));
            if hs >= he {
                next.push(piece);
                continue;
            }
            let part = |from: Length, to: Length, cap: Option<f64>| RodPiece {
                rod: WorldRod {
                    start: from,
                    end: to,
                    ..piece.rod
                },
                cap,
            };
            if s < hs {
                next.push(part(s, hs, piece.cap));
            }
            if !held {
                let cap = Some(piece.cap.map_or(protrudes, |c| c.min(protrudes)));
                next.push(part(hs, he, cap));
            }
            if he < e {
                next.push(part(he, e, piece.cap));
            }
        }
        pieces = next;
    }
    pieces
}

/// A box cut back to where `holes` don't hold it: a hole whose disc holds
/// the box's whole cross-section clears the box over the hole's length.
fn clear(region: WorldBox, holes: &[WorldRod]) -> Vec<WorldBox> {
    let tolerance = render::nm(CONTACT_TOLERANCE);
    let mut pieces = vec![region];
    for hole in holes {
        let [first, second] = hole.axis.others();
        let mut next = Vec::new();
        for piece in pieces {
            let (Some(cx), Some(cy)) = (hole.centre_on(first), hole.centre_on(second)) else {
                next.push(piece);
                continue;
            };
            let reach = (hole.radius() + tolerance).powi(2);
            let corners_inside = [piece.min(), piece.max()].iter().all(|p| {
                [piece.min(), piece.max()].iter().all(|q| {
                    let x = render::nm(p.get(first)) - cx;
                    let y = render::nm(q.get(second)) - cy;
                    x * x + y * y <= reach
                })
            });
            let (s, e) = (piece.min().get(hole.axis), piece.max().get(hole.axis));
            let (hs, he) = (hole.start.max(s), hole.end.min(e));
            if !corners_inside || hs >= he {
                next.push(piece);
                continue;
            }
            if s < hs {
                next.push(WorldBox::new(piece.min(), piece.max().with(hole.axis, hs)));
            }
            if he < e {
                next.push(WorldBox::new(piece.min().with(hole.axis, he), piece.max()));
            }
        }
        pieces = next;
    }
    pieces
}

/// How far a disc goes into a rectangle on the plane across an axis:
/// the depth and the axis along which it is shortest to push it out.
pub(crate) fn disc_into_rectangle(
    centre: [f64; 2],
    radius: f64,
    low: [f64; 2],
    high: [f64; 2],
    axes: [Axis; 2],
) -> (f64, Axis) {
    let inside = (0..2).all(|i| centre[i] >= low[i] && centre[i] <= high[i]);
    if inside {
        // Push out through the nearest side.
        let mut best = (f64::INFINITY, axes[0]);
        for i in 0..2 {
            for distance in [centre[i] - low[i], high[i] - centre[i]] {
                if distance + radius < best.0 {
                    best = (distance + radius, axes[i]);
                }
            }
        }
        best
    } else {
        let nearest = [0, 1].map(|i| centre[i].clamp(low[i], high[i]));
        let dx = centre[0] - nearest[0];
        let dy = centre[1] - nearest[1];
        let distance = (dx * dx + dy * dy).sqrt();
        let axis = if dx.abs() >= dy.abs() {
            axes[0]
        } else {
            axes[1]
        };
        (radius - distance, axis)
    }
}

fn rod_box(piece: &RodPiece, other: &WorldBox) -> Option<Overlap> {
    let rod = &piece.rod;
    let low = rod.start.max(other.min().get(rod.axis));
    let high = rod.end.min(other.max().get(rod.axis));
    if low >= high {
        return None;
    }
    let along = render::nm(high) - render::nm(low);
    let axes = rod.axis.others();
    let centre = axes.map(|axis| rod.centre_on(axis).unwrap_or(0.0));
    let (across, across_axis) = disc_into_rectangle(
        centre,
        rod.radius(),
        axes.map(|axis| render::nm(other.min().get(axis))),
        axes.map(|axis| render::nm(other.max().get(axis))),
        axes,
    );
    let (mut depth, mut axis) = if along <= across {
        (along, rod.axis)
    } else {
        (across, across_axis)
    };
    if let Some(cap) = piece.cap
        && cap < depth
    {
        depth = cap;
        axis = across_axis;
    }
    let region = intersection(&rod.bounds(), other)?;
    finish(depth, axis, region)
}

fn rod_rod(a: &RodPiece, b: &RodPiece) -> Option<Overlap> {
    let (ra, rb) = (&a.rod, &b.rod);
    let region = intersection(&ra.bounds(), &rb.bounds())?;
    let (mut depth, axis) = if ra.axis == rb.axis {
        let along = render::nm(ra.end.min(rb.end)) - render::nm(ra.start.max(rb.start));
        let axes = ra.axis.others();
        let delta = axes.map(|x| ra.centre_on(x).unwrap_or(0.0) - rb.centre_on(x).unwrap_or(0.0));
        let distance = (delta[0] * delta[0] + delta[1] * delta[1]).sqrt();
        let across = ra.radius() + rb.radius() - distance;
        let across_axis = if delta[0].abs() >= delta[1].abs() {
            axes[0]
        } else {
            axes[1]
        };
        if along <= across {
            (along, ra.axis)
        } else {
            (across, across_axis)
        }
    } else {
        let third = ra.axis.third(rb.axis)?;
        let gap = (ra.centre_on(third).unwrap_or(0.0) - rb.centre_on(third).unwrap_or(0.0)).abs();
        let mut best = (ra.radius() + rb.radius() - gap, third);
        // Along each rod, against the other's width.
        for (rod, other) in [(ra, rb), (rb, ra)] {
            let middle = other.centre_on(rod.axis).unwrap_or(0.0);
            let low = render::nm(rod.start).max(middle - other.radius());
            let high = render::nm(rod.end).min(middle + other.radius());
            if high - low < best.0 {
                best = (high - low, rod.axis);
            }
        }
        best
    };
    for cap in [a.cap, b.cap].into_iter().flatten() {
        if cap < depth {
            depth = cap;
        }
    }
    finish(depth, axis, region)
}

fn finish(depth: f64, axis: Axis, region: WorldBox) -> Option<Overlap> {
    let depth = render::length_from_nm(depth)?;
    (depth > CONTACT_TOLERANCE).then_some(Overlap {
        depth,
        axis,
        region,
    })
}

/// Every place where wood of `a` goes deeper than the contact tolerance
/// into wood of `b`, both at rest (or moved exactly). Pieces come in a
/// fixed order: `a`'s boxes, then its rods, each against `b`'s boxes, then
/// its rods.
pub fn overlaps(a: &WorldSolid, b: &WorldSolid) -> Vec<Overlap> {
    let mut found = Vec::new();
    let holes: Vec<WorldRod> = a.holes.iter().chain(&b.holes).copied().collect();
    let b_rods: Vec<RodPiece> = b.rods.iter().flat_map(|r| clip(r, &a.holes)).collect();
    for box_a in &a.boxes {
        for box_b in &b.boxes {
            if let Some(shared) = intersection(box_a, box_b) {
                for piece in clear(shared, &holes) {
                    let mut cuts = a.cuts_meeting(&piece);
                    cuts.extend(b.cuts_meeting(&piece));
                    if cuts.is_empty() {
                        let (depth, axis) = thinnest(&piece);
                        if depth > CONTACT_TOLERANCE {
                            found.push(Overlap {
                                depth,
                                axis,
                                region: piece,
                            });
                        }
                        continue;
                    }
                    for left in piece::residual(&piece, &cuts) {
                        let (depth, axis) = left.depth();
                        if let Some(region) = left.bounds() {
                            found.extend(finish(depth, axis, region));
                        }
                    }
                }
            }
        }
        for piece in &b_rods {
            found.extend(rod_box(piece, box_a));
        }
    }
    for rod in &a.rods {
        for piece in clip(rod, &b.holes) {
            for box_b in &b.boxes {
                found.extend(rod_box(&piece, box_b));
            }
            for other in &b_rods {
                found.extend(rod_rod(&piece, other));
            }
        }
    }
    // Prisms: against boxes less the holes, against the boxes around rods,
    // and against each other.
    for (prisms, other) in [(&a.prisms, b), (&b.prisms, a)] {
        for prism in prisms {
            for region in &other.boxes {
                found.extend(prism_box(prism, region, &holes));
            }
            for rod in &other.rods {
                found.extend(prism_box(prism, &rod.bounds(), &[]));
            }
        }
    }
    for x in &a.prisms {
        for y in &b.prisms {
            found.extend(prism_prism(x, y));
        }
    }
    found
}

/// How far a prism goes into a box, less what `holes` clear of the box
/// they share.
fn prism_box(prism: &WorldPrism, region: &WorldBox, holes: &[WorldRod]) -> Vec<Overlap> {
    let Some(shared) = prism.bounds().and_then(|b| intersection(&b, region)) else {
        return Vec::new();
    };
    let outline = prism.outline();
    let [first, second] = prism.axis.others();
    clear(shared, holes)
        .into_iter()
        .filter_map(|piece| {
            let low = |axis: Axis| render::nm(piece.min().get(axis));
            let high = |axis: Axis| render::nm(piece.max().get(axis));
            let rectangle = [
                glam::DVec2::new(low(first), low(second)),
                glam::DVec2::new(high(first), low(second)),
                glam::DVec2::new(high(first), high(second)),
                glam::DVec2::new(low(first), high(second)),
            ];
            if !crate::plane::intersect(&outline, &rectangle) {
                return None;
            }
            let (across, normal) = crate::plane::overlap_depth(&outline, &rectangle);
            let along = high(prism.axis) - low(prism.axis);
            let (depth, axis) = if along <= across {
                (along, prism.axis)
            } else if normal.x.abs() >= normal.y.abs() {
                (across, first)
            } else {
                (across, second)
            };
            finish(depth, axis, piece)
        })
        .collect()
}

/// How far two prisms overlap: exactly in their plane when they share an
/// axis, else as the boxes around them.
fn prism_prism(x: &WorldPrism, y: &WorldPrism) -> Option<Overlap> {
    let region = intersection(&x.bounds()?, &y.bounds()?)?;
    if x.axis != y.axis {
        let (depth, axis) = thinnest(&region);
        return finish(render::nm(depth), axis, region);
    }
    let (a, b) = (x.outline(), y.outline());
    if !crate::plane::intersect(&a, &b) {
        return None;
    }
    let (across, normal) = crate::plane::overlap_depth(&a, &b);
    let along = render::nm(region.size(x.axis));
    let [first, second] = x.axis.others();
    let (depth, axis) = if along <= across {
        (along, x.axis)
    } else if normal.x.abs() >= normal.y.abs() {
        (across, first)
    } else {
        (across, second)
    };
    finish(depth, axis, region)
}

/// Do `a` and `b` touch (come within the contact tolerance of each other)
/// anywhere? Holes are ignored: a rod in a hole of its own size touches.
pub fn touches(a: &WorldSolid, b: &WorldSolid) -> bool {
    let tolerance = render::nm(CONTACT_TOLERANCE);
    // A prism counts as the box around it.
    let shapes = |solid: &WorldSolid| -> Vec<Shape> {
        solid
            .boxes
            .iter()
            .map(|b| Shape::Box(*b))
            .chain(solid.rods.iter().map(|r| Shape::Rod(*r)))
            .chain(
                solid
                    .prisms
                    .iter()
                    .filter_map(WorldPrism::bounds)
                    .map(Shape::Box),
            )
            .collect()
    };
    let (a_shapes, b_shapes) = (shapes(a), shapes(b));
    a_shapes.iter().any(|x| {
        b_shapes
            .iter()
            .any(|y| distance(x, y) <= tolerance && !cut_apart(a, b, x, y))
    })
}

/// Do `a` and `b` touch over an area: a face against a face, or wood in
/// wood, rather than only along an edge or at a corner (grammar, "Fit and
/// contact")? Two boxes touch over an area when the gap between them is
/// within the contact tolerance and they share more than the tolerance
/// along two axes at least. A rod touches a box with its end on a face
/// (the disc reaching more than the tolerance into the face) or by going
/// into it; lying along a face, it touches only along a line. Two rods
/// on one axis touch end to end or by going into each other; rods on
/// different axes, and prisms, count as the boxes around them. Holes are
/// ignored, as in [`touches`].
pub fn touches_over_area(a: &WorldSolid, b: &WorldSolid) -> bool {
    let shapes = |solid: &WorldSolid| -> Vec<Shape> {
        solid
            .boxes
            .iter()
            .map(|b| Shape::Box(*b))
            .chain(solid.rods.iter().map(|r| Shape::Rod(*r)))
            .chain(
                solid
                    .prisms
                    .iter()
                    .filter_map(WorldPrism::bounds)
                    .map(Shape::Box),
            )
            .collect()
    };
    let (a_shapes, b_shapes) = (shapes(a), shapes(b));
    a_shapes.iter().any(|x| {
        b_shapes
            .iter()
            .any(|y| area_between(x, y) && !cut_apart(a, b, x, y))
    })
}

/// How much two ranges share: negative for the gap between them.
fn shared(a: (f64, f64), b: (f64, f64)) -> f64 {
    a.1.min(b.1) - a.0.max(b.0)
}

/// Do two shapes touch over an area ([`touches_over_area`])?
fn area_between(a: &Shape, b: &Shape) -> bool {
    let tolerance = render::nm(CONTACT_TOLERANCE);
    match (a, b) {
        (Shape::Box(x), Shape::Box(y)) => {
            let along: Vec<f64> = Axis::ALL
                .into_iter()
                .map(|axis| shared(box_range(x, axis), box_range(y, axis)))
                .collect();
            along.iter().all(|s| *s >= -tolerance)
                && along.iter().filter(|s| **s > tolerance).count() >= 2
        }
        (Shape::Rod(rod), Shape::Box(other)) | (Shape::Box(other), Shape::Rod(rod)) => {
            let along = shared(
                (render::nm(rod.start), render::nm(rod.end)),
                box_range(other, rod.axis),
            );
            let axes = rod.axis.others();
            let centre = axes.map(|axis| rod.centre_on(axis).unwrap_or(0.0));
            let (into, _) = disc_into_rectangle(
                centre,
                rod.radius(),
                axes.map(|axis| box_range(other, axis).0),
                axes.map(|axis| box_range(other, axis).1),
                axes,
            );
            along >= -tolerance && into > tolerance
        }
        (Shape::Rod(x), Shape::Rod(y)) if x.axis == y.axis => {
            let along = shared(
                (render::nm(x.start), render::nm(x.end)),
                (render::nm(y.start), render::nm(y.end)),
            );
            let axes = x.axis.others();
            let delta = axes
                .map(|axis| x.centre_on(axis).unwrap_or(0.0) - y.centre_on(axis).unwrap_or(0.0));
            let into = x.radius() + y.radius() - (delta[0].powi(2) + delta[1].powi(2)).sqrt();
            along >= -tolerance && into > tolerance
        }
        (Shape::Rod(x), Shape::Rod(y)) => {
            area_between(&Shape::Box(x.bounds()), &Shape::Box(y.bounds()))
        }
    }
}

/// Do the angled cuts take away all the wood where two boxes come within
/// the contact tolerance of each other?
fn cut_apart(a: &WorldSolid, b: &WorldSolid, x: &Shape, y: &Shape) -> bool {
    let (Shape::Box(x), Shape::Box(y)) = (x, y) else {
        return false;
    };
    let grow = |region: &WorldBox| {
        let by = CONTACT_TOLERANCE.nm();
        let shift = |p: Point, sign: i64| {
            let mut out = p;
            for axis in Axis::ALL {
                out = out.with(
                    axis,
                    Length::from_nm(p.get(axis).nm().saturating_add(sign * by)),
                );
            }
            out
        };
        WorldBox::new(shift(region.min(), -1), shift(region.max(), 1))
    };
    let Some(near) = intersection(&grow(x), &grow(y)) else {
        return false;
    };
    let mut cuts = a.cuts_meeting(&near);
    cuts.extend(b.cuts_meeting(&near));
    if cuts.is_empty() {
        return false;
    }
    // Wood of `x` and of `y` within the tolerance of each other, less the
    // cuts: touching if any of it is left.
    let (Some(near_x), Some(near_y)) = (intersection(&near, x), intersection(&near, y)) else {
        return false;
    };
    let left = |region: &WorldBox| {
        piece::residual(region, &cuts)
            .iter()
            .any(|p| p.depth().0 > 1.0)
    };
    !(left(&near_x) && left(&near_y))
}

#[derive(Debug, Clone, Copy)]
enum Shape {
    Box(WorldBox),
    Rod(WorldRod),
}

/// The gap between two ranges (0 when they overlap).
fn gap(a: (f64, f64), b: (f64, f64)) -> f64 {
    (b.0 - a.1).max(a.0 - b.1).max(0.0)
}

fn box_range(b: &WorldBox, axis: Axis) -> (f64, f64) {
    (render::nm(b.min().get(axis)), render::nm(b.max().get(axis)))
}

/// The distance between two shapes (0 when they overlap).
fn distance(a: &Shape, b: &Shape) -> f64 {
    match (a, b) {
        (Shape::Box(x), Shape::Box(y)) => Axis::ALL
            .into_iter()
            .map(|axis| gap(box_range(x, axis), box_range(y, axis)).powi(2))
            .sum::<f64>()
            .sqrt(),
        (Shape::Rod(rod), Shape::Box(other)) | (Shape::Box(other), Shape::Rod(rod)) => {
            let along = gap(
                (render::nm(rod.start), render::nm(rod.end)),
                box_range(other, rod.axis),
            );
            let axes = rod.axis.others();
            let centre = axes.map(|axis| rod.centre_on(axis).unwrap_or(0.0));
            let nearest = [0, 1].map(|i| {
                let (low, high) = box_range(other, axes[i]);
                centre[i].clamp(low, high)
            });
            let flat = ((centre[0] - nearest[0]).powi(2) + (centre[1] - nearest[1]).powi(2)).sqrt()
                - rod.radius();
            (along.powi(2) + flat.max(0.0).powi(2)).sqrt()
        }
        (Shape::Rod(x), Shape::Rod(y)) if x.axis == y.axis => {
            let along = gap(
                (render::nm(x.start), render::nm(x.end)),
                (render::nm(y.start), render::nm(y.end)),
            );
            let axes = x.axis.others();
            let delta = axes
                .map(|axis| x.centre_on(axis).unwrap_or(0.0) - y.centre_on(axis).unwrap_or(0.0));
            let flat = (delta[0].powi(2) + delta[1].powi(2)).sqrt() - x.radius() - y.radius();
            (along.powi(2) + flat.max(0.0).powi(2)).sqrt()
        }
        (Shape::Rod(x), Shape::Rod(y)) => {
            distance(&Shape::Box(x.bounds()), &Shape::Box(y.bounds()))
        }
    }
}

// ---------------------------------------------------------------------------
// Moved solids
// ---------------------------------------------------------------------------

/// Where a solid is in a scene, relative to rest: moved exactly by slides,
/// or turned by a hinge (then only as exact as `f64`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Placement {
    /// Moved by an exact offset (zero at rest).
    Exact(Point),
    /// Turned (and perhaps moved): world nanometres to world nanometres.
    Turned(DMat4),
}

impl Placement {
    /// At rest.
    pub const REST: Placement = Placement::Exact(Point {
        across: Length::ZERO,
        up: Length::ZERO,
        deep: Length::ZERO,
    });

    /// The placement after a chain of assembly mechanisms, outermost first
    /// (a drawer inside a cabinet that itself moves), each at its pose
    /// value: with `member`, the body is that member of the last
    /// mechanism (a link) instead of moving with its assembly.
    ///
    /// A travel along one rail stays exact; a turn about one fixed line
    /// turns by the pose's angle as given; any other motion is posed by the
    /// solver within its range (past it, where it stops).
    pub fn of_mechanisms(
        chain: &[(&MechanismPath, PoseValue)],
        member: Option<MemberIndex>,
    ) -> Placement {
        let mut offset = Point::default();
        let mut matrix = DMat4::IDENTITY;
        let mut turned = false;
        let last = chain.len().saturating_sub(1);
        for (index, (path, value)) in chain.iter().enumerate() {
            let member = member.filter(|_| index == last);
            match (member, path.shape()) {
                (None, MotionShape::Travel { toward, travel }) => {
                    let amount = match *value {
                        PoseValue::Open => travel,
                        PoseValue::Travel(length) => length,
                        PoseValue::Closed | PoseValue::Angle(_) => Length::ZERO,
                    };
                    let step = direction_offset(toward, amount);
                    for axis in Axis::ALL {
                        offset = offset.with(
                            axis,
                            Length::from_nm(
                                offset.get(axis).nm().saturating_add(step.get(axis).nm()),
                            ),
                        );
                    }
                    matrix *= DMat4::from_translation(point_vector(step));
                }
                (
                    None,
                    MotionShape::Turn {
                        line: pivot,
                        opening,
                    },
                ) => {
                    let angle = match *value {
                        PoseValue::Open => opening,
                        PoseValue::Angle(angle) => angle,
                        PoseValue::Closed | PoseValue::Travel(_) => continue,
                    };
                    if angle.microdeg() == 0 {
                        continue;
                    }
                    turned = true;
                    matrix *= turn_about(&pivot, angle);
                }
                (Some(_), _) | (None, MotionShape::General) => {
                    let amount = path.amount(*value);
                    if amount == 0.0 {
                        continue;
                    }
                    let moved = path.matrix(member.unwrap_or_else(|| path.moving()), amount);
                    if let Some(moved) = moved {
                        turned = true;
                        matrix *= moved;
                    }
                }
            }
        }
        if turned {
            Placement::Turned(matrix)
        } else {
            Placement::Exact(offset)
        }
    }

    pub(crate) fn matrix(&self) -> DMat4 {
        match self {
            Placement::Exact(offset) => DMat4::from_translation(point_vector(*offset)),
            Placement::Turned(matrix) => *matrix,
        }
    }
}

/// The world turn of `angle` about a pivot line, `free_side` toward
/// `swing_toward`.
pub(crate) fn turn_about(pivot: &TurnLine, angle: Angle) -> DMat4 {
    let free = direction_vector(pivot.free_side);
    let swing = direction_vector(pivot.swing_toward);
    let axis = free.cross(swing).normalize_or(axis_vector(pivot.axis));
    let through = point_vector(pivot.through);
    DMat4::from_translation(through)
        * DMat4::from_axis_angle(axis, render::radians_f64(angle))
        * DMat4::from_translation(-through)
}

pub(crate) fn direction_offset(direction: Direction, amount: Length) -> Point {
    let value = if direction.is_positive() {
        amount
    } else {
        Length::from_nm(amount.nm().saturating_neg())
    };
    Point::default().with(direction.axis(), value)
}

/// World coordinates as an `f64` vector (across, up, deep), nanometres.
pub(crate) fn point_vector(point: Point) -> DVec3 {
    DVec3::new(
        render::nm(point.across),
        render::nm(point.up),
        render::nm(point.deep),
    )
}

fn axis_index(axis: Axis) -> usize {
    match axis {
        Axis::Across => 0,
        Axis::Up => 1,
        Axis::Deep => 2,
    }
}

pub(crate) fn axis_vector(axis: Axis) -> DVec3 {
    let mut v = DVec3::ZERO;
    v[axis_index(axis)] = 1.0;
    v
}

pub(crate) fn direction_vector(direction: Direction) -> DVec3 {
    let v = axis_vector(direction.axis());
    if direction.is_positive() { v } else { -v }
}

/// An oriented box: centre, three unit axes, half sizes.
#[derive(Debug, Clone, Copy)]
struct Oriented {
    centre: DVec3,
    axes: [DVec3; 3],
    half: [f64; 3],
}

impl Oriented {
    fn of(region: &WorldBox, matrix: &DMat4) -> Oriented {
        let low = point_vector(region.min());
        let high = point_vector(region.max());
        let centre = matrix.transform_point3((low + high) / 2.0);
        let size = high - low;
        Oriented {
            centre,
            axes: [DVec3::X, DVec3::Y, DVec3::Z].map(|a| matrix.transform_vector3(a)),
            half: [size.x / 2.0, size.y / 2.0, size.z / 2.0],
        }
    }

    fn reach(&self, direction: DVec3) -> f64 {
        (0..3)
            .map(|i| self.half[i] * self.axes[i].dot(direction).abs())
            .sum()
    }

    fn bounds(&self) -> (DVec3, DVec3) {
        let reach = DVec3::new(
            self.reach(DVec3::X),
            self.reach(DVec3::Y),
            self.reach(DVec3::Z),
        );
        (self.centre - reach, self.centre + reach)
    }
}

/// The separating axis test: the smallest overlap of the two boxes'
/// shadows over the fifteen candidate axes, with that axis.
fn separation(a: &Oriented, b: &Oriented) -> (f64, DVec3) {
    let mut candidates: Vec<DVec3> = a.axes.iter().chain(&b.axes).copied().collect();
    for x in &a.axes {
        for y in &b.axes {
            let cross = x.cross(*y);
            if cross.length_squared() > 1e-12 {
                candidates.push(cross.normalize());
            }
        }
    }
    let between = b.centre - a.centre;
    let mut best = (f64::INFINITY, DVec3::X);
    for axis in candidates {
        let overlap = a.reach(axis) + b.reach(axis) - between.dot(axis).abs();
        if overlap < best.0 {
            best = (overlap, axis);
        }
    }
    best
}

fn nearest_axis(direction: DVec3) -> Axis {
    let a = direction.abs();
    if a.x >= a.y && a.x >= a.z {
        Axis::Across
    } else if a.y >= a.z {
        Axis::Up
    } else {
        Axis::Deep
    }
}

/// The box around a world box moved by a placement, rounded outward to the
/// nanometre.
pub fn placed_bounds(region: &WorldBox, placement: &Placement) -> WorldBox {
    match placement {
        Placement::Exact(offset) => translate_box(region, *offset),
        Placement::Turned(matrix) => {
            let (low, high) = Oriented::of(region, matrix).bounds();
            let corner = |v: DVec3, round: fn(f64) -> f64| {
                Some(Point {
                    across: render::length_from_nm(round(v.x))?,
                    up: render::length_from_nm(round(v.y))?,
                    deep: render::length_from_nm(round(v.z))?,
                })
            };
            match (corner(low, f64::floor), corner(high, f64::ceil)) {
                (Some(a), Some(b)) => WorldBox::new(a, b),
                // Never for a design's coordinates; the unmoved box is the
                // best that can be said.
                _ => *region,
            }
        }
    }
}

/// The world axis both placements turn about (either may only move), when
/// there is one: the motions then lie in the plane across it.
fn common_turn_axis(a: &DMat4, b: &DMat4) -> Option<Axis> {
    let keeps = |matrix: &DMat4, axis: Axis| {
        let unit = axis_vector(axis);
        (matrix.transform_vector3(unit) - unit).length() < 1e-9
    };
    Axis::ALL
        .into_iter()
        .find(|axis| keeps(a, *axis) && keeps(b, *axis))
}

/// The overlaps of two solids moved in the plane across `axis`: their
/// pieces there, each solid's boxes less its holes (grammar, "Sweeps see
/// cuts").
fn overlaps_in_plane(
    a: &WorldSolid,
    ma: &DMat4,
    b: &WorldSolid,
    mb: &DMat4,
    axis: Axis,
) -> Vec<Overlap> {
    let plane = crate::plane::Plane::across(axis);
    let [first, second] = axis.others();
    let mut found = Vec::new();
    for (p, q) in crate::plane::piece_pairs(&plane, a, ma, b, mb) {
        let Some((depth, normal)) = crate::plane::piece_depth(&p, &q) else {
            continue;
        };
        let range = |piece: &crate::plane::Piece, pick: fn(glam::DVec2) -> f64| {
            piece
                .corners
                .iter()
                .map(|c| pick(*c))
                .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), v| {
                    (lo.min(v), hi.max(v))
                })
        };
        let shared = |pick: fn(glam::DVec2) -> f64| {
            let (x, y) = (range(&p, pick), range(&q, pick));
            (x.0.max(y.0), x.1.min(y.1))
        };
        let (x, y) = (shared(|c| c.x), shared(|c| c.y));
        let corner = |a: f64, b: f64, c: f64| -> Option<Point> {
            Some(
                Point::default()
                    .with(first, render::length_from_nm(a)?)
                    .with(second, render::length_from_nm(b)?)
                    .with(axis, render::length_from_nm(c)?),
            )
        };
        let (Some(low), Some(high)) = (
            corner(x.0, y.0, p.low.max(q.low)),
            corner(x.1, y.1, p.high.min(q.high)),
        ) else {
            continue;
        };
        let along = p.shared(&q);
        let shortest = if depth >= along {
            axis
        } else if normal.x.abs() >= normal.y.abs() {
            first
        } else {
            second
        };
        found.extend(finish(depth, shortest, WorldBox::new(low, high)));
    }
    found
}

/// Every place where wood of `a` goes into wood of `b` when each is moved
/// by its placement. Exact when neither is turned.
pub fn overlaps_placed(
    a: &WorldSolid,
    a_placement: &Placement,
    b: &WorldSolid,
    b_placement: &Placement,
) -> Vec<Overlap> {
    if let (Placement::Exact(da), Placement::Exact(db)) = (a_placement, b_placement) {
        return overlaps(&a.translated(*da), &b.translated(*db));
    }
    let (ma, mb) = (a_placement.matrix(), b_placement.matrix());
    if let Some(axis) = common_turn_axis(&ma, &mb) {
        return overlaps_in_plane(a, &ma, b, &mb, axis);
    }
    let boxes = |solid: &WorldSolid, matrix: &DMat4| -> Vec<Oriented> {
        solid
            .boxes
            .iter()
            .copied()
            .chain(solid.rods.iter().map(WorldRod::bounds))
            .chain(solid.prisms.iter().filter_map(WorldPrism::bounds))
            .map(|region| Oriented::of(&region, matrix))
            .collect()
    };
    let (oa, ob) = (boxes(a, &ma), boxes(b, &mb));
    let mut found = Vec::new();
    for x in &oa {
        for y in &ob {
            let (depth, axis) = separation(x, y);
            let (xl, xh) = x.bounds();
            let (yl, yh) = y.bounds();
            let (low, high) = (xl.max(yl), xh.min(yh));
            let corner = |v: DVec3| {
                Some(Point {
                    across: render::length_from_nm(v.x)?,
                    up: render::length_from_nm(v.y)?,
                    deep: render::length_from_nm(v.z)?,
                })
            };
            if let (Some(a), Some(b)) = (corner(low), corner(high)) {
                found.extend(finish(depth, nearest_axis(axis), WorldBox::new(a, b)));
            }
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mm(value: i64) -> Length {
        Length::from_mm(value)
    }

    fn point(across: i64, up: i64, deep: i64) -> Point {
        Point {
            across: mm(across),
            up: mm(up),
            deep: mm(deep),
        }
    }

    fn block(a: Point, b: Point) -> WorldSolid {
        WorldSolid {
            boxes: vec![WorldBox::new(a, b)],
            ..WorldSolid::default()
        }
    }

    #[test]
    fn boxes_that_touch_do_not_overlap() {
        let a = block(point(0, 0, 0), point(10, 10, 10));
        let b = block(point(10, 0, 0), point(20, 10, 10));
        assert!(overlaps(&a, &b).is_empty());
        assert!(touches(&a, &b));
        let apart = block(point(11, 0, 0), point(20, 10, 10));
        assert!(!touches(&a, &apart));
    }

    #[test]
    fn only_a_face_against_a_face_touches_over_an_area() {
        let a = block(point(0, 0, 0), point(10, 10, 10));
        // Face to face, and wood in wood.
        assert!(touches_over_area(
            &a,
            &block(point(10, 0, 0), point(20, 10, 10))
        ));
        assert!(touches_over_area(
            &a,
            &block(point(5, 5, 5), point(20, 20, 20))
        ));
        // Along an edge, and at a corner.
        let edge = block(point(10, 10, 0), point(20, 20, 10));
        assert!(touches(&a, &edge));
        assert!(!touches_over_area(&a, &edge));
        let corner = block(point(10, 10, 10), point(20, 20, 20));
        assert!(touches(&a, &corner));
        assert!(!touches_over_area(&a, &corner));
        // A rod standing on its end touches over an area; one lying along
        // the face only along a line.
        let floor = block(point(0, -10, 0), point(100, 0, 100));
        let standing = WorldSolid {
            rods: vec![rod(Axis::Up, [50, 50], 20, 0, 40)],
            ..WorldSolid::default()
        };
        assert!(touches_over_area(&standing, &floor));
        let lying = WorldSolid {
            rods: vec![WorldRod {
                axis: Axis::Across,
                centre: [mm(10), mm(50)],
                diameter: mm(20),
                start: mm(0),
                end: mm(100),
            }],
            ..WorldSolid::default()
        };
        assert!(touches(&lying, &floor));
        assert!(!touches_over_area(&lying, &floor));
    }

    #[test]
    fn the_depth_is_the_thinnest_side_of_what_they_share() {
        let a = block(point(0, 0, 0), point(10, 10, 10));
        let b = block(point(7, 2, 0), point(20, 8, 10));
        let found = overlaps(&a, &b);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].depth, mm(3));
        assert_eq!(found[0].axis, Axis::Across);
    }

    #[test]
    fn within_the_tolerance_is_touching() {
        let a = block(point(0, 0, 0), point(10, 10, 10));
        let mut b = block(point(10, 0, 0), point(20, 10, 10));
        b = b.translated(Point::default().with(Axis::Across, Length::from_nm(-50_000)));
        assert!(overlaps(&a, &b).is_empty());
        b = b.translated(Point::default().with(Axis::Across, Length::from_nm(-1)));
        assert_eq!(overlaps(&a, &b).len(), 1);
    }

    fn rod(axis: Axis, centre: [i64; 2], diameter: i64, start: i64, end: i64) -> WorldRod {
        WorldRod {
            axis,
            centre: centre.map(mm),
            diameter: mm(diameter),
            start: mm(start),
            end: mm(end),
        }
    }

    #[test]
    fn a_rod_in_its_hole_fits_and_a_longer_one_does_not() {
        // A seat block with a 22mm hole 20mm deep from its bottom face.
        let seat = WorldSolid {
            boxes: vec![WorldBox::new(point(0, 0, 0), point(100, 30, 100))],
            rods: Vec::new(),
            holes: vec![rod(Axis::Up, [50, 50], 22, 0, 20)],
            cuts: Vec::new(),
            prisms: Vec::new(),
        };
        let tenon = WorldSolid {
            rods: vec![rod(Axis::Up, [50, 50], 22, -50, 20)],
            ..WorldSolid::default()
        };
        assert!(overlaps(&tenon, &seat).is_empty());
        assert!(overlaps(&seat, &tenon).is_empty());
        assert!(touches(&tenon, &seat));
        let long = WorldSolid {
            rods: vec![rod(Axis::Up, [50, 50], 22, -50, 23)],
            ..WorldSolid::default()
        };
        let found = overlaps(&long, &seat);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].depth, mm(3));
        assert_eq!(found[0].axis, Axis::Up);
        let fat = WorldSolid {
            rods: vec![rod(Axis::Up, [50, 50], 24, -50, 20)],
            ..WorldSolid::default()
        };
        let found = overlaps(&fat, &seat);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].depth, mm(1));
    }

    #[test]
    fn a_turned_door_hits_what_is_in_its_way() {
        use stave_model::mechanism::{
            FramePoint, Holding, Mechanism, MechanismDrive, MechanismEntry, MechanismFrame,
            MechanismMember,
        };
        use stave_model::solved::{BodyId, HardwareId};
        // A door 500mm wide hinged at its left edge, swinging out toward
        // the front, and a post 100mm in front of its hinge: a leaf turning
        // on a pin at the door's front left edge.
        let door = block(point(0, 0, 0), point(500, 700, 19));
        let post = block(point(5, 0, -150), point(15, 700, -100));
        let member = |name: &str, holding| MechanismMember {
            hardware: HardwareId::from_usize(0),
            body: BodyId::new(0).unwrap(),
            name: name.to_owned(),
            holding,
        };
        let index = |i| MemberIndex::new(i).unwrap();
        let hinge = Mechanism::new(
            MechanismFrame::new(Axis::Up, point(0, 0, 0), Direction::Right, Direction::Front)
                .unwrap(),
            vec![
                member("leaf", Holding::Free),
                member("fixed", Holding::Held),
            ],
            vec![MechanismEntry::Pivot {
                first: index(0),
                second: index(1),
                at: FramePoint::default(),
            }],
            MechanismDrive::Turn {
                member: index(0),
                from: Angle::ZERO,
                to: Angle::from_deg(110),
            },
            None,
            vec![],
        )
        .unwrap();
        let path = MechanismPath::new(&hinge).unwrap();
        assert!(matches!(path.shape(), MotionShape::Turn { .. }));
        let at = |value| Placement::of_mechanisms(&[(&path, value)], None);
        let shut = at(PoseValue::Closed);
        assert_eq!(shut, Placement::REST);
        assert!(overlaps_placed(&door, &shut, &post, &Placement::REST).is_empty());
        let square = at(PoseValue::Angle(Angle::from_deg(90)));
        assert!(!overlaps_placed(&door, &square, &post, &Placement::REST).is_empty());
        let ajar = at(PoseValue::Angle(Angle::from_deg(10)));
        assert!(overlaps_placed(&door, &ajar, &post, &Placement::REST).is_empty());
        // The leaf's own pose, from the solver, is the same turn.
        let solved = Placement::of_mechanisms(
            &[(&path, PoseValue::Angle(Angle::from_deg(90)))],
            Some(index(0)),
        );
        assert!(!overlaps_placed(&door, &solved, &post, &Placement::REST).is_empty());
    }
}
