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
//! **Hardware frames** work the same way for the envelope: local `length`
//! runs along the piece's length, local `thickness` stands off the mount
//! face (outward), local `width` takes the remaining axis. A cylinder's own
//! axis is the local `thickness`; its diameter spans `length` and `width`.
//! A hinge's cup and a butt hinge's leaf are the exception: they are sunk
//! into the mount face, so their `thickness` runs into the piece.
//!
//! **Hardware members.** A hinge and a slide each have a member on the
//! piece, which moves with the piece's assembly ([`Hardware::envelope`]: a
//! hinge's cup or leaf, a slide's drawer member) and a member fixed to the
//! carcass, which stays put ([`Hardware::fixed`]: a cup hinge's mounting
//! plate on the inside face of `on`, a butt hinge's other leaf sunk into
//! `on`, a slide's cabinet member). A cup hinge also has its mechanism
//! ([`Hardware::mechanism`]): the arm on the plate and two links, which
//! move the door on a four-bar [`Linkage`] ([`Motion::Linkage`]).

use std::fmt;

use serde::Serialize;
use stave_lang::ast::{BaseKind, CutOperation};
use stave_lang::placement::{Axis, Direction, Face, PartAxis};
use stave_lang::units::{Angle, Length, Rational};
use stave_schema::species::Species;
use stave_schema::words::{Grain, Measure, OutputUnit, StockForm};

pub use crate::ids::{AssemblyId, HardwareId, JointId, KindId, PartId, RegionId, SceneId};
pub use crate::linkage::{Branch, Link, Linkage, LinkageError, LinkageFrame, PlanePoint};
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
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

/// A fully placed design.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SolvedDesign {
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
}

impl SolvedDesign {
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

    /// The lowest point of any part: where the floor is.
    pub fn floor(&self) -> Option<Length> {
        self.parts.iter().map(|p| p.cut_box().min().up).min()
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
    /// Finished thickness; `None` for dowels.
    pub thickness: Option<Length>,
    /// Diameter of dowel stock.
    pub diameter: Option<Length>,
    /// A fixed width the stock is sold at.
    pub width: Option<Length>,
    /// The widest board available.
    pub max_width: Option<Length>,
    /// The longest board available.
    pub max_length: Option<Length>,
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
        /// The pair (index into [`Joint::pairs`]).
        pair: usize,
    },
    /// A part a board joint makes (a spline): once per seam of a part made
    /// of boards, or once for a kind with one `Parts` role.
    Boards {
        /// The part made of boards.
        part: PartId,
        /// The seam (index into [`Boards::seams`]); `None` for a kind
        /// across all of the part's boards.
        seam: Option<usize>,
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
        pair: usize,
    },
    /// A hinge cup, a hinge leaf's mortise or the screw holes of a pull.
    Hardware {
        /// The hardware.
        hardware: HardwareId,
    },
    /// A `cut` of a joint kind that joins the boards of a part.
    Boards {
        /// The part made of boards.
        part: PartId,
        /// The seam (index into [`Boards::seams`]); `None` for a kind
        /// across all of the part's boards.
        seam: Option<usize>,
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
    /// Round holes.
    Drill {
        /// Hole diameter.
        diameter: Length,
        /// Hole depth.
        depth: Depth,
        /// How many, spaced along the longer side.
        count: u32,
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
        }
    }
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
    /// A hardware recess, such as a butt hinge's leaf mortise: the region
    /// the hardware fills, in this part's local coordinates.
    Recess {
        /// The region the hardware fills.
        footprint: LocalBox,
    },
    /// A hardware cut, centred at a point on the face.
    Mount {
        /// The centre, on the face.
        centre: LocalPoint,
        /// For two holes: their distance apart along the length.
        centres: Option<Length>,
    },
}

/// One cut a part receives.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Cut {
    /// Where it comes from.
    pub source: CutSource,
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

/// How far an assembly step moves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StepAmount {
    /// A distance (push).
    Length(Length),
    /// An angle (rotate).
    Angle(Angle),
}

/// What an assembly step does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StepKind {
    /// Move along an axis until seated (or exactly `by`).
    Push,
    /// Turn about an axis.
    Rotate,
    /// Knock in a part the joint makes.
    Drive,
}

/// One step of `insert:`, bound to parts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AssemblyStep {
    /// What it does.
    pub kind: StepKind,
    /// The part that moves.
    pub part: PartId,
    /// The moving part's own axis, as placed in the finished joint.
    #[serde(serialize_with = "serialize_part_axis")]
    pub axis: PartAxis,
    /// `by:`
    pub by: Option<StepAmount>,
    /// `angle:` of a push.
    pub angle: Option<Angle>,
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
}

// ---------------------------------------------------------------------------
// Hardware, assemblies, regions, scenes
// ---------------------------------------------------------------------------

/// A hardware envelope, exact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum Envelope {
    /// A box in the envelope's frame.
    Box {
        /// Local length.
        length: Length,
        /// Local width.
        width: Length,
        /// Local thickness (stand-off).
        thickness: Length,
    },
    /// A cylinder whose axis is the local thickness.
    Cylinder {
        /// Diameter (spans local length and width).
        diameter: Length,
        /// Length along its axis (stand-off).
        length: Length,
    },
}

impl Envelope {
    /// The size of the envelope's local box.
    pub fn size(&self) -> Size3 {
        match *self {
            Envelope::Box {
                length,
                width,
                thickness,
            } => Size3 {
                length,
                width,
                thickness,
            },
            Envelope::Cylinder { diameter, length } => Size3 {
                length: diameter,
                width: diameter,
                thickness: length,
            },
        }
    }
}

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
}

impl HardwareBase {
    /// The base for a kind's base, if it is mounted hardware.
    pub fn from_base(base: BaseKind) -> Option<HardwareBase> {
        match base {
            BaseKind::Slide => Some(HardwareBase::Slide),
            BaseKind::Hinge => Some(HardwareBase::Hinge),
            BaseKind::Pull => Some(HardwareBase::Pull),
            BaseKind::Joint | BaseKind::Fastener | BaseKind::Tool => None,
        }
    }
}

/// The catalog data of a piece of hardware.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum HardwareData {
    /// A slide.
    Slide {
        /// Closed length.
        length: Length,
        /// Gap it needs beside the piece.
        clearance: Length,
        /// How far it opens.
        travel: Length,
        /// The region it runs in.
        opening: RegionId,
    },
    /// A butt hinge: two leaves on a pin, each housed in a mortise.
    ButtHinge {
        /// How far it opens.
        opening_angle: Angle,
        /// The part its fixed leaf is housed in.
        on: PartId,
    },
    /// A cup hinge.
    Hinge {
        /// How far it opens.
        opening_angle: Angle,
        /// Cup diameter.
        cup_diameter: Length,
        /// Cup depth.
        cup_depth: Length,
        /// Cup inset from the edge.
        cup_inset: Length,
        /// The part it is fixed to.
        on: PartId,
        /// How far the piece overlays `on`: from the piece's edge nearer
        /// `on` to the face of `on` the plate is screwed to (negative when
        /// the edge is short of it).
        overlay: Length,
        /// What the catalogue says about the overlay, when the kind gives
        /// its `base_overlay`.
        overlay_range: Option<OverlayRange>,
    },
    /// A pull.
    Pull {
        /// Screw centres, for two screws.
        centres: Option<Length>,
        /// Diameter of a round pull.
        diameter: Option<Length>,
        /// Screw diameter.
        screw_diameter: Length,
    },
}

/// The overlays a cup hinge can give: `base + cup_inset - plate_height`,
/// give or take `adjustment` on the arm's side screw.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct OverlayRange {
    /// The overlay with the cup at the door's edge on a 0mm plate (the
    /// catalogue's base or fixed distance).
    pub base: Length,
    /// The mounting plate's height (the catalogue's distance).
    pub plate_height: Length,
    /// How far the side screw moves the door, each way.
    pub adjustment: Length,
}

impl OverlayRange {
    /// The overlay at the side screw's middle setting, for a cup this far
    /// from the door's edge.
    pub fn nominal(&self, cup_inset: Length) -> Length {
        Length::from_nm(
            self.base
                .nm()
                .saturating_add(cup_inset.nm())
                .saturating_sub(self.plate_height.nm()),
        )
    }
}

/// A model for looks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Model {
    /// The source as written.
    pub source: String,
    /// `align: node("name")`; `None` for `align: bounds`.
    pub node: Option<String>,
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
    /// The member on the piece: the whole of a pull, a hinge's cup (sunk
    /// into the mount face), a slide's drawer member. It moves with the
    /// piece's assembly.
    pub envelope: Envelope,
    /// Its local origin and frame.
    pub pose: Pose,
    /// Its world box.
    pub bounds: WorldBox,
    /// The member fixed to the carcass, which stays put when the assembly
    /// moves: a hinge's mounting plate, a slide's cabinet member. `None`
    /// for pulls.
    pub fixed: Option<HardwareMember>,
    /// Catalog data.
    pub data: HardwareData,
    /// A cup hinge's mechanism: its arm, links and pins. `None` for every
    /// other piece of hardware.
    pub mechanism: Option<CupMechanism>,
    /// A model, when the kind has one.
    pub model: Option<Model>,
}

/// The mechanism of a cup hinge: the arm on the mounting plate, and two
/// links from the arm to the boss in the cup (grammar, "Hardware
/// placement"). The cup ([`Hardware::envelope`]) moves with the door, the
/// arm and plate stay put, and each link turns about its pin in the arm as
/// the linkage moves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct CupMechanism {
    /// The four pins at rest and the range: the same linkage the
    /// assembly's motion has when this is its first hinge.
    pub linkage: Linkage,
    /// The arm's body, fixed: a box on the plate, from the back of the
    /// plate to the door's back face.
    pub arm: HardwareMember,
    /// The arm's front end, fixed: a box reaching into the cup (when the
    /// door is closed) that carries the arm pins.
    pub knuckle: HardwareMember,
    /// The two links' sizes, in the order of [`Linkage::links`].
    pub links: [LinkBar; 2],
    /// The diameter of the four pins.
    pub pin_diameter: Length,
    /// How long the pins are, along the linkage's axis (as wide as the
    /// widest link).
    pub pin_length: Length,
}

/// The size of a link: a flat bar from pin to pin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct LinkBar {
    /// Along the pins.
    pub width: Length,
    /// Across the bar, in the linkage's plane.
    pub thickness: Length,
    /// The gap down its middle along the pins: a strap is two flanges with
    /// the other link between them; zero for a solid bar.
    pub slot: Length,
}

impl Hardware {
    /// `true` for a butt hinge: two leaves, each housed in a mortise.
    pub fn is_butt_hinge(&self) -> bool {
        matches!(self.data, HardwareData::ButtHinge { .. })
    }
}

/// A member of a piece of hardware fixed to the carcass.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct HardwareMember {
    /// Its shape.
    pub envelope: Envelope,
    /// Its local origin and frame.
    pub pose: Pose,
    /// Its world box.
    pub bounds: WorldBox,
}

/// The line a hinged assembly turns about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Pivot {
    /// The world axis the line runs along.
    #[serde(serialize_with = "serialize_axis")]
    pub axis: Axis,
    /// A point on the line (its coordinate along `axis` is the hinged
    /// part's lowest point along it, and carries no meaning).
    pub through: Point,
    /// The direction from the line toward the rest of the assembly.
    #[serde(serialize_with = "serialize_direction")]
    pub free_side: Direction,
    /// The direction the free side moves as it opens.
    #[serde(serialize_with = "serialize_direction")]
    pub swing_toward: Direction,
}

/// How an assembly moves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum Motion {
    /// It turns about a pivot line, up to an angle.
    Turn {
        /// The line.
        pivot: Pivot,
        /// How far it opens.
        opening_angle: Angle,
    },
    /// It turns and moves on a four-bar linkage (a cup hinge): one input,
    /// the angle the door has turned, up to the linkage's opening angle.
    Linkage {
        /// The linkage.
        linkage: Linkage,
    },
    /// It travels in one direction, up to a distance.
    Travel {
        /// The direction it opens in.
        #[serde(serialize_with = "serialize_direction")]
        toward: Direction,
        /// How far it opens.
        travel: Length,
    },
}

impl Motion {
    /// `true` for a hinge's motion (an angle), `false` for a slide's.
    pub fn turns(&self) -> bool {
        match self {
            Motion::Turn { .. } | Motion::Linkage { .. } => true,
            Motion::Travel { .. } => false,
        }
    }

    /// How far a hinge's motion opens; `None` for a slide.
    pub fn opening_angle(&self) -> Option<Angle> {
        match self {
            Motion::Turn { opening_angle, .. } => Some(*opening_angle),
            Motion::Linkage { linkage } => Some(linkage.opening_angle()),
            Motion::Travel { .. } => None,
        }
    }

    /// For a hinge: the direction from the hinge edge toward the middle of
    /// the door, and the direction the door swings to.
    pub fn swing(&self) -> Option<(Direction, Direction)> {
        match self {
            Motion::Turn { pivot, .. } => Some((pivot.free_side, pivot.swing_toward)),
            Motion::Linkage { linkage } => {
                Some((linkage.frame().free_side(), linkage.frame().swing_toward()))
            }
            Motion::Travel { .. } => None,
        }
    }
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
    /// Its motion, from its hinges or slides.
    pub motion: Option<Motion>,
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

fn serialize_direction<S: serde::Serializer>(
    value: &Direction,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(value.word())
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
