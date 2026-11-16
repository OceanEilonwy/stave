//! Typed placement vocabulary (grammar draft 3, see `docs/grammar.md`).
//!
//! `place { }` blocks are lowered into these types after name resolution.
//! The vocabulary has **no way to express a coordinate**: every position is a
//! relationship to a named anchor on something already placed, plus an
//! optional measured offset.
//!
//! Each axis is set by exactly one term and anchors only point at things
//! placed earlier, so positions are worked out directly in dependency order;
//! no solver is involved. Joints never position parts.
//!
//! The types make the placement rules hold by construction:
//!
//! - a [`Placement`] has one [`Translation`] per world axis, so an axis can't
//!   be missing or set twice once a part is lowered;
//! - an [`Orientation`] is always two perpendicular directions;
//! - an [`ArraySlot`] is always inside its array;
//! - a [`HardwarePlacement`] never has a term on the axis its mount face
//!   already fixes.
//!
//! Arrays are lowered **per element**: every element is its own part with its
//! own [`Placement`]. `spaced` becomes a [`Translation::Spaced`] carrying the
//! element's slot, `corners` becomes two [`Translation::Flush`] terms, and a
//! `[same]` index is resolved to the matching element's anchor.
//!
//! Motions (hinges, slides) come from hardware. Placement fixes the *rest*
//! pose (doors closed, drawers shut); scenes set motion values relative to it.

use std::fmt;
use std::num::NonZeroU32;

use crate::units::{Angle, Length};

/// Index of a resolved part (an array element is its own part).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PartId(pub u32);

/// Index of a resolved region (an opening, a bay).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RegionId(pub u32);

/// Index of a resolved assembly (an instance of a component).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AssemblyId(pub u32);

/// World axes, named from the viewer's point of view standing at the front.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Axis {
    /// Floor to ceiling.
    Up,
    /// Left to right.
    Across,
    /// Front to back.
    Deep,
}

impl Axis {
    /// Every world axis.
    pub const ALL: [Axis; 3] = [Axis::Up, Axis::Across, Axis::Deep];

    /// The axis as written in source.
    pub fn word(self) -> &'static str {
        match self {
            Axis::Up => "up",
            Axis::Across => "across",
            Axis::Deep => "deep",
        }
    }

    /// The axis for a word, if it is one.
    pub fn from_word(word: &str) -> Option<Axis> {
        Axis::ALL.into_iter().find(|axis| axis.word() == word)
    }

    /// The two other axes, in the order of [`Axis::ALL`].
    pub fn others(self) -> [Axis; 2] {
        match self {
            Axis::Up => [Axis::Across, Axis::Deep],
            Axis::Across => [Axis::Up, Axis::Deep],
            Axis::Deep => [Axis::Up, Axis::Across],
        }
    }

    /// The axis that is neither `self` nor `other`; `None` if they are equal.
    pub fn third(self, other: Axis) -> Option<Axis> {
        Axis::ALL
            .into_iter()
            .find(|axis| *axis != self && *axis != other)
            .filter(|_| self != other)
    }
}

/// A part's own axes. Joint kinds use these, because a joint doesn't know how
/// its parts are placed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum PartAxis {
    /// Along the part's length.
    Length,
    /// Along the part's width.
    Width,
    /// Through the part's thickness.
    Thickness,
}

impl PartAxis {
    /// Every part axis.
    pub const ALL: [PartAxis; 3] = [PartAxis::Length, PartAxis::Width, PartAxis::Thickness];

    /// The axis as written in source.
    pub fn word(self) -> &'static str {
        match self {
            PartAxis::Length => "length",
            PartAxis::Width => "width",
            PartAxis::Thickness => "thickness",
        }
    }

    /// The part axis for a word, if it is one.
    pub fn from_word(word: &str) -> Option<PartAxis> {
        PartAxis::ALL.into_iter().find(|axis| axis.word() == word)
    }
}

/// A direction along a world axis. Used by `orient`, where the sign matters
/// for parts that aren't symmetric end to end (a turned foot with its tenon
/// at one end).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Direction {
    /// Toward the ceiling.
    Up,
    /// Toward the floor.
    Down,
    /// Toward the left, seen from the front.
    Left,
    /// Toward the right, seen from the front.
    Right,
    /// Toward the viewer.
    Front,
    /// Away from the viewer.
    Back,
}

impl Direction {
    /// Every direction.
    pub const ALL: [Direction; 6] = [
        Direction::Up,
        Direction::Down,
        Direction::Left,
        Direction::Right,
        Direction::Front,
        Direction::Back,
    ];

    /// The direction as written in source.
    pub fn word(self) -> &'static str {
        match self {
            Direction::Up => "up",
            Direction::Down => "down",
            Direction::Left => "left",
            Direction::Right => "right",
            Direction::Front => "front",
            Direction::Back => "back",
        }
    }

    /// The direction for a word, if it is one.
    pub fn from_word(word: &str) -> Option<Direction> {
        Direction::ALL
            .into_iter()
            .find(|direction| direction.word() == word)
    }

    /// The axis the direction runs along.
    pub fn axis(self) -> Axis {
        match self {
            Direction::Up | Direction::Down => Axis::Up,
            Direction::Left | Direction::Right => Axis::Across,
            Direction::Front | Direction::Back => Axis::Deep,
        }
    }

    /// `true` for up, right and back.
    pub fn is_positive(self) -> bool {
        match self {
            Direction::Up | Direction::Right | Direction::Back => true,
            Direction::Down | Direction::Left | Direction::Front => false,
        }
    }

    /// The direction pointing the other way.
    pub fn opposite(self) -> Direction {
        match self {
            Direction::Up => Direction::Down,
            Direction::Down => Direction::Up,
            Direction::Left => Direction::Right,
            Direction::Right => Direction::Left,
            Direction::Front => Direction::Back,
            Direction::Back => Direction::Front,
        }
    }

    /// The face whose outward normal is this direction.
    pub fn face(self) -> Face {
        match self {
            Direction::Up => Face::Top,
            Direction::Down => Face::Bottom,
            Direction::Left => Face::Left,
            Direction::Right => Face::Right,
            Direction::Front => Face::Front,
            Direction::Back => Face::Back,
        }
    }
}

/// Faces of a part, region or hardware envelope, named by the world direction
/// they face once placed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Face {
    /// Faces up.
    Top,
    /// Faces down.
    Bottom,
    /// Faces left.
    Left,
    /// Faces right.
    Right,
    /// Faces the viewer.
    Front,
    /// Faces away from the viewer.
    Back,
}

impl Face {
    /// Every face.
    pub const ALL: [Face; 6] = [
        Face::Top,
        Face::Bottom,
        Face::Left,
        Face::Right,
        Face::Front,
        Face::Back,
    ];

    /// The face as written in source.
    pub fn word(self) -> &'static str {
        match self {
            Face::Top => "top",
            Face::Bottom => "bottom",
            Face::Left => "left",
            Face::Right => "right",
            Face::Front => "front",
            Face::Back => "back",
        }
    }

    /// The face for a word, if it is one.
    pub fn from_word(word: &str) -> Option<Face> {
        Face::ALL.into_iter().find(|face| face.word() == word)
    }

    /// The direction of the face's outward normal.
    pub fn direction(self) -> Direction {
        match self {
            Face::Top => Direction::Up,
            Face::Bottom => Direction::Down,
            Face::Left => Direction::Left,
            Face::Right => Direction::Right,
            Face::Front => Direction::Front,
            Face::Back => Direction::Back,
        }
    }

    /// Axis the face is perpendicular to.
    pub fn axis(self) -> Axis {
        self.direction().axis()
    }

    /// The face on the other side.
    pub fn opposite(self) -> Face {
        self.direction().opposite().face()
    }

    /// `true` if the face points in the positive direction of its axis
    /// (up, right, back).
    pub fn is_positive(self) -> bool {
        self.direction().is_positive()
    }
}

/// The four corners of a part's footprint, as the labels `corners(...)` binds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Corner {
    /// `front_left`
    FrontLeft,
    /// `front_right`
    FrontRight,
    /// `back_left`
    BackLeft,
    /// `back_right`
    BackRight,
}

impl Corner {
    /// Every corner.
    pub const ALL: [Corner; 4] = [
        Corner::FrontLeft,
        Corner::FrontRight,
        Corner::BackLeft,
        Corner::BackRight,
    ];

    /// The label an array element must have to sit in this corner.
    pub fn word(self) -> &'static str {
        match self {
            Corner::FrontLeft => "front_left",
            Corner::FrontRight => "front_right",
            Corner::BackLeft => "back_left",
            Corner::BackRight => "back_right",
        }
    }

    /// The corner for a label, if it is one.
    pub fn from_word(word: &str) -> Option<Corner> {
        Corner::ALL.into_iter().find(|corner| corner.word() == word)
    }

    /// The face of the part that bounds this corner along `across`.
    pub fn across_face(self) -> Face {
        match self {
            Corner::FrontLeft | Corner::BackLeft => Face::Left,
            Corner::FrontRight | Corner::BackRight => Face::Right,
        }
    }

    /// The face of the part that bounds this corner along `deep`.
    pub fn deep_face(self) -> Face {
        match self {
            Corner::FrontLeft | Corner::FrontRight => Face::Front,
            Corner::BackLeft | Corner::BackRight => Face::Back,
        }
    }

    /// `corners(part, inset: d)` for the element in this corner: flush with
    /// the two faces of `part` that form the corner, each moved in by `inset`.
    /// Returns the `across` term and the `deep` term.
    pub fn translations(self, part: PartId, inset: Length) -> (Translation, Translation) {
        let flush = |face| Translation::Flush {
            anchor: FaceAnchor::Part { part, face },
            inset,
        };
        (flush(self.across_face()), flush(self.deep_face()))
    }
}

/// A face of something placed: what `against` and `flush` take.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FaceAnchor {
    /// `base.top`
    Part {
        /// The part.
        part: PartId,
        /// Which of its faces.
        face: Face,
    },
    /// `front_opening.left`
    Region {
        /// The region.
        region: RegionId,
        /// Which of its faces.
        face: Face,
    },
}

impl FaceAnchor {
    /// Which face of its owner the anchor is.
    pub fn face(self) -> Face {
        match self {
            FaceAnchor::Part { face, .. } | FaceAnchor::Region { face, .. } => face,
        }
    }

    /// The world axis this anchor fixes.
    pub fn axis(self) -> Axis {
        self.face().axis()
    }
}

/// A plane that placement terms measure from: a face or a mid-plane.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Anchor {
    /// A face of a part or region.
    Face(FaceAnchor),
    /// `mid(base, across)`: the plane halfway between two opposite faces.
    PartMid {
        /// The part.
        part: PartId,
        /// The axis the plane is perpendicular to.
        axis: Axis,
    },
    /// `mid(door, across)` where `door` is a region.
    RegionMid {
        /// The region.
        region: RegionId,
        /// The axis the plane is perpendicular to.
        axis: Axis,
    },
}

impl Anchor {
    /// The world axis the anchor's plane is perpendicular to.
    pub fn axis(self) -> Axis {
        match self {
            Anchor::Face(face) => face.axis(),
            Anchor::PartMid { axis, .. } | Anchor::RegionMid { axis, .. } => axis,
        }
    }
}

impl From<FaceAnchor> for Anchor {
    fn from(face: FaceAnchor) -> Self {
        Anchor::Face(face)
    }
}

/// How an array is distributed between two anchors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Spacing {
    /// `evenly`: equal gaps everywhere, including at both ends.
    Evenly,
    /// `ends` or `ends(inset: d)`: first element flush with `from` and last
    /// flush with `to`, each moved inward by `inset`; equal gaps between.
    Ends {
        /// How far the first and last elements are moved inward; zero for
        /// plain `ends`.
        inset: Length,
    },
    /// `pitch(d)`: fixed centre-to-centre distance, starting flush at `from`.
    Pitch(Length),
    /// `gap(d)`: fixed gap between elements, starting flush at `from`.
    Gap(Length),
}

/// An element's place in its array: `index` of `count`, with
/// `index < count` always.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ArraySlot {
    index: u32,
    count: NonZeroU32,
}

impl ArraySlot {
    /// `None` if `index` is not inside an array of `count` elements.
    pub fn new(index: u32, count: NonZeroU32) -> Option<Self> {
        (index < count.get()).then_some(Self { index, count })
    }

    /// Zero-based position in declaration order.
    pub fn index(self) -> u32 {
        self.index
    }

    /// Number of elements in the array.
    pub fn count(self) -> NonZeroU32 {
        self.count
    }

    /// `true` for the first element.
    pub fn is_first(self) -> bool {
        self.index == 0
    }

    /// `true` for the last element.
    pub fn is_last(self) -> bool {
        self.index + 1 == self.count.get()
    }
}

/// Sets the position along one world axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Translation {
    /// `against(face)` (gap zero) or `against(face, gap: d)`: this item's
    /// opposite face is `gap` away from the anchor, on the anchor face's
    /// outward side.
    Against {
        /// The face it sits against.
        anchor: FaceAnchor,
        /// Distance from the face; zero when touching.
        gap: Length,
    },
    /// `flush(face)` (inset zero) or `flush(face, inset: d)`: this item's
    /// same-named face lies in the anchor's plane, moved by `inset` the other
    /// way from where the face points.
    Flush {
        /// The face it lines up with.
        anchor: FaceAnchor,
        /// How far it is moved inward; zero when exactly flush.
        inset: Length,
    },
    /// `center(item, axis)` (the item's two opposite faces) or
    /// `center(from: a, to: b)`: centred between two parallel anchors.
    Center {
        /// One of the two anchors.
        from: Anchor,
        /// The other one.
        to: Anchor,
    },
    /// `spaced(spacing, from: a, to: b)` for one element of an array.
    Spaced {
        /// How the array is distributed.
        spacing: Spacing,
        /// Where the array starts.
        from: Anchor,
        /// Where the array ends.
        to: Anchor,
        /// Which element this is.
        slot: ArraySlot,
    },
}

impl Translation {
    /// The world axis this term fixes. `None` when its two anchors are not on
    /// the same axis, which the type checker reports before lowering.
    pub fn axis(self) -> Option<Axis> {
        match self {
            Translation::Against { anchor, .. } | Translation::Flush { anchor, .. } => {
                Some(anchor.axis())
            }
            Translation::Center { from, to } | Translation::Spaced { from, to, .. } => {
                (from.axis() == to.axis()).then_some(from.axis())
            }
        }
    }
}

/// Why two directions can't orient a part.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NotPerpendicular {
    /// The direction given for the length.
    pub length: Direction,
    /// The direction given for the width, on the same axis.
    pub width: Direction,
}

impl fmt::Display for NotPerpendicular {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "length ({}) and width ({}) must run along different axes",
            self.length.word(),
            self.width.word()
        )
    }
}

impl std::error::Error for NotPerpendicular {}

/// How a part is turned: the directions its length and width run in. They
/// are always perpendicular; thickness takes the remaining axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Orientation {
    length: Direction,
    width: Direction,
}

impl Orientation {
    /// `orient(length: ..., width: ...)`. The two must be on different axes.
    pub fn new(length: Direction, width: Direction) -> Result<Self, NotPerpendicular> {
        if length.axis() == width.axis() {
            Err(NotPerpendicular { length, width })
        } else {
            Ok(Self { length, width })
        }
    }

    /// `orient(length: ...)` for a round part, where the width's direction
    /// doesn't matter. The width is given the first other axis's positive
    /// direction, so results are the same every run.
    pub fn round(length: Direction) -> Self {
        let width = match length.axis() {
            Axis::Up => Direction::Right,
            Axis::Across => Direction::Back,
            Axis::Deep => Direction::Right,
        };
        Self { length, width }
    }

    /// Direction the length runs from its start (station 0 of a profile).
    pub fn length(self) -> Direction {
        self.length
    }

    /// Direction the width runs.
    pub fn width(self) -> Direction {
        self.width
    }

    /// The world axis the thickness lies along.
    pub fn thickness_axis(self) -> Axis {
        // `new` and `round` never pair an axis with itself, so a third axis
        // always exists; the fallback only keeps this total.
        self.length
            .axis()
            .third(self.width.axis())
            .unwrap_or(self.length.axis())
    }

    /// The world axis one of the part's own axes lies along.
    pub fn world_axis(self, axis: PartAxis) -> Axis {
        match axis {
            PartAxis::Length => self.length.axis(),
            PartAxis::Width => self.width.axis(),
            PartAxis::Thickness => self.thickness_axis(),
        }
    }

    /// The part's own axis that lies along a world axis.
    pub fn part_axis(self, axis: Axis) -> PartAxis {
        if self.length.axis() == axis {
            PartAxis::Length
        } else if self.width.axis() == axis {
            PartAxis::Width
        } else {
            PartAxis::Thickness
        }
    }
}

impl Default for Orientation {
    /// Lying flat: length runs left to right, width runs front to back.
    fn default() -> Self {
        Orientation {
            length: Direction::Right,
            width: Direction::Back,
        }
    }
}

/// The fully lowered `place { }` of one part. Each world axis has exactly one
/// term, by construction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Placement {
    /// `ground`: the root part. Exactly one per project.
    Ground {
        /// How it is turned.
        orientation: Orientation,
    },
    /// Positioned by relationships.
    Related {
        /// How it is turned.
        orientation: Orientation,
        /// The term that fixes the position floor to ceiling.
        up: Translation,
        /// The term that fixes the position left to right.
        across: Translation,
        /// The term that fixes the position front to back.
        deep: Translation,
    },
}

impl Placement {
    /// How the part is turned.
    pub fn orientation(self) -> Orientation {
        match self {
            Placement::Ground { orientation } | Placement::Related { orientation, .. } => {
                orientation
            }
        }
    }

    /// The term that fixes an axis; `None` for the ground part.
    pub fn translation(self, axis: Axis) -> Option<Translation> {
        match self {
            Placement::Ground { .. } => None,
            Placement::Related {
                up, across, deep, ..
            } => Some(match axis {
                Axis::Up => up,
                Axis::Across => across,
                Axis::Deep => deep,
            }),
        }
    }
}

/// A placement term for hardware, with the world axis it fixes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AxisTerm {
    /// The axis the term fixes.
    pub axis: Axis,
    /// The term.
    pub translation: Translation,
}

/// The terms a hardware `place { }` holds, by how many axes the base kind
/// leaves open.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HardwareTerms {
    /// The base kind fixes everything (a slide).
    Fixed,
    /// One axis is open (a hinge: along the piece's length).
    One(AxisTerm),
    /// Both axes of the mount face are open (a pull).
    Two(AxisTerm, AxisTerm),
}

/// Why a hardware placement is invalid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HardwarePlacementError {
    /// A term fixes the axis the mount face already fixes.
    AxisFixedByMountFace(Axis),
    /// Two terms fix the same axis.
    AxisSetTwice(Axis),
}

impl fmt::Display for HardwarePlacementError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HardwarePlacementError::AxisFixedByMountFace(axis) => {
                write!(
                    f,
                    "position along '{}' is already set by the mount face",
                    axis.word()
                )
            }
            HardwarePlacementError::AxisSetTwice(axis) => {
                write!(f, "position along '{}' is set twice", axis.word())
            }
        }
    }
}

impl std::error::Error for HardwarePlacementError {}

/// The fully lowered placement of one piece of hardware: the face of its
/// `piece` it is mounted on, and the terms for the axes that leaves open.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct HardwarePlacement {
    piece: PartId,
    face: Face,
    terms: HardwareTerms,
}

impl HardwarePlacement {
    /// Checks that no term fixes the mount face's own axis and that two terms
    /// fix different axes.
    pub fn new(
        piece: PartId,
        face: Face,
        terms: HardwareTerms,
    ) -> Result<Self, HardwarePlacementError> {
        let fixed = face.axis();
        let check = |term: AxisTerm| {
            if term.axis == fixed {
                Err(HardwarePlacementError::AxisFixedByMountFace(fixed))
            } else {
                Ok(())
            }
        };
        match terms {
            HardwareTerms::Fixed => {}
            HardwareTerms::One(term) => check(term)?,
            HardwareTerms::Two(first, second) => {
                check(first)?;
                check(second)?;
                if first.axis == second.axis {
                    return Err(HardwarePlacementError::AxisSetTwice(first.axis));
                }
            }
        }
        Ok(Self { piece, face, terms })
    }

    /// The part the hardware is mounted on.
    pub fn piece(self) -> PartId {
        self.piece
    }

    /// The face of the piece it is mounted on. It fixes the axis it is
    /// normal to.
    pub fn face(self) -> Face {
        self.face
    }

    /// The terms for the axes the mount face leaves open.
    pub fn terms(self) -> HardwareTerms {
        self.terms
    }

    /// The term that fixes an axis, if the `place { }` block has one for it.
    pub fn translation(self, axis: Axis) -> Option<Translation> {
        let pick = |term: AxisTerm| (term.axis == axis).then_some(term.translation);
        match self.terms {
            HardwareTerms::Fixed => None,
            HardwareTerms::One(term) => pick(term),
            HardwareTerms::Two(first, second) => pick(first).or_else(|| pick(second)),
        }
    }
}

// ---------------------------------------------------------------------------
// Motion (used by scenes)
// ---------------------------------------------------------------------------

/// The line a hinged assembly turns about: the long edge of the hinge's
/// `piece` where two of its faces meet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Pivot {
    /// The part that carries the hinges (a door stile).
    pub piece: PartId,
    /// The face opposite the hinge's mount face.
    pub outer: Face,
    /// The face of the piece nearer the part the hinge is fixed to.
    pub edge: Face,
}

/// A degree of freedom that hardware gives an assembly. The whole assembly
/// moves as one piece; the rest pose from placement is the motion's zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Motion {
    /// Hinges: the assembly turns about a pivot line, from closed (zero) to
    /// the hinge kind's opening angle.
    Turn {
        /// The assembly that moves.
        moving: AssemblyId,
        /// The line it turns about.
        pivot: Pivot,
        /// How far it opens.
        opening_angle: Angle,
    },
    /// Slides: the assembly travels in one direction, from closed (zero) to
    /// the slide kind's travel.
    Travel {
        /// The assembly that moves.
        moving: AssemblyId,
        /// The direction it opens in.
        toward: Direction,
        /// How far it opens.
        travel: Length,
    },
}

impl Motion {
    /// The assembly that moves.
    pub fn moving(self) -> AssemblyId {
        match self {
            Motion::Turn { moving, .. } | Motion::Travel { moving, .. } => moving,
        }
    }
}

/// Value a scene's `pose` gives a motion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MotionValue {
    /// `closed`: the rest position.
    Closed,
    /// `open`: the hinge's opening angle or the slide's travel.
    Open,
    /// An angle, for hinges.
    Angle(Angle),
    /// A length, for slides.
    Travel(Length),
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: PartId = PartId(0);

    fn count(n: u32) -> NonZeroU32 {
        NonZeroU32::new(n).unwrap()
    }

    fn face(face: Face) -> FaceAnchor {
        FaceAnchor::Part { part: BASE, face }
    }

    #[test]
    fn words_round_trip() {
        for axis in Axis::ALL {
            assert_eq!(Axis::from_word(axis.word()), Some(axis));
        }
        for axis in PartAxis::ALL {
            assert_eq!(PartAxis::from_word(axis.word()), Some(axis));
        }
        for direction in Direction::ALL {
            assert_eq!(Direction::from_word(direction.word()), Some(direction));
        }
        for side in Face::ALL {
            assert_eq!(Face::from_word(side.word()), Some(side));
        }
        for corner in Corner::ALL {
            assert_eq!(Corner::from_word(corner.word()), Some(corner));
        }
        assert_eq!(Face::from_word("up"), None);
        assert_eq!(Direction::from_word("top"), None);
        assert_eq!(Axis::from_word("length"), None);
    }

    #[test]
    fn faces_and_directions_agree() {
        for side in Face::ALL {
            assert_eq!(side.direction().face(), side);
            assert_eq!(side.opposite().opposite(), side);
            assert_eq!(side.opposite().axis(), side.axis());
            assert_ne!(side.opposite().is_positive(), side.is_positive());
        }
        assert_eq!(Face::Top.axis(), Axis::Up);
        assert_eq!(Face::Left.opposite(), Face::Right);
        assert!(Face::Back.is_positive());
        assert!(!Direction::Front.is_positive());
    }

    #[test]
    fn axes_know_the_others() {
        assert_eq!(Axis::Across.others(), [Axis::Up, Axis::Deep]);
        assert_eq!(Axis::Up.third(Axis::Deep), Some(Axis::Across));
        assert_eq!(Axis::Up.third(Axis::Up), None);
    }

    #[test]
    fn orientation_must_be_perpendicular() {
        let standing = Orientation::new(Direction::Up, Direction::Back).unwrap();
        assert_eq!(standing.thickness_axis(), Axis::Across);
        assert_eq!(standing.world_axis(PartAxis::Length), Axis::Up);
        assert_eq!(standing.part_axis(Axis::Deep), PartAxis::Width);
        assert_eq!(standing.part_axis(Axis::Across), PartAxis::Thickness);

        let error = Orientation::new(Direction::Up, Direction::Down).unwrap_err();
        assert_eq!(
            error.to_string(),
            "length (up) and width (down) must run along different axes"
        );

        let flat = Orientation::default();
        assert_eq!(
            (flat.length(), flat.width()),
            (Direction::Right, Direction::Back)
        );
        assert_eq!(flat.thickness_axis(), Axis::Up);
    }

    #[test]
    fn a_round_part_needs_only_its_length() {
        for direction in Direction::ALL {
            let orientation = Orientation::round(direction);
            assert_eq!(orientation.length(), direction);
            assert_ne!(orientation.width().axis(), direction.axis());
            assert!(Orientation::new(orientation.length(), orientation.width()).is_ok());
        }
    }

    #[test]
    fn a_slot_is_inside_its_array() {
        let slot = ArraySlot::new(1, count(2)).unwrap();
        assert!(slot.is_last() && !slot.is_first());
        assert_eq!((slot.index(), slot.count().get()), (1, 2));
        assert!(ArraySlot::new(0, count(1)).unwrap().is_first());
        assert_eq!(ArraySlot::new(2, count(2)), None);
    }

    #[test]
    fn terms_fix_the_axis_of_their_anchor() {
        let against = Translation::Against {
            anchor: face(Face::Top),
            gap: Length::ZERO,
        };
        assert_eq!(against.axis(), Some(Axis::Up));
        let centred = Translation::Center {
            from: Anchor::PartMid {
                part: BASE,
                axis: Axis::Across,
            },
            to: Anchor::PartMid {
                part: BASE,
                axis: Axis::Across,
            },
        };
        assert_eq!(centred.axis(), Some(Axis::Across));
        let crooked = Translation::Center {
            from: face(Face::Left).into(),
            to: face(Face::Front).into(),
        };
        assert_eq!(crooked.axis(), None);
        let spaced = Translation::Spaced {
            spacing: Spacing::Ends {
                inset: Length::from_mm(100),
            },
            from: face(Face::Bottom).into(),
            to: face(Face::Top).into(),
            slot: ArraySlot::new(0, count(2)).unwrap(),
        };
        assert_eq!(spaced.axis(), Some(Axis::Up));
    }

    #[test]
    fn corners_are_two_flush_terms() {
        let inset = Length::from_mm(60);
        let (across, deep) = Corner::BackLeft.translations(BASE, inset);
        assert_eq!(
            across,
            Translation::Flush {
                anchor: face(Face::Left),
                inset
            }
        );
        assert_eq!(
            deep,
            Translation::Flush {
                anchor: face(Face::Back),
                inset
            }
        );
        for corner in Corner::ALL {
            let (across, deep) = corner.translations(BASE, inset);
            assert_eq!(across.axis(), Some(Axis::Across));
            assert_eq!(deep.axis(), Some(Axis::Deep));
        }
    }

    #[test]
    fn a_placement_has_one_term_per_axis() {
        let orientation = Orientation::default();
        let ground = Placement::Ground { orientation };
        assert_eq!(ground.translation(Axis::Up), None);
        let up = Translation::Against {
            anchor: face(Face::Top),
            gap: Length::ZERO,
        };
        let across = Translation::Flush {
            anchor: face(Face::Left),
            inset: Length::ZERO,
        };
        let deep = Translation::Flush {
            anchor: face(Face::Back),
            inset: Length::ZERO,
        };
        let placed = Placement::Related {
            orientation,
            up,
            across,
            deep,
        };
        assert_eq!(placed.orientation(), orientation);
        for axis in Axis::ALL {
            assert_eq!(
                placed.translation(axis).and_then(Translation::axis),
                Some(axis)
            );
        }
    }

    #[test]
    fn hardware_terms_stay_off_the_mount_face_axis() {
        let term = |axis, side| AxisTerm {
            axis,
            translation: Translation::Flush {
                anchor: face(side),
                inset: Length::ZERO,
            },
        };
        let up = term(Axis::Up, Face::Bottom);
        let across = term(Axis::Across, Face::Left);
        let deep = term(Axis::Deep, Face::Front);

        let pull =
            HardwarePlacement::new(BASE, Face::Front, HardwareTerms::Two(across, up)).unwrap();
        assert_eq!((pull.piece(), pull.face()), (BASE, Face::Front));
        assert_eq!(pull.translation(Axis::Up), Some(up.translation));
        assert_eq!(pull.translation(Axis::Across), Some(across.translation));
        assert_eq!(pull.translation(Axis::Deep), None);

        let slide = HardwarePlacement::new(BASE, Face::Left, HardwareTerms::Fixed).unwrap();
        assert_eq!(slide.terms(), HardwareTerms::Fixed);
        assert_eq!(slide.translation(Axis::Up), None);

        assert!(HardwarePlacement::new(BASE, Face::Back, HardwareTerms::One(up)).is_ok());
        assert_eq!(
            HardwarePlacement::new(BASE, Face::Back, HardwareTerms::One(deep)),
            Err(HardwarePlacementError::AxisFixedByMountFace(Axis::Deep))
        );
        let error =
            HardwarePlacement::new(BASE, Face::Front, HardwareTerms::Two(up, up)).unwrap_err();
        assert_eq!(error, HardwarePlacementError::AxisSetTwice(Axis::Up));
        assert_eq!(error.to_string(), "position along 'up' is set twice");
    }

    #[test]
    fn motions_name_the_assembly_that_moves() {
        let door = AssemblyId(3);
        let turn = Motion::Turn {
            moving: door,
            pivot: Pivot {
                piece: BASE,
                outer: Face::Front,
                edge: Face::Left,
            },
            opening_angle: Angle::from_deg(110),
        };
        assert_eq!(turn.moving(), door);
        let travel = Motion::Travel {
            moving: AssemblyId(4),
            toward: Direction::Front,
            travel: Length::from_mm(500),
        };
        assert_eq!(travel.moving(), AssemblyId(4));
    }
}
