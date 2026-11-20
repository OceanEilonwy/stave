//! The solved design: the contract every stage after `stave-solve` reads
//! (geometry, scene, viewer, checks, work plan, output).
//!
//! It holds only resolved, typed data: ids, whole-nanometre [`Length`]s,
//! whole-micro-degree [`Angle`]s, poses and the cuts each part receives.
//! It never holds a name that still needs resolving.
//!
//! # Coordinate convention
//!
//! **World axes** are named as in the language, from the viewer standing at
//! the front of the design:
//!
//! | Axis | Positive direction |
//! |---|---|
//! | `across` | `right` |
//! | `up` | `up` |
//! | `deep` | `back` (away from the viewer) |
//!
//! A [`Point`] is `(across, up, deep)` in nanometres. The **origin** is the
//! left-bottom-front corner of the drawn box of the part placed with
//! `ground`. (The floor is wherever the lowest part ends up; feet below the
//! ground part have negative `up`.)
//!
//! **Part frames.** Every part has its own axes `length`, `width` and
//! `thickness` (in that order, the part's local `x`, `y`, `z`). Local
//! coordinates run from `0` to the part's **cut size** along each axis: the
//! local box is the part as it is cut from stock, extensions added by
//! joints (`adds:`) included. A part's [`Pose`] maps local to world:
//!
//! ```text
//! world = pose.origin + l * dir(length) + w * dir(width) + t * dir(thickness)
//! ```
//!
//! where `dir(axis)` is the unit vector of the world [`Direction`] that
//! [`Orientation`] gives for that axis. So `pose.origin` is the world
//! position of the cut box's local corner `(0, 0, 0)`: the start of the
//! length (station 0 of a turned profile), the start of the width and the
//! start of the thickness.
//!
//! An [`Orientation`] is any assignment of three distinct world axes, each
//! with a sign. A part that isn't mirrored has `thickness` along the
//! positive direction of the remaining axis (so `orient(length: up,
//! width: back)` gives thickness `right`). A mirrored part's frame is the
//! reflection of its source's: the sign on the mirror axis flips, which may
//! make the frame left-handed ([`Orientation::is_reflection`]); a renderer
//! must then reverse triangle winding. Because the frame is reflected with
//! the part, a mirrored part's cuts have exactly its source's local
//! coordinates.
//!
//! The **drawn box** (the sizes the file writes, used by placement) sits
//! inside the cut box at [`Part::drawn_offset`], in local coordinates.
//! [`Part::drawn_box`] and [`Part::cut_box`] give both in world
//! coordinates.
//!
//! **Hardware members.** A piece of hardware is its kind's members
//! ([`Hardware::members`]), each a [`Body`] of world shapes at rest with
//! what carries it ([`BodyCarrier`]): a part, a region, or its assembly's
//! mechanism ([`Assembly::mechanism`]), which places every member it moves.
//! Nothing here knows a kind or a member by name.

use std::fmt;

use serde::Serialize;
use stave_lang::ast::{BaseKind, CutOperation};
use stave_lang::placement::{Axis, Direction, Face, PartAxis};
use stave_lang::units::{Angle, Length, Rational};
use stave_schema::species::Species;
use stave_schema::words::{Grain, Measure, OutputUnit, StockForm};
use std::collections::BTreeMap;

pub use crate::defs::{BodyId, CutName};

pub use crate::ids::{
    AssemblyId, HardwareId, JointId, KindId, PairIndex, PartId, RegionId, RequireIndex, SceneId,
    SeamIndex,
};
pub use crate::mechanism::{Mechanism, MemberIndex};
pub use crate::nonempty::NonEmpty;

// ---------------------------------------------------------------------------
// Geometry
// ---------------------------------------------------------------------------

/// A point in world coordinates, in nanometres.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize)]
pub struct Point {
    /// Left to right.
    pub across: Length,
    /// Floor to ceiling.
    pub up: Length,
    /// Front to back.
    pub deep: Length,
}

impl Point {
    /// The coordinate along an axis.
    pub fn get(&self, axis: Axis) -> Length {
        match axis {
            Axis::Across => self.across,
            Axis::Up => self.up,
            Axis::Deep => self.deep,
        }
    }

    /// The same point with one coordinate replaced.
    pub fn with(mut self, axis: Axis, value: Length) -> Self {
        match axis {
            Axis::Across => self.across = value,
            Axis::Up => self.up = value,
            Axis::Deep => self.deep = value,
        }
        self
    }
}

/// An axis-aligned box in world coordinates. `min <= max` on every axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub struct WorldBox {
    min: Point,
    max: Point,
}

impl WorldBox {
    /// A box from two opposite corners, in any order.
    pub fn new(a: Point, b: Point) -> Self {
        let mut min = Point::default();
        let mut max = Point::default();
        for axis in Axis::ALL {
            let (low, high) = if a.get(axis) <= b.get(axis) {
                (a.get(axis), b.get(axis))
            } else {
                (b.get(axis), a.get(axis))
            };
            min = min.with(axis, low);
            max = max.with(axis, high);
        }
        WorldBox { min, max }
    }

    /// The left-bottom-front corner.
    pub fn min(&self) -> Point {
        self.min
    }

    /// The right-top-back corner.
    pub fn max(&self) -> Point {
        self.max
    }

    /// The extent along an axis.
    pub fn size(&self, axis: Axis) -> Length {
        Length::from_nm(
            self.max
                .get(axis)
                .nm()
                .saturating_sub(self.min.get(axis).nm()),
        )
    }

    /// The position of a face's plane.
    pub fn face(&self, face: Face) -> Length {
        if face.is_positive() {
            self.max.get(face.axis())
        } else {
            self.min.get(face.axis())
        }
    }

    /// The smallest box around both.
    pub fn union(&self, other: &WorldBox) -> WorldBox {
        let mut min = self.min;
        let mut max = self.max;
        for axis in Axis::ALL {
            min = min.with(axis, min.get(axis).min(other.min.get(axis)));
            max = max.with(axis, max.get(axis).max(other.max.get(axis)));
        }
        WorldBox { min, max }
    }
}

/// Validation error of an [`Orientation`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("a part's length, width and thickness must run along three different axes")]
pub struct NotAFrame;

/// The world directions a part's own axes run in. Always three distinct
/// world axes; signs are free (see the module docs).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub struct Orientation {
    length: Direction,
    width: Direction,
    thickness: Direction,
}

impl Orientation {
    /// Checks that the three directions are on three different axes.
    pub fn new(
        length: Direction,
        width: Direction,
        thickness: Direction,
    ) -> Result<Self, NotAFrame> {
        let (a, b, c) = (length.axis(), width.axis(), thickness.axis());
        if a == b || b == c || a == c {
            return Err(NotAFrame);
        }
        Ok(Orientation {
            length,
            width,
            thickness,
        })
    }

    /// The frame `orient(length:, width:)` gives an unmirrored part:
    /// thickness along the positive direction of the remaining axis.
    pub fn from_length_and_width(length: Direction, width: Direction) -> Result<Self, NotAFrame> {
        let third = length.axis().third(width.axis()).ok_or(NotAFrame)?;
        let thickness = Direction::ALL
            .into_iter()
            .find(|d| d.axis() == third && d.is_positive())
            .ok_or(NotAFrame)?;
        Orientation::new(length, width, thickness)
    }

    /// Where the length runs.
    pub fn length(&self) -> Direction {
        self.length
    }

    /// Where the width runs.
    pub fn width(&self) -> Direction {
        self.width
    }

    /// Where the thickness runs.
    pub fn thickness(&self) -> Direction {
        self.thickness
    }

    /// The world direction of one of the part's own axes.
    pub fn direction(&self, axis: PartAxis) -> Direction {
        match axis {
            PartAxis::Length => self.length,
            PartAxis::Width => self.width,
            PartAxis::Thickness => self.thickness,
        }
    }

    /// The part's own axis that lies along a world axis.
    pub fn part_axis(&self, axis: Axis) -> PartAxis {
        if self.length.axis() == axis {
            PartAxis::Length
        } else if self.width.axis() == axis {
            PartAxis::Width
        } else {
            PartAxis::Thickness
        }
    }

    /// The frame reflected through a plane perpendicular to `axis`.
    pub fn mirrored(&self, axis: Axis) -> Orientation {
        let flip = |d: Direction| if d.axis() == axis { d.opposite() } else { d };
        Orientation {
            length: flip(self.length),
            width: flip(self.width),
            thickness: flip(self.thickness),
        }
    }

    /// `true` when the frame is a reflection of the right-handed frame
    /// (right, up, front) rather than a rotation of it. A renderer reverses
    /// triangle winding for these.
    pub fn is_reflection(&self) -> bool {
        // Determinant of the signed permutation, against the right-handed
        // basis x = right, y = up, z = front.
        let vector = |d: Direction| -> [i32; 3] {
            match d {
                Direction::Right => [1, 0, 0],
                Direction::Left => [-1, 0, 0],
                Direction::Up => [0, 1, 0],
                Direction::Down => [0, -1, 0],
                Direction::Front => [0, 0, 1],
                Direction::Back => [0, 0, -1],
            }
        };
        let [a, b, c] = [
            vector(self.length),
            vector(self.width),
            vector(self.thickness),
        ];
        let determinant = a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0])
            + a[2] * (b[0] * c[1] - b[1] * c[0]);
        determinant < 0
    }
}

/// Where a part or envelope is: its local origin and its frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub struct Pose {
    /// World position of the local corner (0, 0, 0).
    pub origin: Point,
    /// The frame.
    pub orientation: Orientation,
}

impl Pose {
    /// The world box covered by a local box from (0,0,0) to `size`.
    pub fn world_box(&self, size: Size3) -> WorldBox {
        self.world_box_at(Size3::default(), size)
    }

    /// The world box covered by the local box `offset .. offset + size`.
    pub fn world_box_at(&self, offset: Size3, size: Size3) -> WorldBox {
        let mut a = self.origin;
        let mut b = self.origin;
        for axis in PartAxis::ALL {
            let direction = self.orientation.direction(axis);
            let world = direction.axis();
            let sign: i64 = if direction.is_positive() { 1 } else { -1 };
            let start = offset.get(axis).nm().saturating_mul(sign);
            let end = offset
                .get(axis)
                .nm()
                .saturating_add(size.get(axis).nm())
                .saturating_mul(sign);
            a = a.with(
                world,
                Length::from_nm(a.get(world).nm().saturating_add(start)),
            );
            b = b.with(
                world,
                Length::from_nm(b.get(world).nm().saturating_add(end)),
            );
        }
        WorldBox::new(a, b)
    }
}

/// A size or offset along a part's three own axes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize)]
pub struct Size3 {
    /// Along the length.
    pub length: Length,
    /// Along the width.
    pub width: Length,
    /// Through the thickness.
    pub thickness: Length,
}

impl Size3 {
    /// The value along one axis.
    pub fn get(&self, axis: PartAxis) -> Length {
        match axis {
            PartAxis::Length => self.length,
            PartAxis::Width => self.width,
            PartAxis::Thickness => self.thickness,
        }
    }

    /// The same with one axis replaced.
    pub fn with(mut self, axis: PartAxis, value: Length) -> Self {
        match axis {
            PartAxis::Length => self.length = value,
            PartAxis::Width => self.width = value,
            PartAxis::Thickness => self.thickness = value,
        }
        self
    }
}

/// One end of a part's own axis: local `0` or local cut size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum End {
    /// Local coordinate 0.
    Start,
    /// Local coordinate equal to the cut size.
    End,
}

/// A face of a part in its own frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
pub struct LocalFace {
    /// The axis the face is perpendicular to.
    pub axis: PartAxis,
    /// Which end of that axis it is at.
    pub end: End,
}

/// A point in a part's own frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize)]
pub struct LocalPoint {
    /// Along the length.
    pub length: Length,
    /// Along the width.
    pub width: Length,
    /// Through the thickness.
    pub thickness: Length,
}

/// A box in a part's own frame; `min <= max` on every axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub struct LocalBox {
    /// The corner nearest the local origin.
    pub min: LocalPoint,
    /// The opposite corner.
    pub max: LocalPoint,
}

// ---------------------------------------------------------------------------
// The design
// ---------------------------------------------------------------------------

/// A design as the solver writes it out, before [`SolvedDesign::new`]
/// checks that every table is indexed by its ids.
#[derive(Debug, Clone, PartialEq)]
pub struct DesignDraft {
    /// Title, units, tools.
    pub project: ProjectInfo,
    /// Every part: declared parts, mirror images, parts of mirrored
    /// assemblies and parts joints make. Indexed by [`PartId`].
    pub parts: Vec<Part>,
    /// Every piece of mounted hardware. Indexed by [`HardwareId`].
    pub hardware: Vec<Hardware>,
    /// Fasteners the joints use (never placed).
    pub fasteners: Vec<FastenerUse>,
    /// Every joint. Indexed by [`JointId`].
    pub joints: Vec<Joint>,
    /// Every region. Indexed by [`RegionId`].
    pub regions: Vec<Region>,
    /// Every assembly. Indexed by [`AssemblyId`].
    pub assemblies: Vec<Assembly>,
    /// The scenes; the first is always the built-in `rest`.
    pub scenes: Vec<Scene>,
    /// Every `require` of the project and its components, at the current
    /// values.
    pub requires: Vec<RequireResult>,
    /// Elements spaced with `pitch` or `gap` that run past their `to`
    /// anchor (the fit check reports them as `overlap`).
    pub overruns: Vec<Overrun>,
    /// The kinds' rules that read `wall(cut)`, for every pair or piece of
    /// hardware: measured by the fit check once every cut is made
    /// (grammar, "Walls").
    pub wall_rules: Vec<WallRule>,
}

/// A fully placed design. Its parts, hardware, joints, regions and
/// assemblies are each indexed by their ids; the tables are private so
/// that holds, and are read through [`SolvedDesign::parts`] and the like.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SolvedDesign {
    /// Title, units, tools.
    pub project: ProjectInfo,
    /// Every part: declared parts, mirror images, parts of mirrored
    /// assemblies and parts joints make. Indexed by [`PartId`].
    parts: Vec<Part>,
    /// Every piece of mounted hardware. Indexed by [`HardwareId`].
    hardware: Vec<Hardware>,
    /// Fasteners the joints use (never placed).
    pub fasteners: Vec<FastenerUse>,
    /// Every joint. Indexed by [`JointId`].
    joints: Vec<Joint>,
    /// Every region. Indexed by [`RegionId`].
    regions: Vec<Region>,
    /// Every assembly. Indexed by [`AssemblyId`].
    assemblies: Vec<Assembly>,
    /// The scenes; the first is always the built-in `rest`.
    pub scenes: Vec<Scene>,
    /// Every `require` of the project and its components, at the current
    /// values.
    pub requires: Vec<RequireResult>,
    /// Elements spaced with `pitch` or `gap` that run past their `to`
    /// anchor (the fit check reports them as `overlap`).
    pub overruns: Vec<Overrun>,
    /// The kinds' rules that read `wall(cut)`, for every pair or piece of
    /// hardware: measured by the fit check once every cut is made
    /// (grammar, "Walls").
    pub wall_rules: Vec<WallRule>,
}

impl DesignDraft {
    /// The part with an id.
    pub fn part(&self, id: PartId) -> Option<&Part> {
        self.parts.get(id.index())
    }
}

/// A table of a [`DesignDraft`] whose entries are not at their ids.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("the solved design's {table} table has an entry at {index} whose id is not {index}")]
pub struct DesignError {
    /// The table: `parts`, `hardware`, `joints`, `regions` or `assemblies`.
    pub table: &'static str,
    /// The first entry out of place.
    pub index: usize,
}

/// `Ok` when every entry of `table` sits at its id's index.
fn indexed<T>(
    table: &'static str,
    entries: &[T],
    index_of: impl Fn(&T) -> usize,
) -> Result<(), DesignError> {
    match entries
        .iter()
        .enumerate()
        .find(|(index, entry)| index_of(entry) != *index)
    {
        Some((index, _)) => Err(DesignError { table, index }),
        None => Ok(()),
    }
}

impl SolvedDesign {
    /// The design a draft describes, once each table is checked to be
    /// indexed by its ids.
    pub fn new(draft: DesignDraft) -> Result<SolvedDesign, DesignError> {
        indexed("parts", &draft.parts, |part| part.id.index())?;
        indexed("hardware", &draft.hardware, |hardware| hardware.id.index())?;
        indexed("joints", &draft.joints, |joint| joint.id.index())?;
        indexed("regions", &draft.regions, |region| region.id.index())?;
        indexed("assemblies", &draft.assemblies, |assembly| {
            assembly.id.index()
        })?;
        let DesignDraft {
            project,
            parts,
            hardware,
            fasteners,
            joints,
            regions,
            assemblies,
            scenes,
            requires,
            overruns,
            wall_rules,
        } = draft;
        Ok(SolvedDesign {
            project,
            parts,
            hardware,
            fasteners,
            joints,
            regions,
            assemblies,
            scenes,
            requires,
            overruns,
            wall_rules,
        })
    }

    /// Back to a draft, to change and check again.
    pub fn into_draft(self) -> DesignDraft {
        let SolvedDesign {
            project,
            parts,
            hardware,
            fasteners,
            joints,
            regions,
            assemblies,
            scenes,
            requires,
            overruns,
            wall_rules,
        } = self;
        DesignDraft {
            project,
            parts,
            hardware,
            fasteners,
            joints,
            regions,
            assemblies,
            scenes,
            requires,
            overruns,
            wall_rules,
        }
    }

    /// Every part, in [`PartId`] order.
    pub fn parts(&self) -> &[Part] {
        &self.parts
    }

    /// Every piece of mounted hardware, in [`HardwareId`] order.
    pub fn hardware(&self) -> &[Hardware] {
        &self.hardware
    }

    /// The piece of hardware with an id.
    pub fn hardware_piece(&self, id: HardwareId) -> Option<&Hardware> {
        self.hardware.get(id.index())
    }

    /// Every joint, in [`JointId`] order.
    pub fn joints(&self) -> &[Joint] {
        &self.joints
    }

    /// The joint with an id.
    pub fn joint(&self, id: JointId) -> Option<&Joint> {
        self.joints.get(id.index())
    }

    /// Every region, in [`RegionId`] order.
    pub fn regions(&self) -> &[Region] {
        &self.regions
    }

    /// The region with an id.
    pub fn region(&self, id: RegionId) -> Option<&Region> {
        self.regions.get(id.index())
    }

    /// Every assembly, in [`AssemblyId`] order.
    pub fn assemblies(&self) -> &[Assembly] {
        &self.assemblies
    }

    /// The assembly with an id.
    pub fn assembly(&self, id: AssemblyId) -> Option<&Assembly> {
        self.assemblies.get(id.index())
    }

    /// The part with an id.
    pub fn part(&self, id: PartId) -> Option<&Part> {
        self.parts.get(id.index())
    }

    /// The part with a display name.
    pub fn part_named(&self, name: &str) -> Option<&Part> {
        self.parts.iter().find(|p| p.name == name)
    }

    /// The parts joints made.
    pub fn made_parts(&self) -> impl Iterator<Item = &Part> {
        self.parts.iter().filter(|p| {
            matches!(
                p.origin,
                PartOrigin::Made { .. } | PartOrigin::Boards { .. }
            )
        })
    }

    /// The region made with `box(across:, up:, deep:)` that is the root,
    /// when the root is a box rather than a part placed with `ground`.
    pub fn root_box(&self) -> Option<&Region> {
        self.regions.iter().find(|region| region.root)
    }

    /// Where the floor is: the bottom face of the root box, or, when the
    /// root is a part placed with `ground`, the lowest point of any part.
    pub fn floor(&self) -> Option<Length> {
        if let Some(root) = self.root_box() {
            return Some(root.bounds.min().up);
        }
        self.parts.iter().map(|p| p.cut_box().min().up).min()
    }

    /// A joint pair's member, laid out on its contact patch.
    pub fn pair_member(&self, member: PairMember) -> Option<&Body> {
        self.joints
            .get(member.joint.index())?
            .pair(member.pair)?
            .members
            .iter()
            .find(|body| body.id.body == member.body)
    }

    /// A joint pair's member in words: `top_dowels's pin`, with the pair's
    /// parts when the joint has more than one pair (`top_dowels's pin
    /// between side_l and top`).
    pub fn pair_member_name(&self, member: PairMember) -> Option<String> {
        let joint = self.joints.get(member.joint.index())?;
        let body = self.pair_member(member)?;
        let between = match joint.pairs.as_slice() {
            [_, _, ..] => {
                let parts: Vec<&str> = joint
                    .pair(member.pair)?
                    .roles
                    .iter()
                    .filter_map(|role| self.part(role.part).map(|p| p.name.as_str()))
                    .collect();
                match parts.split_last() {
                    Some((last, [first, rest @ ..])) => {
                        let mut list = (*first).to_owned();
                        for part in rest {
                            list.push_str(", ");
                            list.push_str(part);
                        }
                        format!(" between {list} and {last}")
                    }
                    Some((only, [])) => format!(" in {only}"),
                    None => String::new(),
                }
            }
            [] | [_] => String::new(),
        };
        Some(format!("{}'s {}{between}", joint.name, body.name))
    }

    /// Every joint pair's members, in joint, pair and member order.
    pub fn pair_members(&self) -> impl Iterator<Item = (PairMember, &Body)> {
        self.joints.iter().flat_map(|joint| {
            joint.indexed_pairs().flat_map(move |(pair, found)| {
                found.members.iter().map(move |body| {
                    (
                        PairMember {
                            joint: joint.id,
                            pair,
                            body: body.id.body,
                        },
                        body,
                    )
                })
            })
        })
    }
}

/// The project item and its settings.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ProjectInfo {
    /// The title.
    pub title: String,
    /// `goal:`
    pub goal: Option<String>,
    /// `units:`
    #[serde(serialize_with = "serialize_word")]
    pub units: OutputUnit,
    /// `resolution:`, always positive.
    pub resolution: Length,
    /// `style:`
    pub style: Option<String>,
    /// `tools:`; `None` means every tool is assumed.
    pub tools: Option<Vec<Tool>>,
    /// `methods:`: recipe names, most preferred first.
    pub methods: Vec<String>,
    /// `kerf:`: what each saw cut takes (grammar, "Cut list").
    pub kerf: Length,
    /// `rough_length:`: extra along the grain for solid parts.
    pub rough_length: Length,
    /// `rough_width:`: extra across the grain for solid parts.
    pub rough_width: Length,
}

/// A tool the project lists.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Tool {
    /// The kind's name, qualified when it isn't from the prelude.
    pub name: String,
    /// `title:`
    pub title: Option<String>,
    /// `powered:`
    pub powered: bool,
}

/// What a part is cut from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Stock {
    /// The stock item's name.
    pub name: String,
    /// The species.
    #[serde(serialize_with = "serialize_species")]
    pub species: Species,
    /// What is bought.
    #[serde(serialize_with = "serialize_word")]
    pub form: StockForm,
    /// Its cross-section, by its form.
    #[serde(flatten)]
    pub section: StockSection,
    /// The widest board available.
    pub max_width: Option<Length>,
    /// The longest board available.
    pub max_length: Option<Length>,
    /// The allowances the cut list makes for parts cut from it, its own
    /// or the project's (grammar, "Cut list").
    pub allowances: Allowances,
}

/// What the cut list allows for a stock beyond its parts' cut sizes
/// (grammar, "Cut list"): the stock's own values, or the project's where
/// it doesn't say.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Allowances {
    /// What each saw cut takes, counted between pieces when boards are
    /// counted. Never negative.
    pub kerf: Length,
    /// Extra along the grain a part is first cut at; zero for a sheet or
    /// a dowel. Never negative.
    pub rough_length: Length,
    /// Extra across the grain a part is first cut at; zero for a sheet or
    /// a dowel. Never negative.
    pub rough_width: Length,
    /// The thickness the boards are bought at, when it is more than the
    /// finished thickness (`sold:`, solid stock only).
    pub sold: Option<Length>,
}

impl Stock {
    /// The finished thickness of a board stock; `None` for a dowel.
    pub fn board_thickness(&self) -> Option<Length> {
        match self.section {
            StockSection::Board { thickness, .. } => Some(thickness),
            StockSection::Dowel { .. } => None,
        }
    }
}

/// A stock's cross-section, by its form: a board (`solid`, `plywood`,
/// `mdf`) has a thickness and perhaps a fixed width, a dowel a diameter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum StockSection {
    /// A board or sheet.
    Board {
        /// Finished thickness.
        thickness: Length,
        /// A fixed width the stock is sold at.
        width: Option<Length>,
    },
    /// Round stock.
    Dowel {
        /// Its diameter.
        diameter: Length,
    },
}

/// Where a part comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum PartOrigin {
    /// Declared with `part name : stock { }`.
    Declared,
    /// `part name = mirror(source, ...)`.
    Mirror {
        /// The source part.
        source: PartId,
    },
    /// A part of a mirrored assembly: the image of `source`.
    MirroredAssembly {
        /// The part it is the image of.
        source: PartId,
    },
    /// A part a joint makes, once per pair.
    Made {
        /// The joint.
        joint: JointId,
        /// The pair.
        pair: PairIndex,
    },
    /// A piece of a spliced part after its first (grammar, "Splices"):
    /// the first piece keeps the part's own id and origin, and its
    /// [`Part::pieces`] lists them all.
    Piece {
        /// The part's first piece, which keeps the part's id.
        part: PartId,
        /// Its number, from the end the part's `lengths:` measures from.
        index: PieceIndex,
    },
    /// A part a board joint makes (a spline): once per seam of a part made
    /// of boards, or once for a kind with one `Parts` role.
    Boards {
        /// The part made of boards.
        part: PartId,
        /// The seam; `None` for a kind across all of the part's boards.
        seam: Option<SeamIndex>,
    },
}

/// A turned profile's station: a diameter at a distance from the start.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Station {
    /// Distance from station 0, along the drawn length.
    pub at: Length,
    /// The diameter there (radii are doubled).
    pub diameter: Length,
}

/// The shape of a part.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum Shape {
    /// A rectangular board (or a blank before its cuts).
    Board,
    /// Round stock (a dowel): a cylinder of the stock's diameter along the
    /// length.
    Round {
        /// The diameter.
        diameter: Length,
    },
    /// Turned on a lathe to a profile.
    Turned {
        /// The profile's name.
        profile: String,
        /// The stations, from station 0, diameters only. Station 0 is at
        /// the start of the drawn length (local length =
        /// `drawn_offset.length`).
        stations: Vec<Station>,
        /// `smooth:`
        smooth: bool,
        /// What the profile's values were written as.
        #[serde(serialize_with = "serialize_word")]
        measure: Measure,
    },
}

/// A part, fully sized and placed.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Part {
    /// Its id.
    pub id: PartId,
    /// Its name as written in the source: `drawer[top].side[left]`,
    /// `splice.pin`.
    pub name: String,
    /// The assembly it belongs to, if any (the innermost one).
    pub assembly: Option<AssemblyId>,
    /// Where it comes from.
    pub origin: PartOrigin,
    /// What it is cut from.
    pub stock: Stock,
    /// The drawn size (as written; what placement uses).
    pub drawn: Size3,
    /// The cut size: drawn plus what joints add.
    pub cut: Size3,
    /// Where the drawn box starts inside the cut box, in local coordinates.
    pub drawn_offset: Size3,
    /// Local origin and frame.
    pub pose: Pose,
    /// Board, round or turned.
    pub shape: Shape,
    /// Which way the grain runs.
    #[serde(serialize_with = "serialize_word")]
    pub grain: Grain,
    /// The reference face: one of the two faces across the thickness.
    pub reference_face: LocalFace,
    /// The reference edge: one of the two faces across the width.
    pub reference_edge: LocalFace,
    /// The cuts the part receives, in the order joints and hardware are
    /// declared. The work plan decides the order of work.
    pub cuts: Vec<Cut>,
    /// The joints it is a member of.
    pub joints: Vec<JointId>,
    /// `note:`
    pub note: Option<String>,
    /// The boards it is made of, when it is wider than its stock's
    /// `max_width` (grammar, "Boards"). Derived, never written.
    pub boards: Option<Boards>,
    /// `loose: true`: meant to lift out, so nothing needs to hold it
    /// (grammar, "Held").
    pub loose: bool,
    /// On the first piece of a part spliced end to end (grammar,
    /// "Splices"): every piece and splice. Derived, never written.
    pub pieces: Option<Pieces>,
    /// `banding:`: the edge banding on its edges (grammar, "Edge
    /// banding"). The drawn and cut sizes include it.
    pub banding: Option<Banding>,
}

/// Edge banding glued on a sheet part's edges (grammar, "Edge banding").
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Banding {
    /// The band's thickness, more than zero.
    pub thickness: Length,
    /// The faces it is glued to, in the part's own frame: never across
    /// its thickness, each once, sorted.
    pub faces: Vec<LocalFace>,
}

impl Banding {
    /// How many of the banded faces are at an end of `axis`: 0, 1 or 2.
    pub fn ends_on(&self, axis: PartAxis) -> usize {
        self.faces.iter().filter(|face| face.axis == axis).count()
    }
}

/// The number of a piece of a spliced part: 1 is the piece at the end its
/// `lengths:` measures from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct PieceIndex(std::num::NonZeroU32);

impl PieceIndex {
    /// The piece numbered `number`; `None` for 0.
    pub fn new(number: u32) -> Option<PieceIndex> {
        std::num::NonZeroU32::new(number).map(PieceIndex)
    }

    /// Its number, from 1.
    pub fn get(self) -> u32 {
        self.0.get()
    }
}

/// The pieces a part longer than its stock is made of, joined end to end
/// (grammar, "Splices"). Each piece is a part of its own in the solved
/// design and each splice a joint, so everything that works on parts and
/// joints works on them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Pieces {
    /// The part's name as written, before it was split (`rail`).
    pub name: String,
    /// The pieces, numbered from 1 at the end `from:` names; the first is
    /// the part that carries this list.
    pub parts: NonEmpty<PartId>,
    /// The splices, from the first piece: splice `n` joins piece `n` to
    /// piece `n + 1`.
    pub splices: Vec<JointId>,
}

/// The number of a board within its part: 1 is the board at the
/// reference edge (at the start of the length when `grain: width`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct BoardIndex(std::num::NonZeroU32);

impl BoardIndex {
    /// The board numbered `number`; `None` for 0.
    pub fn new(number: u32) -> Option<BoardIndex> {
        std::num::NonZeroU32::new(number).map(BoardIndex)
    }

    /// Its number, from 1.
    pub fn get(self) -> u32 {
        self.0.get()
    }
}

/// The boards a part is made of, how they are joined and where the seams
/// are. Lengths along `axis` are in the part's own frame.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Boards {
    /// The part's axis across the grain that the boards divide: `width`,
    /// or `length` when `grain: width`.
    pub axis: PartAxis,
    /// The boards, numbered from 1 at the reference edge.
    pub boards: NonEmpty<Board>,
    /// The seams between neighbouring boards, from the reference edge.
    pub seams: Vec<Seam>,
    /// The joint kinds the seams are joined by, as `boards:` lists them.
    pub methods: NonEmpty<BoardMethod>,
    /// The board joints' requires, per seam (`top.seam[1].fits`) or across
    /// all boards (`top.boards.fits`). A failing one is also an error.
    pub requires: Vec<RequireResult>,
}

impl Boards {
    /// The seam at `index`.
    pub fn seam(&self, index: SeamIndex) -> Option<&Seam> {
        self.seams.get(index.index())
    }
}

/// One board of a part.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Board {
    /// Its number.
    pub index: BoardIndex,
    /// Its cut width across the grain: an equal share of the part's,
    /// plus what its edge joints add (a tongue).
    pub width: Length,
    /// Where its cut width starts along [`Boards::axis`].
    pub offset: Length,
    /// The cuts its edge joints make, measured on the board itself (its
    /// own frame starts at `offset`). A neighbouring board is named by the
    /// part it belongs to.
    pub cuts: Vec<Cut>,
}

/// A seam between two neighbouring boards.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Seam {
    /// The boards on either side: the one nearer the reference edge first.
    pub between: (BoardIndex, BoardIndex),
    /// Where the boards meet along [`Boards::axis`], before joints add.
    pub at: Length,
}

/// One joint kind a part's boards are joined by.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BoardMethod {
    /// The kind.
    pub kind: KindId,
    /// Its name as written.
    pub name: String,
    /// `false` when the kind value says `glue: false`.
    pub glue: bool,
}

impl Part {
    /// The cut box in world coordinates.
    pub fn cut_box(&self) -> WorldBox {
        self.pose.world_box(self.cut)
    }

    /// The drawn box in world coordinates.
    pub fn drawn_box(&self) -> WorldBox {
        self.pose.world_box_at(self.drawn_offset, self.drawn)
    }

    /// A copy of the part with no cuts and no boards, room kept for
    /// `cuts` cuts: a plain blank to make trial cuts on. Its own cuts and
    /// boards are never copied.
    pub fn blank(&self, cuts: usize) -> Part {
        Part {
            id: self.id,
            name: self.name.clone(),
            assembly: self.assembly,
            origin: self.origin,
            stock: self.stock.clone(),
            drawn: self.drawn,
            cut: self.cut,
            drawn_offset: self.drawn_offset,
            pose: self.pose,
            shape: self.shape.clone(),
            grain: self.grain,
            reference_face: self.reference_face,
            reference_edge: self.reference_edge,
            cuts: Vec::with_capacity(cuts),
            joints: self.joints.clone(),
            note: self.note.clone(),
            boards: None,
            loose: self.loose,
            pieces: self.pieces.clone(),
            banding: self.banding.clone(),
        }
    }
}

/// Where a cut comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum CutSource {
    /// A `cut` of a joint kind, for one pair.
    Joint {
        /// The joint.
        joint: JointId,
        /// The pair.
        pair: PairIndex,
    },
    /// A `cut` of a hardware kind, for one piece of hardware.
    Hardware {
        /// The hardware.
        hardware: HardwareId,
    },
    /// A `cut` of a joint kind that joins the boards of a part.
    Boards {
        /// The part made of boards.
        part: PartId,
        /// The seam; `None` for a kind across all of the part's boards.
        seam: Option<SeamIndex>,
    },
    /// A profile of the part's own `edges:` (grammar, "Edge profiles").
    Edges {
        /// The part whose `edges:` it is written in (a mirror image's
        /// source, for a mirror image).
        part: PartId,
    },
}

/// A depth: a length below the face, or all the way through.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Depth {
    /// This far below the face.
    Length(Length),
    /// All the way through the part.
    Through,
}

/// How steeply a dovetail's sides slope away from straight.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Slope {
    /// `angle: 10deg`: the angle from straight.
    Angle(Angle),
    /// `slope: 1 in 8`: across for every one along, exactly (the ratio
    /// `1/8`, always positive).
    Ratio(Rational),
}

impl fmt::Display for Slope {
    /// `angle 10deg` or `slope 1 in 8` (a ratio's numerator and
    /// denominator, in lowest terms).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Slope::Angle(angle) => write!(f, "angle {angle}"),
            Slope::Ratio(ratio) => {
                write!(f, "slope {} in {}", ratio.numerator(), ratio.denominator())
            }
        }
    }
}

/// A point of an `outline`, measured on the face.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Mark {
    /// Along the face, from the end the selector measures from.
    pub along: Length,
    /// Across the face, from the reference edge (or face).
    pub across: Length,
}

/// A cut operation with its measurements.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", tag = "operation")]
pub enum CutKind {
    /// The whole footprint, to a depth.
    Notch {
        /// How deep.
        depth: Depth,
    },
    /// A slot through the footprint's centre.
    Slot {
        /// Its width.
        width: Length,
        /// Its depth.
        depth: Depth,
        /// Its angle in the face.
        angle: Angle,
        /// `true`: from the centre out one side only.
        from_centre: bool,
    },
    /// A rectangular mortise, centred.
    Pocket {
        /// In the entering part's width axis.
        width: Length,
        /// In the entering part's thickness axis.
        thickness: Length,
        /// How deep.
        depth: Depth,
    },
    /// Everything around a centred rectangular tenon.
    Tenon {
        /// Tenon length.
        length: Length,
        /// Tenon width.
        width: Length,
        /// Tenon thickness.
        thickness: Length,
    },
    /// Everything around a centred round tenon.
    Spigot {
        /// Tenon length.
        length: Length,
        /// Tenon diameter.
        diameter: Length,
    },
    /// Turned round where the other part passes.
    Round {
        /// The diameter.
        diameter: Length,
        /// Distance across two flats, when kept.
        flats: Option<Length>,
        /// The flats' angle.
        flats_angle: Angle,
    },
    /// One round hole, centred on the footprint (or on a member's shape).
    Drill {
        /// Hole diameter.
        diameter: Length,
        /// Hole depth (along the slant, for a slanted hole).
        depth: Depth,
        /// How far below the face the hole starts: zero for a hole that
        /// opens onto it.
        from_depth: Length,
        /// The slant of the member it is drilled for, when it has one
        /// (grammar, "Slanted members").
        tilt: Option<Tilt>,
    },
    /// A dovetail socket.
    DovetailSocket {
        /// Neck width.
        neck: Length,
        /// Depth.
        depth: Length,
        /// How steeply its sides slope.
        slope: Slope,
    },
    /// Everything around a dovetail tail.
    DovetailTail {
        /// Neck width.
        neck: Length,
        /// Depth.
        depth: Length,
        /// How steeply its sides slope.
        slope: Slope,
    },
    /// A marked-out shape.
    Outline {
        /// The points.
        marks: Vec<Mark>,
        /// Down to here.
        depth: Depth,
        /// From here (zero: the face).
        from_depth: Length,
    },
    /// Where two parts cross at a corner, the half of their overlap at
    /// this part's end, on the line from the outside corner to the inside
    /// corner (a mitre). Worked out from the crossing's footprint and the
    /// other part.
    Diagonal,
    /// An edge planed off at 45deg, `size` back along each face.
    Chamfer {
        /// How far back along each face.
        size: Length,
    },
    /// An edge rounded to a quarter circle.
    Roundover {
        /// The radius.
        radius: Length,
    },
}

impl CutKind {
    /// The operation's vocabulary word.
    pub fn operation(&self) -> CutOperation {
        match self {
            CutKind::Notch { .. } => CutOperation::Notch,
            CutKind::Slot { .. } => CutOperation::Slot,
            CutKind::Pocket { .. } => CutOperation::Pocket,
            CutKind::Tenon { .. } => CutOperation::Tenon,
            CutKind::Spigot { .. } => CutOperation::Spigot,
            CutKind::Round { .. } => CutOperation::Round,
            CutKind::Drill { .. } => CutOperation::Drill,
            CutKind::DovetailSocket { .. } => CutOperation::DovetailSocket,
            CutKind::DovetailTail { .. } => CutOperation::DovetailTail,
            CutKind::Outline { .. } => CutOperation::Outline,
            CutKind::Diagonal => CutOperation::Diagonal,
            CutKind::Chamfer { .. } => CutOperation::Chamfer,
            CutKind::Roundover { .. } => CutOperation::Roundover,
        }
    }
}

/// Which way a slanted hole leans, in the drilled part's own frame: going
/// deeper, it turns from square to its face by `angle` toward the face
/// `toward` (grammar, "Slanted members").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub struct Tilt {
    /// How far it turns from square to the face: more than 0deg, less
    /// than 90deg.
    pub angle: Angle,
    /// The face of the part it leans toward; never across the axis of the
    /// face it is drilled from.
    pub toward: LocalFace,
}

/// Where on its face a cut is, in the part's own frame.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum CutPlace {
    /// `end(p, facing: q)`: the end of the part nearest `toward`. Width and
    /// thickness are measured across the end, centred unless the operation
    /// says otherwise; depth runs along the length.
    End {
        /// The part it faces.
        toward: PartId,
    },
    /// `side(p, ...)`: `along` is measured from `along_from` on the length
    /// axis, `across` from `across_from` on the face's other axis.
    Side {
        /// The part it is measured from.
        from: PartId,
        /// The end of the length `along` starts at.
        along_from: End,
        /// The axis `across` runs along.
        across_axis: PartAxis,
        /// The end of that axis `across` starts at.
        across_from: End,
    },
    /// `crossing(p, q)`: where `other` meets this part. The footprint is the
    /// region the two cut boxes share, in this part's local coordinates:
    /// the volume a housing cut removes at most, or the contact face (flat
    /// along the face's axis) for parts that only touch.
    Crossing {
        /// The part that touches or enters.
        other: PartId,
        /// The region both cut boxes share, in this part's frame.
        footprint: LocalBox,
    },
    /// `crossing(p, member)`: where one shape of a member of a piece of
    /// hardware, or of one copy of a joint's member, meets this part. The
    /// footprint is the region the shape's box and the part's cut box
    /// share; the shape is the shape's whole box, which a hole is centred
    /// on, so the holes a member makes in two parts line up even where it
    /// runs past the edge of one of them. Both are in this part's local
    /// coordinates.
    Member {
        /// The piece of hardware or the joint pair.
        owner: BodyOwner,
        /// The member, in its kind's list of members with a body.
        body: BodyId,
        /// The region the shape and the part share, in this part's frame.
        footprint: LocalBox,
        /// The shape's whole box, in this part's frame.
        shape: LocalBox,
    },
    /// An edge of the part, where two of its faces meet (grammar, "Edge
    /// profiles"): the profile runs the edge's whole drawn length.
    Edge {
        /// The other face that meets the cut's face at the edge; never on
        /// the same axis.
        other: LocalFace,
    },
}

/// One cut a part receives.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Cut {
    /// Where it comes from.
    pub source: CutSource,
    /// The name its kind gives it (`cut dado: ...`): what messages call it.
    pub name: Option<CutName>,
    /// The face it is cut from.
    pub face: LocalFace,
    /// Where on the face.
    pub place: CutPlace,
    /// What is removed.
    pub kind: CutKind,
}

/// What an anchor plane belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AnchorItem {
    /// A part.
    Part(PartId),
    /// A region.
    Region(RegionId),
}

/// A fixed spacing: `pitch(d)` or `gap(d)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FixedSpacing {
    /// Centre to centre.
    Pitch(Length),
    /// Between neighbours.
    Gap(Length),
}

/// An element spaced with `pitch` or `gap` that runs past its `to`
/// anchor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Overrun {
    /// The element.
    pub part: PartId,
    /// What the `to` anchor belongs to.
    pub to: AnchorItem,
    /// The anchor's face, when it is one.
    #[serde(serialize_with = "serialize_optional_face")]
    pub face: Option<Face>,
    /// The spacing.
    pub spacing: FixedSpacing,
    /// How far the element's far edge lies past the anchor.
    pub past: Length,
    /// The largest pitch or gap at which every element fits; `None` when
    /// none does (one element, or elements wider than the space).
    pub largest: Option<Length>,
}

// ---------------------------------------------------------------------------
// Joints
// ---------------------------------------------------------------------------

/// How the two parts of a pair meet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum Contact {
    /// Their drawn boxes touch over an area, across a plane perpendicular
    /// to `axis`.
    Butt {
        /// The world axis of the touching plane.
        #[serde(serialize_with = "serialize_axis")]
        axis: Axis,
    },
    /// Their drawn boxes overlap.
    Cross,
    /// A connection of three or more parts, or two `Part` roles that don't
    /// meet (reported by the fit check).
    Connection,
}

/// One role of a pair, bound to a part.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RoleBinding {
    /// The role's name.
    pub role: String,
    /// The part.
    pub part: PartId,
}

/// A part lengthened by a joint (`adds: extend(...)`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Extension {
    /// The part.
    pub part: PartId,
    /// The part's own axis it is lengthened along.
    #[serde(serialize_with = "serialize_part_axis")]
    pub axis: PartAxis,
    /// The end (in the part's frame before extension: `Start` is the drawn
    /// box's start).
    pub end: End,
    /// By how much.
    pub by: Length,
}

/// What an assembly step does, with what each kind of step takes: a push
/// may say how far and at what angle, a rotate always says how far it
/// turns, a drive says neither.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StepMotion {
    /// Move along the axis until seated, or exactly `by`.
    Push {
        /// `by:`: how far, when written.
        by: Option<Length>,
        /// `angle:`: the push's tilt off the axis, when written.
        angle: Option<Angle>,
    },
    /// Turn about the axis.
    Rotate {
        /// `by:`: how far it turns.
        by: Angle,
    },
    /// Knock in a part the joint makes, along the axis.
    Drive,
}

/// One step of `insert:`, bound to parts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AssemblyStep {
    /// The part that moves.
    pub part: PartId,
    /// The moving part's own axis, as placed in the finished joint: what a
    /// push or drive moves along, or what a rotate turns about.
    #[serde(serialize_with = "serialize_part_axis")]
    pub axis: PartAxis,
    /// What it does.
    #[serde(flatten)]
    pub motion: StepMotion,
}

impl AssemblyStep {
    /// The step as the source writes it, with `part` naming the moving
    /// part and `axis` the axis it moves along or turns about:
    /// `push(rail, along: width)`.
    pub fn source_text(&self, part: &str, axis: PartAxis) -> String {
        let axis = axis.word();
        match self.motion {
            StepMotion::Push { by, angle } => {
                let by = by.map(|by| format!(", by: {by}")).unwrap_or_default();
                let angle = angle
                    .map(|angle| format!(", angle: {angle}"))
                    .unwrap_or_default();
                format!("push({part}, along: {axis}{by}{angle})")
            }
            StepMotion::Drive => format!("drive({part}, along: {axis})"),
            StepMotion::Rotate { by } => format!("rotate({part}, by: {by}, about: {axis})"),
        }
    }
}

/// A `require` evaluated at the current values.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RequireResult {
    /// Its name; inside an assembly, prefixed with the assembly's name.
    pub name: String,
    /// `true` when it holds.
    pub holds: bool,
    /// Its message, with values filled in.
    pub message: String,
}

/// How a rule compares `wall(cut)` with what it needs, with the wall on
/// the left (`3mm <= wall(hole)` is `wall(hole) >= 3mm`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WallCompare {
    /// `>=`
    AtLeast,
    /// `>`
    MoreThan,
    /// `<=`
    AtMost,
    /// `<`
    LessThan,
    /// `==`
    Exactly,
    /// `!=`
    Not,
}

impl WallCompare {
    /// `true` when a wall of `wall` keeps the rule against `needs`.
    pub fn holds(self, wall: Length, needs: Length) -> bool {
        match self {
            WallCompare::AtLeast => wall >= needs,
            WallCompare::MoreThan => wall > needs,
            WallCompare::AtMost => wall <= needs,
            WallCompare::LessThan => wall < needs,
            WallCompare::Exactly => wall == needs,
            WallCompare::Not => wall != needs,
        }
    }

    /// How a message says what the rule needs: `at least`, `exactly`.
    pub fn phrase(self) -> &'static str {
        match self {
            WallCompare::AtLeast => "at least",
            WallCompare::MoreThan => "more than",
            WallCompare::AtMost => "at most",
            WallCompare::LessThan => "less than",
            WallCompare::Exactly => "exactly",
            WallCompare::Not => "anything but",
        }
    }
}

/// A kind's `require` that reads `wall(cut)`, for one pair of a joint or
/// one piece of hardware: everything but the wall is worked out when the
/// kind's rules run; the wall is measured once every cut is made
/// (grammar, "Walls").
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WallRule {
    /// The pair or piece of hardware whose cut it measures.
    pub source: CutSource,
    /// The kind the rule belongs to.
    #[serde(skip)]
    pub kind: KindId,
    /// The rule, in the kind's list of requires.
    pub require: RequireIndex,
    /// The rule's name.
    pub name: String,
    /// Its message, with values filled in.
    pub message: String,
    /// The cut it measures.
    pub cut: CutName,
    /// How the wall must compare with `needs`.
    pub compare: WallCompare,
    /// The other side of the comparison, rounded once.
    pub needs: Length,
}

/// A fastener a joint uses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FastenerUse {
    /// The joint.
    pub joint: JointId,
    /// The fastener kind's name.
    pub kind: String,
    /// Its diameter.
    pub diameter: Length,
    /// Its length.
    pub length: Length,
    /// How many, over all pairs.
    pub count: u32,
}

/// One application of a joint kind: a pair (or a connection).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct JointPair {
    /// Roles bound to parts, in the kind's role order.
    pub roles: Vec<RoleBinding>,
    /// How the parts meet.
    pub contact: Contact,
    /// What the joint adds to its parts.
    pub extensions: Vec<Extension>,
    /// Parts in the order they are cut (the kind's `cut_first:`, then the
    /// rest, then made parts). Empty when the kind puts no order.
    pub cut_order: Vec<PartId>,
    /// How the parts come together.
    pub insert: Vec<AssemblyStep>,
    /// Parts the joint made for this pair.
    pub made_parts: Vec<PartId>,
    /// The kind's requires for this pair.
    pub requires: Vec<RequireResult>,
    /// The kind's members, laid out on this pair's contact patch: each
    /// with one shape per shape of the member for every copy, drawn and
    /// checked like any body (grammar, "Joint members").
    pub members: Vec<Body>,
}

/// A joint, resolved per pair.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Joint {
    /// Its id.
    pub id: JointId,
    /// Its name as written: `shelf_dados`, `door_l.frame_joints`.
    pub name: String,
    /// The kind's name, qualified when it isn't from the prelude.
    pub kind: String,
    /// The kind's definition in the resolved model (its doc comments and
    /// `insert:` steps, for a viewer).
    #[serde(skip)]
    pub kind_id: KindId,
    /// The assembly it belongs to.
    pub assembly: Option<AssemblyId>,
    /// `glue:` (default true).
    pub glue: bool,
    /// `methods:` recipe names.
    pub methods: Vec<String>,
    /// The pairs.
    pub pairs: Vec<JointPair>,
    /// Listed parts that meet none of their counterparts (the fit check
    /// reports `no-contact` for them).
    pub unmatched: Vec<PartId>,
    /// `note:`, with values filled in.
    pub note: Option<String>,
    /// Where it comes from: a `joint` declaration, or a part's `lengths:`.
    pub origin: JointOrigin,
}

impl Joint {
    /// The pair at `index`.
    pub fn pair(&self, index: PairIndex) -> Option<&JointPair> {
        self.pairs.get(index.index())
    }

    /// Each pair with its index, in order.
    pub fn indexed_pairs(&self) -> impl Iterator<Item = (PairIndex, &JointPair)> {
        self.pairs
            .iter()
            .enumerate()
            .map(|(index, pair)| (PairIndex::from_usize(index), pair))
    }
}

/// Where a joint comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum JointOrigin {
    /// Declared with `joint name : kind(...)`.
    Declared,
    /// A splice of a part's `lengths:` (grammar, "Splices"), joining two of
    /// its pieces.
    Splice {
        /// The part's first piece, which keeps the part's id.
        part: PartId,
        /// Which splice, from 1 at the end `from:` names.
        index: PieceIndex,
    },
}

// ---------------------------------------------------------------------------

/// What kind of mounted hardware.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HardwareBase {
    /// A drawer slide.
    Slide,
    /// A hinge.
    Hinge,
    /// A knob or pull.
    Pull,
    /// A shelf support.
    Support,
}

impl HardwareBase {
    /// The base for a kind's base, if it is mounted hardware.
    pub fn from_base(base: BaseKind) -> Option<HardwareBase> {
        match base {
            BaseKind::Slide => Some(HardwareBase::Slide),
            BaseKind::Hinge => Some(HardwareBase::Hinge),
            BaseKind::Pull => Some(HardwareBase::Pull),
            BaseKind::Support => Some(HardwareBase::Support),
            BaseKind::Joint | BaseKind::Fastener | BaseKind::Tool => None,
        }
    }
}

/// The rest setting of one `adjust` of a piece of hardware (grammar,
/// "Adjustment"): how far a member sits from its nominal place on the
/// member its kind's `adjust` names, along one axis of the kind's frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Setting {
    /// The member that is set.
    pub body: BodyRef,
    /// The axis of the kind's frame.
    pub axis: crate::defs::FrameAxis,
    /// How far from its nominal place, signed along the axis.
    pub offset: Length,
}

/// Whether a piece of hardware keeps its kind's rules where it is placed:
/// every `require` holds and every member sits within its adjustments.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Rules {
    /// Every rule holds.
    Hold,
    /// A `require` or an adjustment failed, and was reported: the
    /// hardware is not where it can be, so the overlap and motion checks
    /// leave it out.
    Broken,
}

/// A model for looks, with what the text says its nodes are (grammar,
/// "Checked against the text").
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Model {
    /// The source as written.
    pub source: String,
    /// `align: node("name")`: the node on the kind's origin; `None` for the
    /// model's own origin.
    pub node: Option<String>,
    /// The file, when it is on this computer: the source, relative to the
    /// folder of the file that declares the kind. `None` for a model in a
    /// standard module or at a remote source, which is not read.
    pub path: Option<std::path::PathBuf>,
    /// `sha256:`: the one file it is pinned to.
    pub sha256: Option<crate::value::Sha256>,
    /// The file the `model(...)` is written in.
    #[serde(skip)]
    pub file: stave_lang::FileId,
    /// Where the `model(...)` is written.
    #[serde(skip)]
    pub span: stave_lang::ast::Span,
    /// Every node the text says the model has, in the kind's order: its
    /// shapes', then its pivots' and slots'.
    pub nodes: Vec<DeclaredNode>,
}

/// A node a hardware kind's text says its model has, and where.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DeclaredNode {
    /// Its name: `arm knuckle`, `knob`, `arm inner_link`.
    pub name: String,
    /// What the text says about it.
    pub value: DeclaredValue,
}

/// What a kind's text says about one node of its model, in the kind's
/// frame with the hardware closed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum DeclaredValue {
    /// A pivot's pin or a slot's end: the node's origin.
    Point(FramePoint),
    /// A member's shapes (one named shape, or its unnamed shapes
    /// together): the bounds of the node's mesh. Never empty.
    Shapes(Vec<FrameShape>),
}

/// A point in a hardware kind's frame (grammar, "The kind's frame").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct FramePoint {
    /// Along the piece.
    pub along: Length,
    /// Across the mount face.
    pub across: Length,
    /// Into the piece from the mount face.
    pub depth: Length,
}

/// A shape of a hardware kind's member in the kind's frame, as declared
/// (before the member's own `mount { }`), as far as its bounds need.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum FrameShape {
    /// A box, a cylinder or a prism: its bounds, exact, each range low end
    /// first.
    Bounds {
        /// Along the piece.
        along: [Length; 2],
        /// Across the mount face.
        across: [Length; 2],
        /// Into the piece.
        depth: [Length; 2],
    },
    /// A bar, whose bounds depend on the angle it lies at.
    Bar {
        /// One end's centre, `[across, depth]`.
        from: [Length; 2],
        /// The other end's centre, `[across, depth]`.
        to: [Length; 2],
        /// How thick it is; it reaches half of it past each centre.
        thickness: Length,
        /// Its extent along the piece, low end first.
        along: [Length; 2],
    },
}

/// A piece of mounted hardware.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Hardware {
    /// Its id.
    pub id: HardwareId,
    /// Its name as written: `drawer[top].slides[left]`.
    pub name: String,
    /// The kind's name.
    pub kind: String,
    /// The kind's definition in the resolved model (its doc comments and
    /// members, for a viewer).
    #[serde(skip)]
    pub kind_id: KindId,
    /// Slide, hinge or pull.
    pub base: HardwareBase,
    /// The assembly it belongs to.
    pub assembly: Option<AssemblyId>,
    /// The part it is mounted on.
    pub piece: PartId,
    /// The world face of the piece it is mounted on.
    #[serde(serialize_with = "serialize_face")]
    pub mount_face: Face,
    /// The world box around every member at rest.
    pub bounds: WorldBox,
    /// A model, when the kind has one.
    pub model: Option<Model>,
    /// The parts and regions it joins, in its kind's role order.
    pub roles: Vec<(RoleName, RoleValue)>,
    /// Its kind's params and fields for this piece of hardware, by name:
    /// the ones whose value is a length, an angle, a count, a truth value
    /// or a text.
    pub entries: BTreeMap<EntryName, EntryValue>,
    /// The fasteners its kind's `uses:` names: counted, never placed.
    pub fasteners: Vec<HardwareFastener>,
    /// Its members with a body, in its kind's order: every shape the
    /// checks see and the viewer draws (grammar, "One collision model").
    pub members: Vec<Body>,
    /// Its kind's mechanism counted on its own, in its own plane, when it
    /// has a drive: what the checks run through its range when no
    /// assembly moves it. Its assembly's motion is
    /// [`Assembly::mechanism`].
    pub own_mechanism: Option<crate::mechanism::Mechanism>,
    /// The settings its kind's `adjust` entries give it at rest, in the
    /// kind's order.
    pub settings: Vec<Setting>,
    /// Whether its kind's rules hold where it is placed.
    pub rules: Rules,
}

/// What a member with a body belongs to: a piece of hardware, or one pair
/// of a joint whose kind lays members out on its contact patch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum BodyOwner {
    /// A piece of hardware.
    Hardware {
        /// Which.
        hardware: HardwareId,
    },
    /// One pair of a joint.
    Joint {
        /// The joint.
        joint: JointId,
        /// The pair.
        pair: PairIndex,
    },
}

impl BodyOwner {
    /// The piece of hardware, for a hardware member.
    pub fn hardware(self) -> Option<HardwareId> {
        match self {
            BodyOwner::Hardware { hardware } => Some(hardware),
            BodyOwner::Joint { .. } => None,
        }
    }
}

/// A member a joint lays out on one pair's contact patch (a dowel's pins,
/// the screws of a screwed joint): which joint, which pair, and which of
/// its kind's members. Every copy of the member on that pair is one
/// [`Body`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
pub struct PairMember {
    /// The joint.
    pub joint: JointId,
    /// The pair.
    pub pair: PairIndex,
    /// The member, among its kind's members with a body.
    pub body: BodyId,
}

impl PairMember {
    /// The body reference it is.
    pub fn body_ref(self) -> BodyRef {
        BodyRef {
            owner: BodyOwner::Joint {
                joint: self.joint,
                pair: self.pair,
            },
            body: self.body,
        }
    }
}

/// A member of one piece of hardware or one joint pair: whose, and which
/// of its kind's members.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
pub struct BodyRef {
    /// The piece of hardware or the joint pair.
    pub owner: BodyOwner,
    /// The member, among its kind's members with a body.
    pub body: BodyId,
}

impl BodyRef {
    /// The piece of hardware, for a hardware member.
    pub fn hardware(self) -> Option<HardwareId> {
        self.owner.hardware()
    }

    /// The joint pair's member, for a joint's member.
    pub fn pair_member(self) -> Option<PairMember> {
        match self.owner {
            BodyOwner::Joint { joint, pair } => Some(PairMember {
                joint,
                pair,
                body: self.body,
            }),
            BodyOwner::Hardware { .. } => None,
        }
    }
}

/// What a joint's member is, when its kind names it (`member pin :
/// dowel_pin(...)`): the fastener the hardware list counts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MemberItem {
    /// The fastener kind's name: `dowel_pin`.
    pub kind: String,
    /// The fastener kind's `species:`, when it is made of wood (a dowel
    /// pin); `None` when it doesn't say, which a viewer draws as steel.
    #[serde(serialize_with = "serialize_optional_species")]
    pub wood: Option<Species>,
}

/// What moves a member.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "id")]
pub enum BodyCarrier {
    /// It moves with this part (`with:` a role bound to a part, or riding
    /// on a member that does).
    Part(PartId),
    /// It moves with the assembly this region belongs to (`with:` a role
    /// bound to a region).
    Region(RegionId),
    /// Its assembly's mechanism places it as the assembly moves: this
    /// member of [`Assembly::mechanism`] (a link, or a member riding on
    /// one).
    Link(MemberIndex),
}

/// One convex shape of a member, at rest, in world coordinates.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum MemberShape {
    /// A box.
    Box(WorldBox),
    /// A cylinder along a world axis.
    Cylinder {
        /// The axis it runs along.
        #[serde(serialize_with = "serialize_axis")]
        axis: Axis,
        /// Its centre on the two other axes, in [`Axis::others`] order.
        centre: [Length; 2],
        /// Its diameter.
        diameter: Length,
        /// Its low end along `axis`.
        start: Length,
        /// Its high end along `axis`.
        end: Length,
    },
    /// A cylinder whose axis is turned from a world axis toward a world
    /// direction square to it (grammar, "Slanted members").
    Slanted {
        /// The middle of its axis.
        centre: Point,
        /// The world axis it is turned from.
        #[serde(serialize_with = "serialize_axis")]
        axis: Axis,
        /// Its axis, read toward `axis`'s positive end, leans toward this
        /// direction, square to `axis`.
        toward: Direction,
        /// How far it is turned: more than 0deg, less than 90deg.
        angle: Angle,
        /// Its diameter.
        diameter: Length,
        /// How long it is along its axis.
        length: Length,
    },
    /// A convex outline on the plane across a world axis, extruded along it.
    Prism {
        /// The axis it is extruded along.
        #[serde(serialize_with = "serialize_axis")]
        axis: Axis,
        /// The outline's corners in order around it, each on the two other
        /// axes in [`Axis::others`] order.
        points: Vec<[Length; 2]>,
        /// Its low end along `axis`.
        start: Length,
        /// Its high end along `axis`.
        end: Length,
    },
    /// A straight bar on the plane across a world axis: `thickness` wide,
    /// reaching half its thickness past each end's centre, extruded along
    /// the axis.
    Bar {
        /// The axis it is extruded along.
        #[serde(serialize_with = "serialize_axis")]
        axis: Axis,
        /// One end's centre, on the two other axes in [`Axis::others`]
        /// order.
        from: [Length; 2],
        /// The other end's centre.
        to: [Length; 2],
        /// How thick it is.
        thickness: Length,
        /// Its low end along `axis`.
        start: Length,
        /// Its high end along `axis`.
        end: Length,
    },
}

/// A member of a piece of hardware with its shapes: a body of the design
/// (grammar, "One collision model").
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Body {
    /// Which member of which piece of hardware.
    pub id: BodyRef,
    /// The member's name in its kind: `cup`, `inner_link`.
    pub name: String,
    /// What moves it.
    pub carrier: BodyCarrier,
    /// What it is, for a joint's member whose kind names a fastener.
    pub item: Option<MemberItem>,
    /// Its shapes, in its kind's order; never empty.
    shapes: Vec<MemberShape>,
    /// How many copies its shapes are (a joint's member laid out `count:`
    /// times); at least 1, and the shapes divide evenly among them.
    copies: u32,
}

impl Body {
    /// A member with its shapes, one copy and no item; `None` when it has
    /// no shapes.
    pub fn new(
        id: BodyRef,
        name: String,
        carrier: BodyCarrier,
        shapes: Vec<MemberShape>,
    ) -> Option<Body> {
        (!shapes.is_empty()).then_some(Body {
            id,
            name,
            carrier,
            item: None,
            shapes,
            copies: 1,
        })
    }

    /// The same member laid out `copies` times, its shapes in copy order
    /// (each copy's shapes together); `None` when the shapes don't divide
    /// evenly among that many copies or there are none.
    pub fn laid_out(self, copies: u32, item: Option<MemberItem>) -> Option<Body> {
        let count = usize::try_from(copies).ok().filter(|count| *count > 0)?;
        self.shapes.len().is_multiple_of(count).then_some(Body {
            copies,
            item,
            ..self
        })
    }

    /// The part it moves with, when a part carries it.
    pub fn carried_by(&self) -> Option<PartId> {
        match self.carrier {
            BodyCarrier::Part(part) => Some(part),
            BodyCarrier::Region(_) | BodyCarrier::Link(_) => None,
        }
    }

    /// How many copies of the member it is: 1 for hardware.
    pub fn copies(&self) -> u32 {
        self.copies
    }

    /// Each copy's shapes, in order.
    pub fn copy_shapes(&self) -> impl Iterator<Item = &[MemberShape]> {
        let per_copy = usize::try_from(self.copies)
            .ok()
            .filter(|count| *count > 0)
            .map_or(self.shapes.len(), |count| self.shapes.len() / count)
            .max(1);
        self.shapes.chunks(per_copy)
    }

    /// Its shapes, in its kind's order (at least one).
    pub fn shapes(&self) -> &[MemberShape] {
        &self.shapes
    }

    /// The world box around all its shapes, at rest.
    pub fn bounds(&self) -> WorldBox {
        let mut shapes = self.shapes.iter().map(MemberShape::bounds);
        let first = shapes.next().unwrap_or_else(|| {
            let zero = Point {
                across: Length::ZERO,
                up: Length::ZERO,
                deep: Length::ZERO,
            };
            WorldBox::new(zero, zero)
        });
        shapes.fold(first, |all, one| all.union(&one))
    }
}

impl MemberShape {
    /// The two ends of a slanted cylinder's axis, each rounded to the
    /// nanometre; `None` for any other shape or on overflow.
    pub fn slanted_ends(&self) -> Option<(Point, Point)> {
        let MemberShape::Slanted {
            centre,
            axis,
            toward,
            angle,
            length,
            ..
        } = self
        else {
            return None;
        };
        let (sin, cos) = angle.sin_cos();
        let half = Length::from_nm(length.nm() / 2);
        let along = half.scaled(cos)?;
        let lean = half.scaled(sin)?;
        let lean = if toward.is_positive() {
            lean
        } else {
            lean.checked_neg()?
        };
        let shift = |sign: i64| -> Option<Point> {
            let at = |on: Axis, by: Length| -> Option<Length> {
                centre.get(on).checked_add(by.checked_mul(sign)?)
            };
            Some(
                centre
                    .with(*axis, at(*axis, along)?)
                    .with(toward.axis(), at(toward.axis(), lean)?),
            )
        };
        Some((shift(-1)?, shift(1)?))
    }

    /// The corners of the box around a slanted cylinder, rounded outward;
    /// `None` for any other shape or on overflow.
    fn slanted_corners(&self) -> Option<(Point, Point)> {
        let MemberShape::Slanted {
            axis,
            toward,
            angle,
            diameter,
            ..
        } = self
        else {
            return None;
        };
        let (a, b) = self.slanted_ends()?;
        let (sin, cos) = angle.sin_cos();
        let radius = Length::from_nm(diameter.nm().saturating_add(1) / 2);
        // How far the end discs reach past the axis on each world axis,
        // rounded up by a nanometre.
        let reach = |on: Axis| -> Option<i64> {
            let value = if on == *axis {
                radius.scaled(sin)?
            } else if on == toward.axis() {
                radius.scaled(cos)?
            } else {
                return Some(radius.nm());
            };
            value.nm().checked_add(1)
        };
        let mut low = a;
        let mut high = a;
        for on in Axis::ALL {
            let (x, y) = (a.get(on).nm(), b.get(on).nm());
            let r = reach(on)?;
            low = low.with(on, Length::from_nm(x.min(y).checked_sub(r)?));
            high = high.with(on, Length::from_nm(x.max(y).checked_add(r)?));
        }
        Some((low, high))
    }

    /// The world box around the shape, at rest: exact for a box, a
    /// cylinder and a prism; for a bar, its corners rounded outward to the
    /// nanometre; for a slanted cylinder, rounded outward to the
    /// nanometre.
    pub fn bounds(&self) -> WorldBox {
        // A box from its extent along `axis` and on the two others.
        let boxed = |axis: Axis, start: Length, end: Length, low: [i64; 2], high: [i64; 2]| {
            let [first, second] = axis.others();
            let corner = |along: Length, [a, b]: [i64; 2]| {
                Point {
                    across: Length::ZERO,
                    up: Length::ZERO,
                    deep: Length::ZERO,
                }
                .with(axis, along)
                .with(first, Length::from_nm(a))
                .with(second, Length::from_nm(b))
            };
            WorldBox::new(corner(start, low), corner(end, high))
        };
        match self {
            MemberShape::Box(bounds) => *bounds,
            MemberShape::Cylinder {
                axis,
                centre: [a, b],
                diameter,
                start,
                end,
            } => {
                let radius = diameter.nm() / 2;
                let rounded = diameter.nm() - radius;
                boxed(
                    *axis,
                    *start,
                    *end,
                    [a.nm().saturating_sub(radius), b.nm().saturating_sub(radius)],
                    [
                        a.nm().saturating_add(rounded),
                        b.nm().saturating_add(rounded),
                    ],
                )
            }
            MemberShape::Slanted { .. } => {
                let (low, high) = self.slanted_corners().unwrap_or_default();
                WorldBox::new(low, high)
            }
            MemberShape::Prism {
                axis,
                points,
                start,
                end,
            } => {
                let mut low = [i64::MAX; 2];
                let mut high = [i64::MIN; 2];
                for [a, b] in points {
                    for (slot, value) in [(0, a.nm()), (1, b.nm())] {
                        if let (Some(l), Some(h)) = (low.get_mut(slot), high.get_mut(slot)) {
                            *l = (*l).min(value);
                            *h = (*h).max(value);
                        }
                    }
                }
                if points.is_empty() {
                    low = [0; 2];
                    high = [0; 2];
                }
                boxed(*axis, *start, *end, low, high)
            }
            MemberShape::Bar {
                axis,
                from,
                to,
                thickness,
                start,
                end,
            } => {
                // The bar is a rectangle `thickness` wide reaching half its
                // thickness past each end: each corner lies
                // `t (|dx| + |dy|) / 2L` past its end's centre on one axis
                // and `t (|dy| + |dx|) / 2L` on the other (L the bar's
                // length), so the box grows by that on both, rounded up.
                let [fa, fb] = from.map(|l| i128::from(l.nm()));
                let [ta, tb] = to.map(|l| i128::from(l.nm()));
                let (dx, dy) = (ta.saturating_sub(fa), tb.saturating_sub(fb));
                let square = dx.saturating_mul(dx).saturating_add(dy.saturating_mul(dy));
                let thick = i128::from(thickness.nm());
                let grow = match integer_root(square) {
                    0 => thick.saturating_add(1) / 2,
                    length => {
                        let numerator = thick.saturating_mul(
                            dx.saturating_abs().saturating_add(dy.saturating_abs()),
                        );
                        let denominator = length.saturating_mul(2);
                        // Rounded up, and up again for the root's floor.
                        numerator / denominator + 1
                    }
                };
                let narrow = |value: i128| {
                    i64::try_from(value).unwrap_or(if value < 0 { i64::MIN } else { i64::MAX })
                };
                boxed(
                    *axis,
                    *start,
                    *end,
                    [
                        narrow(fa.min(ta).saturating_sub(grow)),
                        narrow(fb.min(tb).saturating_sub(grow)),
                    ],
                    [
                        narrow(fa.max(ta).saturating_add(grow)),
                        narrow(fb.max(tb).saturating_add(grow)),
                    ],
                )
            }
        }
    }
}

/// The integer square root of a non-negative number, rounded down
/// (Newton's method from above); zero for zero or less.
fn integer_root(square: i128) -> i128 {
    if square <= 0 {
        return 0;
    }
    let mut root = square;
    loop {
        let next = root.saturating_add(square / root) / 2;
        if next >= root {
            return root;
        }
        root = next;
    }
}

/// Fasteners a piece of hardware consumes (its kind's `uses:`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HardwareFastener {
    /// The fastener kind's name.
    pub kind: String,
    /// Its diameter.
    pub diameter: Length,
    /// Its length.
    pub length: Length,
    /// How many.
    pub count: u32,
}

/// The name of a role of a kind, as the kind writes it: `piece`, `on`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
pub struct RoleName(String);

impl RoleName {
    /// The name, if `word` is a word (letters, digits and `_`, not starting
    /// with a digit).
    pub fn new(word: &str) -> Option<RoleName> {
        stave_lang::ast::is_word(word).then(|| RoleName(word.to_owned()))
    }

    /// The name as written.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// What a role of a piece of hardware is bound to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "id")]
pub enum RoleValue {
    /// A part.
    Part(PartId),
    /// A region.
    Region(RegionId),
}

/// The name of a param or field of a kind, as the kind writes it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
pub struct EntryName(String);

impl EntryName {
    /// The name, if `word` is a word (letters, digits and `_`, not starting
    /// with a digit).
    pub fn new(word: &str) -> Option<EntryName> {
        stave_lang::ast::is_word(word).then(|| EntryName(word.to_owned()))
    }

    /// The name as written.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The value of a kind's param or field for one piece of hardware.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "value")]
pub enum EntryValue {
    /// A length, rounded to the nanometre.
    Length(Length),
    /// An angle.
    Angle(Angle),
    /// A count.
    Count(u32),
    /// A truth value.
    Bool(bool),
    /// A text.
    Text(String),
}

/// An assembly: an instance of a component, or its mirror image.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Assembly {
    /// Its id.
    pub id: AssemblyId,
    /// Its name as written: `drawer[top]`.
    pub name: String,
    /// The component's name.
    pub component: String,
    /// The assembly it is inside.
    pub parent: Option<AssemblyId>,
    /// The assembly it is the mirror image of.
    pub mirror_of: Option<AssemblyId>,
    /// Every part inside it, nested assemblies included.
    pub parts: Vec<PartId>,
    /// Every piece of hardware inside it.
    pub hardware: Vec<HardwareId>,
    /// Its mechanism: the hardware with a drive that moves it, and any
    /// that follow (grammar, "Mechanisms"). `None` when nothing moves it.
    pub mechanism: Option<crate::mechanism::Mechanism>,
}

/// A region.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Region {
    /// Its id.
    pub id: RegionId,
    /// Its name as written: `drawer_bay[top]`, `door_l.door`.
    pub name: String,
    /// The assembly it belongs to.
    pub assembly: Option<AssemblyId>,
    /// Its box.
    pub bounds: WorldBox,
    /// `true` for the project's root, made with `box(across:, up:, deep:)`:
    /// its bottom face is the floor.
    pub root: bool,
}

/// A motion setting in a scene.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PoseValue {
    /// The rest position.
    Closed,
    /// Fully open.
    Open,
    /// Turned by an angle.
    Angle(Angle),
    /// Travelled a distance.
    Travel(Length),
}

/// One `pose` of a scene.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ScenePose {
    /// The assembly.
    pub assembly: AssemblyId,
    /// Its motion value.
    pub value: PoseValue,
}

/// Something a scene shows or hides.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SceneItem {
    /// A part.
    Part(PartId),
    /// An assembly.
    Assembly(AssemblyId),
    /// A piece of hardware.
    Hardware(HardwareId),
}

/// A plane cutting the view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Plane {
    /// The axis it is perpendicular to.
    #[serde(serialize_with = "serialize_axis")]
    pub axis: Axis,
    /// Where along that axis.
    pub position: Length,
}

/// A named view of the design.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Scene {
    /// Its id.
    pub id: SceneId,
    /// Its name.
    pub name: String,
    /// Its title.
    pub title: Option<String>,
    /// Motion settings.
    pub poses: Vec<ScenePose>,
    /// Show only these; `None` shows everything.
    pub show: Option<Vec<SceneItem>>,
    /// Hide these.
    pub hide: Vec<SceneItem>,
    /// Pull parts apart along their joints' insert directions.
    pub explode: bool,
    /// Cut the view at a plane.
    pub section: Option<Plane>,
}

// ---------------------------------------------------------------------------
// Serialisation of vocabulary words
// ---------------------------------------------------------------------------

fn serialize_word<S: serde::Serializer, W: Word>(
    value: &W,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(value.word())
}

fn serialize_species<S: serde::Serializer>(
    value: &Species,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(value.word())
}

fn serialize_optional_species<S: serde::Serializer>(
    value: &Option<Species>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    match value {
        Some(species) => serializer.serialize_some(species.word()),
        None => serializer.serialize_none(),
    }
}

fn serialize_axis<S: serde::Serializer>(value: &Axis, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(value.word())
}

fn serialize_part_axis<S: serde::Serializer>(
    value: &PartAxis,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(value.word())
}

fn serialize_face<S: serde::Serializer>(value: &Face, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(value.word())
}

fn serialize_optional_face<S: serde::Serializer>(
    value: &Option<Face>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    match value {
        Some(face) => serializer.serialize_str(face.word()),
        None => serializer.serialize_none(),
    }
}

/// Vocabulary enums with a source word.
trait Word {
    fn word(&self) -> &'static str;
}

impl Word for OutputUnit {
    fn word(&self) -> &'static str {
        OutputUnit::word(*self)
    }
}

impl Word for StockForm {
    fn word(&self) -> &'static str {
        StockForm::word(*self)
    }
}

impl Word for Grain {
    fn word(&self) -> &'static str {
        Grain::word(*self)
    }
}

impl Word for Measure {
    fn word(&self) -> &'static str {
        Measure::word(*self)
    }
}

impl fmt::Display for End {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            End::Start => "start",
            End::End => "end",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mm(value: i64) -> Length {
        Length::from_mm(value)
    }

    #[test]
    fn a_slanted_cylinder_has_its_ends_on_its_slant() {
        // 32mm long, turned 15deg from across toward up, centred on the
        // origin: its ends are 15.454813mm along and 4.141105mm up from it.
        let shape = MemberShape::Slanted {
            centre: Point::default(),
            axis: Axis::Across,
            toward: Direction::Up,
            angle: Angle::from_deg(15),
            diameter: mm(3),
            length: mm(32),
        };
        let (a, b) = shape.slanted_ends().unwrap();
        assert_eq!(b.across, Length::from_nm(15_454_813));
        assert_eq!(b.up, Length::from_nm(4_141_105));
        assert_eq!(a.across, Length::from_nm(-15_454_813));
        assert_eq!(a.up, Length::from_nm(-4_141_105));
        // The box round it reaches past the ends by the end discs.
        let bounds = shape.bounds();
        assert!(bounds.max().across > b.across);
        assert!(bounds.max().up > b.up);
        assert_eq!(bounds.max().deep, Length::from_nm(1_500_000));
    }

    #[test]
    fn a_member_shape_bounds_its_wood() {
        // A cylinder up the `up` axis: its centre plus and minus half its
        // diameter on across and deep.
        let cylinder = MemberShape::Cylinder {
            axis: Axis::Up,
            centre: [mm(10), mm(20)],
            diameter: mm(6),
            start: mm(0),
            end: mm(5),
        };
        let bounds = cylinder.bounds();
        assert_eq!(bounds.min().get(Axis::Across), mm(7));
        assert_eq!(bounds.max().get(Axis::Deep), mm(23));
        assert_eq!(bounds.size(Axis::Up), mm(5));
        // A bar along `across` reaches half its thickness past each end and
        // either side; along a diagonal its corners reach further, rounded
        // out, never in.
        let bar = |to: [Length; 2]| MemberShape::Bar {
            axis: Axis::Deep,
            from: [mm(0), mm(0)],
            to,
            thickness: mm(2),
            start: mm(0),
            end: mm(1),
        };
        let straight = bar([mm(0), mm(10)]).bounds();
        assert!(straight.min().get(Axis::Up) <= mm(-1));
        let end = straight.max().get(Axis::Across).nm() - mm(11).nm();
        assert!((0..2).contains(&end), "{end}");
        let diagonal = bar([mm(10), mm(10)]).bounds();
        // Each corner is sqrt(2) = 1.414..mm past its end on both axes.
        let past = diagonal.max().get(Axis::Up).nm() - mm(10).nm();
        assert!((1_414_214..1_414_217).contains(&past), "{past}");
    }

    #[test]
    fn orientations_need_three_axes() {
        assert!(Orientation::new(Direction::Up, Direction::Down, Direction::Right).is_err());
        let side = Orientation::from_length_and_width(Direction::Up, Direction::Back).unwrap();
        assert_eq!(side.thickness(), Direction::Right);
        assert_eq!(side.part_axis(Axis::Across), PartAxis::Thickness);
        let mirrored = side.mirrored(Axis::Across);
        assert_eq!(mirrored.thickness(), Direction::Left);
        assert_ne!(side.is_reflection(), mirrored.is_reflection());
    }

    #[test]
    fn poses_map_local_boxes_to_world() {
        // A side standing on a 19mm base at the left: length up, width back.
        let pose = Pose {
            origin: Point {
                across: mm(0),
                up: mm(19),
                deep: mm(0),
            },
            orientation: Orientation::from_length_and_width(Direction::Up, Direction::Back)
                .unwrap(),
        };
        let size = Size3 {
            length: mm(1662),
            width: mm(560),
            thickness: mm(19),
        };
        let world = pose.world_box(size);
        assert_eq!(
            world.min(),
            Point {
                across: mm(0),
                up: mm(19),
                deep: mm(0)
            }
        );
        assert_eq!(
            world.max(),
            Point {
                across: mm(19),
                up: mm(1681),
                deep: mm(560)
            }
        );
        // Its mirror image about across = 450mm.
        let mirrored = Pose {
            origin: Point {
                across: mm(900),
                up: mm(19),
                deep: mm(0),
            },
            orientation: pose.orientation.mirrored(Axis::Across),
        };
        let world = mirrored.world_box(size);
        assert_eq!(world.min().across, mm(881));
        assert_eq!(world.max().across, mm(900));
        assert_eq!(world.face(Face::Left), mm(881));
    }
}
