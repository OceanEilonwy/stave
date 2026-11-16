//! Typed placement vocabulary.
//!
//! `place { }` blocks are lowered into these types after name resolution.
//! The vocabulary has **no way to express a coordinate**: every position is a
//! relationship to a named anchor on something already placed, plus an
//! optional measured offset.
//!
//! Each axis is set by exactly one term and anchors only point at things
//! placed earlier, so positions are worked out directly in dependency order;
//! no solver is involved. Joints never position parts, except
//! [`Placement::Inside`].
//!
//! Motions (hinges, slides) come from hardware. Placement fixes the *rest*
//! pose (doors closed, drawers shut); scenes set motion values relative to it.

use crate::units::{Angle, Length};

/// Index of a resolved part (an array element is its own part).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PartId(pub u32);

/// Index of a resolved joint or hardware item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ConnectionId(pub u32);

/// Index of a resolved region (an opening, a bay).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RegionId(pub u32);

/// World axes, named from the viewer's point of view standing at the front.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Axis {
    /// Floor to ceiling.
    Up,
    /// Left to right.
    Across,
    /// Front to back.
    Deep,
}

/// A direction along an axis. Used by `orient`, where the sign matters for
/// parts that aren't symmetric end to end (a turned foot with its tenon at
/// one end).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Direction {
    Up,
    Down,
    Left,
    Right,
    Front,
    Back,
}

impl Direction {
    pub fn axis(self) -> Axis {
        match self {
            Direction::Up | Direction::Down => Axis::Up,
            Direction::Left | Direction::Right => Axis::Across,
            Direction::Front | Direction::Back => Axis::Deep,
        }
    }

    /// `true` for up, right and back.
    pub fn is_positive(self) -> bool {
        matches!(self, Direction::Up | Direction::Right | Direction::Back)
    }
}

/// Faces of a part or region, named by the world direction they face.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Face {
    Top,
    Bottom,
    Left,
    Right,
    Front,
    Back,
}

impl Face {
    /// Axis the face is perpendicular to.
    pub fn axis(self) -> Axis {
        match self {
            Face::Top | Face::Bottom => Axis::Up,
            Face::Left | Face::Right => Axis::Across,
            Face::Front | Face::Back => Axis::Deep,
        }
    }

    pub fn opposite(self) -> Face {
        match self {
            Face::Top => Face::Bottom,
            Face::Bottom => Face::Top,
            Face::Left => Face::Right,
            Face::Right => Face::Left,
            Face::Front => Face::Back,
            Face::Back => Face::Front,
        }
    }

    /// `true` if the face points in the positive direction of its axis
    /// (up, right, back).
    pub fn is_positive(self) -> bool {
        matches!(self, Face::Top | Face::Right | Face::Back)
    }
}

/// A plane that placement terms measure from. A joint housing is not an
/// anchor; it is only used by [`Placement::Inside`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Anchor {
    /// `base.top`
    PartFace { part: PartId, face: Face },
    /// `mid(base, across)`
    PartMid { part: PartId, axis: Axis },
    /// `front_opening.left`
    RegionFace { region: RegionId, face: Face },
    /// `mid(door, across)` where `door` is a region
    RegionMid { region: RegionId, axis: Axis },
}

/// How an array is distributed between two anchors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Spacing {
    /// Equal gaps everywhere, including at both ends.
    Evenly,
    /// First element flush with `from` and last flush with `to`, each moved
    /// inward by `inset`; equal gaps between.
    Ends { inset: Length },
    /// Fixed center-to-center distance, starting at `from`.
    Pitch(Length),
    /// Fixed gap between elements, starting at `from`.
    Gap(Length),
}

/// Sets the position along one axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Translation {
    /// `against(anchor)` (gap zero), `against(anchor, gap: d)`, `on(x.top)`:
    /// this part's opposite face is `gap` away from the anchor, on the anchor
    /// face's outward side.
    Against { anchor: Anchor, gap: Length },
    /// `flush(anchor)` (inset zero) or `flush(anchor, inset: d)`: this part's
    /// same-named face lines up with the anchor, moved inward by `inset`.
    Flush { anchor: Anchor, inset: Length },
    /// `center(part, axis)` or `center(between: a, b)`
    Center { a: Anchor, b: Anchor },
}

/// Sets rotation. Thickness takes the axis not named.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Orientation {
    /// Direction the length runs from its start (station 0 of a profile).
    pub length: Direction,
    pub width: Direction,
}

impl Default for Orientation {
    /// Lying flat: length runs left to right, width runs front to back.
    fn default() -> Self {
        Orientation { length: Direction::Right, width: Direction::Back }
    }
}

/// The fully lowered `place { }` of one part.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Placement {
    /// The root part. Exactly one per project.
    Ground,
    /// Seated in a joint's housing, e.g. a back panel in its rabbets. The
    /// housing sets position and rotation; adding `orient` is an error.
    Inside { connection: ConnectionId },
    /// Positioned by relationships. Each axis is set exactly once.
    Related {
        orientation: Orientation,
        up: Translation,
        across: Translation,
        deep: Translation,
    },
}

/// Placement of an array of parts or assemblies.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ArrayPlacement {
    /// One axis distributed by `spacing`; the other two axes are shared by
    /// every element.
    Spaced {
        axis: Axis,
        spacing: Spacing,
        from: Anchor,
        to: Anchor,
        orientation: Orientation,
        /// The two remaining axes, each set exactly once.
        rest: [(Axis, Translation); 2],
    },
    /// Exactly four elements, one per corner of a part's footprint, in the
    /// order front_left, front_right, back_left, back_right.
    Corners {
        of: PartId,
        inset: Length,
        /// Vertical relation shared by all four, e.g. `against(base.bottom)`.
        up: Translation,
        orientation: Orientation,
    },
}

// ---------------------------------------------------------------------------
// Motion (used by scenes)
// ---------------------------------------------------------------------------

/// Where a hinge pivots, taken from the hinge's catalog entry. Euro hinges
/// pivot away from the door edge, so the offsets matter for the
/// "door hits its neighbour" check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Pivot {
    /// Edge of the moving part the pivot is measured from, as the
    /// intersection of two of its faces.
    pub edge: (Face, Face),
    /// Distance from the first face, measured outward along its normal.
    pub out_from_first: Length,
    /// Distance from the second face, measured outward along its normal.
    pub out_from_second: Length,
}

/// A degree of freedom provided by hardware. The rest pose from placement is
/// the motion's zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Motion {
    /// Hinge: rotation about a pivot from the catalog.
    Rotate { moving: PartId, pivot: Pivot, min: Angle, max: Angle },
    /// Drawer slide or sliding door: translation along one direction.
    Slide { moving: PartId, opens_toward: Direction, min: Length, max: Length },
}

/// Value assigned to a motion in a scene.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MotionValue {
    Closed,
    Open,
    Angle(Angle),
    Travel(Length),
}
