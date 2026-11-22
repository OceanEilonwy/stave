//! An assembly's solved mechanism (grammar, "Mechanisms" and "How a
//! mechanism is solved"): the members of the hardware that moves it, how
//! each is held, its pivots, slots, rails and couples, its one drive and
//! what stops it, exact, in one plane of the world.
//!
//! A [`Mechanism`] is what the kinds declare with every value worked out
//! for the design: the points are in the mechanism's own [`MechanismFrame`]
//! (the frame of the first piece of hardware with a drive, closed), in
//! whole nanometres. [`Mechanism::new`] makes a mechanism that can't be
//! counted wrong unrepresentable: every member it names is declared, a
//! carried member rides on one declared before it, the entries join two
//! different rigid pieces that aren't both held, the members add up to
//! exactly one freedom for each drive, and every drive moves the same free
//! member through a range that isn't one value. Which order of closed-form steps places the
//! members, and where they are at a drive value, are worked out at the
//! geometry boundary (`stave-geom`), which needs square roots.
//!
//! The pins are parallel to the frame's axis and the members move in the
//! plane across it. A positive turn carries the plane's first direction
//! ([`MechanismFrame::across`]) toward its second
//! ([`MechanismFrame::depth`]).

use serde::Serialize;
use stave_lang::placement::{Axis, Direction};
use stave_lang::units::{Angle, Length, Rational};

use crate::defs::{BodyId, RoleIndex};
use crate::ids::HardwareId;
use crate::nonempty::NonEmpty;
use crate::solved::Point;
use crate::solved::PoseValue;

/// The plane a mechanism moves in: across a world axis (the pins' axis),
/// with an origin and the world directions of its two coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub struct MechanismFrame {
    #[serde(serialize_with = "serialize_axis")]
    axis: Axis,
    origin: Point,
    #[serde(serialize_with = "serialize_direction")]
    across: Direction,
    #[serde(serialize_with = "serialize_direction")]
    depth: Direction,
}

impl MechanismFrame {
    /// A frame; `None` unless the axis and the two directions lie along
    /// three different world axes.
    pub fn new(
        axis: Axis,
        origin: Point,
        across: Direction,
        depth: Direction,
    ) -> Option<MechanismFrame> {
        let (a, b) = (across.axis(), depth.axis());
        (a != axis && b != axis && a != b).then_some(MechanismFrame {
            axis,
            origin,
            across,
            depth,
        })
    }

    /// The world axis the pins run along.
    pub fn axis(&self) -> Axis {
        self.axis
    }

    /// The world point of the frame's origin; its coordinate along the
    /// axis carries no meaning.
    pub fn origin(&self) -> Point {
        self.origin
    }

    /// The world direction of the plane's first coordinate.
    pub fn across(&self) -> Direction {
        self.across
    }

    /// The world direction of the plane's second coordinate.
    pub fn depth(&self) -> Direction {
        self.depth
    }

    /// The frame reflected in the world plane across `axis` at `plane`: its
    /// origin's coordinate on that axis becomes `2 * plane - origin`, and a
    /// direction along it turns round. Points in the frame stay as they
    /// are.
    pub fn reflected(&self, axis: Axis, origin: Length) -> MechanismFrame {
        let flip = |direction: Direction| {
            if direction.axis() == axis {
                direction.opposite()
            } else {
                direction
            }
        };
        MechanismFrame {
            axis: self.axis,
            origin: self.origin.with(axis, origin),
            across: flip(self.across),
            depth: flip(self.depth),
        }
    }
}

/// A point in a mechanism's plane, relative to its frame's origin, with the
/// mechanism closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize)]
pub struct FramePoint {
    /// Along [`MechanismFrame::across`].
    pub across: Length,
    /// Along [`MechanismFrame::depth`].
    pub depth: Length,
}

impl FramePoint {
    /// A point from its two coordinates.
    pub const fn new(across: Length, depth: Length) -> FramePoint {
        FramePoint { across, depth }
    }
}

/// A direction in a mechanism's plane: never zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub struct FrameDirection {
    across: Length,
    depth: Length,
}

impl FrameDirection {
    /// The direction toward `(across, depth)`; `None` when both are zero.
    pub fn new(across: Length, depth: Length) -> Option<FrameDirection> {
        (across != Length::ZERO || depth != Length::ZERO)
            .then_some(FrameDirection { across, depth })
    }

    /// Its part along [`MechanismFrame::across`] (any scale).
    pub fn across(&self) -> Length {
        self.across
    }

    /// Its part along [`MechanismFrame::depth`] (any scale).
    pub fn depth(&self) -> Length {
        self.depth
    }
}

/// A member of an assembly's mechanism, by its place in
/// [`Mechanism::members`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
pub struct MemberIndex(u32);

impl MemberIndex {
    /// The member at `index`; `None` past `u32`.
    pub fn new(index: usize) -> Option<MemberIndex> {
        u32::try_from(index).ok().map(MemberIndex)
    }

    /// Its place in the list (from zero).
    pub fn index(self) -> usize {
        // A u32 always fits a usize on the platforms Stave builds for;
        // saturate rather than cast.
        usize::try_from(self.0).unwrap_or(usize::MAX)
    }
}

/// How a member of a mechanism is held.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "member")]
pub enum Holding {
    /// It stays put: `with:` a role whose part doesn't move with the
    /// assembly.
    Held,
    /// The mechanism places it: a member with no `with:`, and the driven
    /// member.
    Free,
    /// It is one rigid piece with an earlier member: `with:` that member,
    /// or `with:` a role whose part moves with the assembly (then it rides
    /// on the driven member).
    Carried(MemberIndex),
}

/// What a member of a mechanism is: one of its kind's members with a
/// body, or the part of a role named in an entry (grammar, "Mechanisms",
/// a role in an entry), which the design places and which has no shape of
/// its own here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(untagged)]
pub enum MemberBody {
    /// One of the kind's members with a body.
    Body(BodyId),
    /// The part a role of the kind binds.
    Role {
        /// Which role.
        role: RoleIndex,
    },
}

impl MemberBody {
    /// The kind's member, when it is one.
    pub fn body(self) -> Option<BodyId> {
        match self {
            MemberBody::Body(body) => Some(body),
            MemberBody::Role { .. } => None,
        }
    }
}

/// A member of a mechanism: which member of which piece of hardware, and
/// how it is held.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MechanismMember {
    /// The piece of hardware it belongs to (for several alike, the first).
    pub hardware: HardwareId,
    /// Which of its kind's members, or which role's part.
    pub body: MemberBody,
    /// The member's name in its kind, for messages: `cup`, `inner_link`;
    /// a role's name for a role's part (`piece`).
    pub name: String,
    /// How it is held.
    pub holding: Holding,
}

/// One entry of a mechanism, between two members.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum MechanismEntry {
    /// `first` and `second` turn on one pin at `at`.
    Pivot {
        /// One member.
        first: MemberIndex,
        /// The other.
        second: MemberIndex,
        /// The pin, closed.
        at: FramePoint,
    },
    /// A pin on `pin` runs in a straight slot in `guide`, from `from`
    /// (where it is closed) to `to`.
    Slot {
        /// The member with the pin.
        pin: MemberIndex,
        /// The member the slot is cut in.
        guide: MemberIndex,
        /// The end where the pin sits closed.
        from: FramePoint,
        /// The other end.
        to: FramePoint,
    },
    /// `runner` runs straight along `guide` without turning.
    Rail {
        /// The member that runs.
        runner: MemberIndex,
        /// The member it runs along.
        guide: MemberIndex,
        /// Which way (either sense).
        direction: FrameDirection,
    },
    /// `follower` travels `ratio` times `leader`'s travel.
    Couple {
        /// The member that follows.
        follower: MemberIndex,
        /// The member it follows.
        leader: MemberIndex,
        /// The share, never zero.
        ratio: Rational,
    },
}

impl MechanismEntry {
    /// Its two members.
    pub fn members(&self) -> [MemberIndex; 2] {
        match *self {
            MechanismEntry::Pivot { first, second, .. } => [first, second],
            MechanismEntry::Slot { pin, guide, .. } => [pin, guide],
            MechanismEntry::Rail { runner, guide, .. } => [runner, guide],
            MechanismEntry::Couple {
                follower, leader, ..
            } => [follower, leader],
        }
    }

    /// The freedoms it takes away: two for a pivot or a rail, one for a
    /// slot or a couple.
    pub fn takes(&self) -> i64 {
        match self {
            MechanismEntry::Pivot { .. } | MechanismEntry::Rail { .. } => 2,
            MechanismEntry::Slot { .. } | MechanismEntry::Couple { .. } => 1,
        }
    }
}

/// A value of a drive, exact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "value")]
pub enum DriveValue {
    /// An angle of a turning drive.
    Turn(Angle),
    /// A travel of a travelling drive.
    Travel(Length),
}

/// The one input of a mechanism, and its range.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum MechanismDrive {
    /// `member` turns from `from` (closed) to `to`.
    Turn {
        /// The driven member.
        member: MemberIndex,
        /// The closed pose's angle.
        from: Angle,
        /// The far end of the range.
        to: Angle,
    },
    /// `member` travels along its rail from `from` (closed) to `to`.
    Travel {
        /// The driven member.
        member: MemberIndex,
        /// The closed pose's travel.
        from: Length,
        /// The far end of the range.
        to: Length,
    },
}

impl MechanismDrive {
    /// The driven member.
    pub fn member(&self) -> MemberIndex {
        match *self {
            MechanismDrive::Turn { member, .. } | MechanismDrive::Travel { member, .. } => member,
        }
    }

    /// The closed pose's value.
    pub fn from(&self) -> DriveValue {
        match *self {
            MechanismDrive::Turn { from, .. } => DriveValue::Turn(from),
            MechanismDrive::Travel { from, .. } => DriveValue::Travel(from),
        }
    }

    /// The far end of the range.
    pub fn to(&self) -> DriveValue {
        match *self {
            MechanismDrive::Turn { to, .. } => DriveValue::Turn(to),
            MechanismDrive::Travel { to, .. } => DriveValue::Travel(to),
        }
    }

    /// `true` for a turning drive.
    pub fn turns(&self) -> bool {
        match self {
            MechanismDrive::Turn { .. } => true,
            MechanismDrive::Travel { .. } => false,
        }
    }

    /// How far the range reaches from `from`, as an amount a beginner reads
    /// (a door opens 110deg, a drawer comes out 450mm): the angle or the
    /// length between its ends, never negative.
    pub fn extent(&self) -> DriveValue {
        match *self {
            MechanismDrive::Turn { from, to, .. } => DriveValue::Turn(Angle::from_microdeg(
                to.microdeg()
                    .saturating_sub(from.microdeg())
                    .saturating_abs(),
            )),
            MechanismDrive::Travel { from, to, .. } => DriveValue::Travel(Length::from_nm(
                to.nm().saturating_sub(from.nm()).saturating_abs(),
            )),
        }
    }

    /// The drive value `amount` along the range from `from` toward `to`
    /// (an angle for a turn, a length for a travel); `None` when the
    /// amount is the other kind of value.
    pub fn value_at(&self, amount: DriveValue) -> Option<DriveValue> {
        let toward = |from: i64, to: i64, by: i64| {
            if to >= from {
                from.saturating_add(by)
            } else {
                from.saturating_sub(by)
            }
        };
        match (*self, amount) {
            (MechanismDrive::Turn { from, to, .. }, DriveValue::Turn(by)) => {
                Some(DriveValue::Turn(Angle::from_microdeg(toward(
                    from.microdeg(),
                    to.microdeg(),
                    by.microdeg(),
                ))))
            }
            (MechanismDrive::Travel { from, to, .. }, DriveValue::Travel(by)) => Some(
                DriveValue::Travel(Length::from_nm(toward(from.nm(), to.nm(), by.nm()))),
            ),
            (MechanismDrive::Turn { .. }, DriveValue::Travel(_))
            | (MechanismDrive::Travel { .. }, DriveValue::Turn(_)) => None,
        }
    }
}

/// A drive of a mechanism, by its place in [`Mechanism::drives`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
pub struct DriveIndex(u32);

impl DriveIndex {
    /// The first drive: the only one of most mechanisms.
    pub const FIRST: DriveIndex = DriveIndex(0);

    /// The drive at `index`; `None` past `u32`.
    pub fn new(index: usize) -> Option<DriveIndex> {
        u32::try_from(index).ok().map(DriveIndex)
    }

    /// Its place in the list (from zero).
    pub fn index(self) -> usize {
        // A u32 always fits a usize on the platforms Stave builds for;
        // saturate rather than cast.
        usize::try_from(self.0).unwrap_or(usize::MAX)
    }
}

impl MechanismDrive {
    /// How far a value of this drive lies from its `from`, toward its
    /// `to` (negative behind `from`), in micro-degrees or nanometres;
    /// `None` for a value of the other kind.
    pub fn signed_amount(&self, value: DriveValue) -> Option<i64> {
        let toward = |from: i64, to: i64, at: i64| {
            if to >= from {
                at.saturating_sub(from)
            } else {
                from.saturating_sub(at)
            }
        };
        match (*self, value) {
            (MechanismDrive::Turn { from, to, .. }, DriveValue::Turn(at)) => {
                Some(toward(from.microdeg(), to.microdeg(), at.microdeg()))
            }
            (MechanismDrive::Travel { from, to, .. }, DriveValue::Travel(at)) => {
                Some(toward(from.nm(), to.nm(), at.nm()))
            }
            (MechanismDrive::Turn { .. }, DriveValue::Travel(_))
            | (MechanismDrive::Travel { .. }, DriveValue::Turn(_)) => None,
        }
    }

    /// Whether a value lies in its range, from `from` to `to`, ends
    /// included.
    pub fn reaches(&self, value: DriveValue) -> bool {
        let extent = match self.extent() {
            DriveValue::Turn(angle) => angle.microdeg(),
            DriveValue::Travel(length) => length.nm(),
        };
        self.signed_amount(value)
            .is_some_and(|amount| (0..=extent).contains(&amount))
    }

    /// A value as the amount a pose names: how far from `from` toward
    /// `to` (an angle for a turn, a length for a travel); `None` for a
    /// value of the other kind.
    pub fn amount_of(&self, value: DriveValue) -> Option<DriveValue> {
        let amount = self.signed_amount(value)?;
        Some(match self {
            MechanismDrive::Turn { .. } => DriveValue::Turn(Angle::from_microdeg(amount)),
            MechanismDrive::Travel { .. } => DriveValue::Travel(Length::from_nm(amount)),
        })
    }
}

/// The values a drive can be locked at (grammar, "Mechanisms", set
/// positions): drive values of its own kind, each once, in order from
/// its `from` toward its `to`, its `from` always among them.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct SetPositions {
    values: Vec<DriveValue>,
    every: Option<DriveValue>,
}

impl SetPositions {
    /// The set positions of `drive`: `values` in any order, repeats
    /// allowed, with its `from` added, and the step of `every(d)` when
    /// they were written so. `None` when a value or the step is the
    /// other kind of value.
    pub fn new(
        drive: &MechanismDrive,
        values: Vec<DriveValue>,
        every: Option<DriveValue>,
    ) -> Option<SetPositions> {
        let same = |value: &DriveValue| drive.signed_amount(*value).is_some();
        if !values.iter().all(same) || !every.iter().all(same) {
            return None;
        }
        let mut values = values;
        values.push(drive.from());
        values.sort_by_key(|value| drive.signed_amount(*value));
        values.dedup();
        Some(SetPositions { values, every })
    }

    /// Every set value, from `from` toward `to`: never empty.
    pub fn values(&self) -> &[DriveValue] {
        &self.values
    }

    /// The step of `every(d)`, when they were written so; `None` for a
    /// list.
    pub fn every(&self) -> Option<DriveValue> {
        self.every
    }
}

/// A drive with its name in its kind, when it has one (grammar,
/// "Mechanisms", drives): `tilt` in `drive tilt: turn(...)`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct NamedDrive {
    /// The name; `None` for the one unnamed drive of a mechanism.
    pub name: Option<String>,
    /// The drive and its range.
    pub drive: MechanismDrive,
    /// The values it can be locked at; `None` for a continuous drive,
    /// set anywhere in its range.
    pub set_at: Option<SetPositions>,
}

impl NamedDrive {
    /// An unnamed, continuous drive.
    pub fn unnamed(drive: MechanismDrive) -> NamedDrive {
        NamedDrive {
            name: None,
            drive,
            set_at: None,
        }
    }

    /// Its set values within its range, from `from` toward `to`; empty
    /// for a continuous drive.
    pub fn set_values(&self) -> impl Iterator<Item = DriveValue> + '_ {
        self.set_at
            .iter()
            .flat_map(|set| set.values().iter().copied())
            .filter(|value| self.drive.reaches(*value))
    }

    /// Whether it can be locked at a value: any value in its range of a
    /// continuous drive, one of its set values otherwise.
    pub fn sets_at(&self, value: DriveValue) -> bool {
        match &self.set_at {
            None => self.drive.reaches(value),
            Some(_) => self.set_values().any(|set| set == value),
        }
    }

    /// Where `open` puts it: the far end of its range, or for a drive
    /// with set positions the furthest of them within its range.
    pub fn open(&self) -> DriveValue {
        match &self.set_at {
            None => self.drive.to(),
            Some(_) => self.set_values().last().unwrap_or(self.drive.from()),
        }
    }

    /// The set values nearest a value, one either side of it (toward
    /// `from`, toward `to`), when there are; both `None` for a
    /// continuous drive.
    pub fn nearest_set(&self, value: DriveValue) -> (Option<DriveValue>, Option<DriveValue>) {
        let Some(at) = self.drive.signed_amount(value) else {
            return (None, None);
        };
        let amount = |set: &DriveValue| self.drive.signed_amount(*set).unwrap_or(0);
        let below = self.set_values().filter(|set| amount(set) < at).last();
        let above = self.set_values().find(|set| amount(set) > at);
        (below, above)
    }
}

/// What stops a mechanism's drive, with the kind's words for it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MechanismStop {
    /// Where the drive stops, as far from `from` as the drive reaches.
    pub at: DriveValue,
    /// What stops it, for a beginner.
    pub message: String,
}

/// Why a [`Mechanism`] can't be made.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MechanismError {
    /// A member index past the members, or a carried member that rides on
    /// itself or on a later member.
    #[error("member #{} is not declared before it is used", index.index())]
    UnknownMember {
        /// The index.
        index: MemberIndex,
    },
    /// No members.
    #[error("a mechanism has no members")]
    NoMembers,
    /// An entry between two pieces of one rigid member.
    #[error("`{first}` and `{second}` are one piece, so nothing can move between them")]
    OnePiece {
        /// One member.
        first: String,
        /// The other.
        second: String,
    },
    /// An entry between two held members.
    #[error("`{first}` and `{second}` are both held, so nothing can move between them")]
    BothHeld {
        /// One member.
        first: String,
        /// The other.
        second: String,
    },
    /// A couple whose ratio is zero.
    #[error("`{follower}` is coupled to a share of nothing")]
    ZeroRatio {
        /// The member that follows.
        follower: String,
    },
    /// The members don't add up to exactly one freedom.
    #[error("the mechanism has {found} freedoms; it needs exactly 1")]
    Freedom {
        /// How many it has.
        found: i64,
    },
    /// The members of a mechanism with several drives don't add up to one
    /// freedom for each.
    #[error("the mechanism has {found} freedoms; it needs exactly {drives}, one for each drive")]
    DriveFreedom {
        /// How many it has.
        found: i64,
        /// How many drives it has.
        drives: usize,
    },
    /// No drives.
    #[error("a mechanism with a motion has at least one drive")]
    NoDrives,
    /// Two drives move different members, or one member twice the same
    /// way.
    #[error("every drive moves one member, once turning and once travelling")]
    DrivesApart,
    /// The drive moves a member that isn't free.
    #[error("the drive moves `{member}`, which the mechanism doesn't place")]
    DriveNotFree {
        /// The driven member.
        member: String,
    },
    /// A drive whose range is one value.
    #[error("the drive's range starts and ends at the same value")]
    EmptyRange,
    /// A number too large for the arithmetic (never a design's).
    #[error("the mechanism is too large to count")]
    Overflow,
}

/// An assembly's mechanism, exact and counted. See the module docs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Mechanism {
    frame: MechanismFrame,
    members: Vec<MechanismMember>,
    entries: Vec<MechanismEntry>,
    /// Every one moves the same member.
    drives: NonEmpty<NamedDrive>,
    stop: Option<MechanismStop>,
    hardware: Vec<HardwareId>,
}

impl Mechanism {
    /// A mechanism: checks that every member named is declared (a carried
    /// one riding on a member declared before it), that each entry joins
    /// two rigid pieces that are different and not both held, that the
    /// free members, three freedoms each, less two for each pivot or rail
    /// and one for each slot or couple, leave exactly one, and that the
    /// drive moves a free member through more than one value. `hardware`
    /// lists the pieces of hardware it moves, drivers first.
    pub fn new(
        frame: MechanismFrame,
        members: Vec<MechanismMember>,
        entries: Vec<MechanismEntry>,
        drive: MechanismDrive,
        stop: Option<MechanismStop>,
        hardware: Vec<HardwareId>,
    ) -> Result<Mechanism, MechanismError> {
        Mechanism::with_drives(
            frame,
            members,
            entries,
            vec![NamedDrive::unnamed(drive)],
            stop,
            hardware,
        )
    }

    /// A mechanism with any number of drives (grammar, "Mechanisms",
    /// drives), checked as [`Mechanism::new`] checks one: the members
    /// leave exactly one freedom for each drive, and every drive moves the
    /// same free member, at most once turning and once travelling, through
    /// a range that isn't one value.
    pub fn with_drives(
        frame: MechanismFrame,
        members: Vec<MechanismMember>,
        entries: Vec<MechanismEntry>,
        drives: Vec<NamedDrive>,
        stop: Option<MechanismStop>,
        hardware: Vec<HardwareId>,
    ) -> Result<Mechanism, MechanismError> {
        if members.is_empty() {
            return Err(MechanismError::NoMembers);
        }
        // Each member's rigid piece: the held or free member it rides on.
        let mut root = Vec::with_capacity(members.len());
        for (index, member) in members.iter().enumerate() {
            let piece = match member.holding {
                Holding::Held | Holding::Free => index,
                Holding::Carried(carrier) => {
                    if carrier.index() >= index {
                        return Err(MechanismError::UnknownMember { index: carrier });
                    }
                    root.get(carrier.index())
                        .copied()
                        .ok_or(MechanismError::UnknownMember { index: carrier })?
                }
            };
            root.push(piece);
        }
        let piece_of = |index: MemberIndex| {
            root.get(index.index())
                .copied()
                .ok_or(MechanismError::UnknownMember { index })
        };
        let name = |index: MemberIndex| {
            members
                .get(index.index())
                .map(|member| member.name.clone())
                .ok_or(MechanismError::UnknownMember { index })
        };
        let held = |piece: usize| {
            members
                .get(piece)
                .is_some_and(|member| member.holding == Holding::Held)
        };
        let mut taken = 0i64;
        for entry in &entries {
            let [first, second] = entry.members();
            let (a, b) = (piece_of(first)?, piece_of(second)?);
            if a == b {
                return Err(MechanismError::OnePiece {
                    first: name(first)?,
                    second: name(second)?,
                });
            }
            if held(a) && held(b) {
                return Err(MechanismError::BothHeld {
                    first: name(first)?,
                    second: name(second)?,
                });
            }
            if let MechanismEntry::Couple {
                follower, ratio, ..
            } = entry
                && *ratio == Rational::ZERO
            {
                return Err(MechanismError::ZeroRatio {
                    follower: name(*follower)?,
                });
            }
            taken = taken
                .checked_add(entry.takes())
                .ok_or(MechanismError::Overflow)?;
        }
        let free = members
            .iter()
            .filter(|member| member.holding == Holding::Free)
            .count();
        let found = i64::try_from(free)
            .ok()
            .and_then(|free| free.checked_mul(3))
            .and_then(|all| all.checked_sub(taken))
            .ok_or(MechanismError::Overflow)?;
        let Some(drives) = NonEmpty::new(drives) else {
            return Err(MechanismError::NoDrives);
        };
        let first = drives.first();
        let wanted = i64::try_from(drives.len()).map_err(|_| MechanismError::Overflow)?;
        if found != wanted {
            return Err(if drives.len() == 1 {
                MechanismError::Freedom { found }
            } else {
                MechanismError::DriveFreedom {
                    found,
                    drives: drives.len(),
                }
            });
        }
        let driven = first.drive.member();
        if members.get(driven.index()).map(|member| member.holding) != Some(Holding::Free) {
            return Err(MechanismError::DriveNotFree {
                member: name(driven)?,
            });
        }
        let turns = drives.iter().filter(|named| named.drive.turns()).count();
        if drives.iter().any(|named| named.drive.member() != driven)
            || turns > 1
            || drives.len() - turns > 1
        {
            return Err(MechanismError::DrivesApart);
        }
        if drives
            .iter()
            .any(|named| named.drive.from() == named.drive.to())
        {
            return Err(MechanismError::EmptyRange);
        }
        Ok(Mechanism {
            frame,
            members,
            entries,
            drives,
            stop,
            hardware,
        })
    }

    /// The plane it moves in.
    pub fn frame(&self) -> &MechanismFrame {
        &self.frame
    }

    /// Its members, in order: never empty.
    pub fn members(&self) -> &[MechanismMember] {
        &self.members
    }

    /// Its entries, in order.
    pub fn entries(&self) -> &[MechanismEntry] {
        &self.entries
    }

    /// Its first drive and range: the only one of most mechanisms (see
    /// [`Mechanism::drives`]).
    pub fn drive(&self) -> &MechanismDrive {
        &self.drives.first().drive
    }

    /// Every drive, in the kind's order: never empty, each moving the
    /// driven member ([`Mechanism::moving`]).
    pub fn drives(&self) -> &NonEmpty<NamedDrive> {
        &self.drives
    }

    /// A drive by its place.
    pub fn drive_at(&self, drive: DriveIndex) -> Option<&NamedDrive> {
        self.drives.get(drive.index())
    }

    /// The drive with a name.
    pub fn drive_named(&self, name: &str) -> Option<DriveIndex> {
        self.drives
            .iter()
            .position(|named| named.name.as_deref() == Some(name))
            .and_then(DriveIndex::new)
    }

    /// Every drive at its `from`: the closed pose.
    pub fn closed(&self) -> Vec<DriveValue> {
        self.drives.iter().map(|named| named.drive.from()).collect()
    }

    /// The exact value a scene's pose value means for one drive: its
    /// `from` for `closed`, its `to` for `open` (the furthest set value,
    /// for a drive with set positions), and an amount measured
    /// from its `from` toward its `to` otherwise; a value of the other
    /// kind is its `from`. `None` for a drive it doesn't have.
    pub fn value_of(&self, drive: DriveIndex, pose: PoseValue) -> Option<DriveValue> {
        let named = self.drives.get(drive.index())?;
        let drive = named.drive;
        Some(match pose {
            PoseValue::Closed => drive.from(),
            PoseValue::Open => named.open(),
            PoseValue::Angle(angle) => drive
                .value_at(DriveValue::Turn(angle))
                .unwrap_or(drive.from()),
            PoseValue::Travel(length) => drive
                .value_at(DriveValue::Travel(length))
                .unwrap_or(drive.from()),
        })
    }

    /// The exact values of every drive for a pose value of each (in
    /// order; `closed` for any left out), as [`Mechanism::value_of`].
    pub fn values_of(&self, poses: &[PoseValue]) -> Vec<DriveValue> {
        self.drives
            .iter()
            .enumerate()
            .map(|(index, named)| {
                DriveIndex::new(index)
                    .and_then(|drive| {
                        self.value_of(
                            drive,
                            poses.get(index).copied().unwrap_or(PoseValue::Closed),
                        )
                    })
                    .unwrap_or(named.drive.from())
            })
            .collect()
    }

    /// What stops it, when a kind says.
    pub fn stop(&self) -> Option<&MechanismStop> {
        self.stop.as_ref()
    }

    /// The pieces of hardware it moves: those with a drive first, in the
    /// order of the design, then those that follow.
    pub fn hardware(&self) -> &[HardwareId] {
        &self.hardware
    }

    /// The member the assembly moves with: the driven one.
    pub fn moving(&self) -> MemberIndex {
        self.drive().member()
    }

    /// The rigid piece a member is part of: the held or free member it
    /// rides on (itself when it rides on nothing).
    pub fn piece_of(&self, member: MemberIndex) -> MemberIndex {
        let mut at = member;
        // Carried members ride on earlier ones (checked by `new`), so this
        // ends within the list.
        for _ in 0..self.members.len() {
            match self.members.get(at.index()).map(|m| m.holding) {
                Some(Holding::Carried(carrier)) => at = carrier,
                Some(Holding::Held | Holding::Free) | None => break,
            }
        }
        at
    }

    /// The same mechanism with its frame reflected (see
    /// [`MechanismFrame::reflected`]) and its members and hardware
    /// re-bound by `rebind`.
    pub fn reflected(
        &self,
        axis: Axis,
        origin: Length,
        rebind: impl Fn(HardwareId) -> HardwareId,
    ) -> Mechanism {
        Mechanism {
            frame: self.frame.reflected(axis, origin),
            members: self
                .members
                .iter()
                .map(|member| MechanismMember {
                    hardware: rebind(member.hardware),
                    body: member.body,
                    name: member.name.clone(),
                    holding: member.holding,
                })
                .collect(),
            hardware: self.hardware.iter().map(|h| rebind(*h)).collect(),
            entries: self.entries.clone(),
            drives: self.drives.clone(),
            stop: self.stop.clone(),
        }
    }

    /// The same mechanism with its first drive's range narrower: `to`
    /// moved to `to`. `None` when `to` is the other kind of value or would
    /// leave an empty range.
    pub fn with_to(&self, to: DriveValue) -> Option<Mechanism> {
        let drive = match (*self.drive(), to) {
            (MechanismDrive::Turn { member, from, .. }, DriveValue::Turn(to)) => {
                MechanismDrive::Turn { member, from, to }
            }
            (MechanismDrive::Travel { member, from, .. }, DriveValue::Travel(to)) => {
                MechanismDrive::Travel { member, from, to }
            }
            (MechanismDrive::Turn { .. }, DriveValue::Travel(_))
            | (MechanismDrive::Travel { .. }, DriveValue::Turn(_)) => return None,
        };
        if drive.from() == drive.to() {
            return None;
        }
        let mut drives = self.drives.clone();
        drives.first_mut().drive = drive;
        Some(Mechanism {
            drives,
            ..self.clone()
        })
    }

    /// The exact shape of its first drive's motion, every other drive at
    /// its `from`: a turn about one fixed pin, a travel along one rail, or
    /// neither.
    pub fn shape(&self) -> MotionShape {
        self.shape_of(DriveIndex::FIRST, &self.closed())
    }

    /// `true` when `entry` joins the moving piece to a held one.
    fn joins_held(&self, [a, b]: [MemberIndex; 2]) -> bool {
        let moving = self.moving();
        let held = |index: MemberIndex| {
            self.members
                .get(self.piece_of(index).index())
                .is_some_and(|member| member.holding == Holding::Held)
        };
        let (pa, pb) = (self.piece_of(a), self.piece_of(b));
        (pa == moving && held(b)) || (pb == moving && held(a))
    }

    /// The slot the driven member's pin runs in, in a held member, with
    /// the frame direction it runs along (a slot a travel drives runs
    /// along one); `None` when there is none.
    fn held_slot(&self) -> Option<(FramePoint, Direction)> {
        let moving = self.moving();
        self.entries.iter().find_map(|entry| match *entry {
            MechanismEntry::Slot {
                pin,
                guide,
                from,
                to,
            } if self.piece_of(pin) == moving && self.joins_held([pin, guide]) => {
                let along = match (
                    to.across.nm().saturating_sub(from.across.nm()),
                    to.depth.nm().saturating_sub(from.depth.nm()),
                ) {
                    (a, 0) if a != 0 => self.frame.across(),
                    (0, d) if d != 0 => self.frame.depth(),
                    _ => return None,
                };
                Some((from, along))
            }
            MechanismEntry::Slot { .. }
            | MechanismEntry::Pivot { .. }
            | MechanismEntry::Rail { .. }
            | MechanismEntry::Couple { .. } => None,
        })
    }

    /// Where the driven member has been moved, as one world offset, when
    /// the drives are at `values` (one for each drive, absolute): `Some`
    /// only when that is a translation (every turning drive at its `from`,
    /// every travelling one an exact travel along a held rail or slot),
    /// `None` when it turns the member or isn't exact. A value of the other
    /// kind counts as its drive's `from`.
    pub fn offset_at(&self, values: &[DriveValue]) -> Option<Point> {
        let mut offset = Point::default();
        for (index, named) in self.drives.iter().enumerate() {
            let value = values.get(index).copied().unwrap_or(named.drive.from());
            match (named.drive, value) {
                (MechanismDrive::Turn { from, .. }, DriveValue::Turn(angle)) => {
                    // A whole turn brings the member back to where it began
                    // (grammar, "Mechanisms").
                    let turned = angle.microdeg().saturating_sub(from.microdeg());
                    if turned.rem_euclid(Angle::from_deg(360).microdeg()) != 0 {
                        return None;
                    }
                }
                (MechanismDrive::Travel { from, .. }, DriveValue::Travel(length)) => {
                    if length == from {
                        continue;
                    }
                    let toward = self.travel_direction(DriveIndex::new(index)?)?;
                    let by = Length::from_nm(length.nm().saturating_sub(from.nm()));
                    offset = stepped(offset, toward, by);
                }
                (MechanismDrive::Turn { .. }, DriveValue::Travel(_))
                | (MechanismDrive::Travel { .. }, DriveValue::Turn(_)) => {}
            }
        }
        Some(offset)
    }

    /// The world direction a positive travel of a travelling drive moves
    /// the driven member: along its rail in a held member, or along its
    /// slot in one (grammar, "Mechanisms", a travel along a slot); `None`
    /// for a turn, or a travel along no such rail or slot.
    fn travel_direction(&self, drive: DriveIndex) -> Option<Direction> {
        let named = self.drives.get(drive.index())?;
        if named.drive.turns() {
            return None;
        }
        let moving = self.moving();
        for entry in &self.entries {
            if let MechanismEntry::Rail {
                runner,
                guide,
                direction,
            } = *entry
                && self.joins_held([runner, guide])
            {
                let along = match (direction.across().nm(), direction.depth().nm()) {
                    (a, 0) if a > 0 => self.frame.across(),
                    (a, 0) if a < 0 => self.frame.across().opposite(),
                    (0, d) if d > 0 => self.frame.depth(),
                    (0, d) if d < 0 => self.frame.depth().opposite(),
                    _ => return None,
                };
                return Some(if self.piece_of(runner) == moving {
                    along
                } else {
                    along.opposite()
                });
            }
        }
        self.held_slot().map(|(_, along)| along)
    }

    /// The exact shape of one drive's motion with every other drive held
    /// at `held` (one value for each drive, absolute; the swept drive's
    /// own is ignored): a turn about one fixed pin, a travel along one
    /// fixed direction, or neither. The shape is where the motion is in
    /// the world: a turn's line is through the pin where the held drives
    /// have put it. A travel is exact only while the held turns are at
    /// their `from`; the moving bodies are then where
    /// [`Mechanism::offset_at`] puts them.
    pub fn shape_of(&self, drive: DriveIndex, held: &[DriveValue]) -> MotionShape {
        let Some(swept) = self.drives.get(drive.index()).map(|named| named.drive) else {
            return MotionShape::General;
        };
        // The other drives, as they are held, with the swept one at its
        // `from`.
        let others: Vec<DriveValue> = self
            .drives
            .iter()
            .enumerate()
            .map(|(index, named)| {
                if index == drive.index() {
                    named.drive.from()
                } else {
                    held.get(index).copied().unwrap_or(named.drive.from())
                }
            })
            .collect();
        let Some(offset) = self.offset_at(&others) else {
            return MotionShape::General;
        };
        let moved = |point: Point| {
            let mut point = point;
            for axis in Axis::ALL {
                point = point.with(
                    axis,
                    Length::from_nm(point.get(axis).nm().saturating_add(offset.get(axis).nm())),
                );
            }
            point
        };
        let frame = self.frame;
        let world = |at: FramePoint| {
            stepped(
                stepped(frame.origin(), frame.across(), at.across),
                frame.depth(),
                at.depth,
            )
        };
        let turn = |at: FramePoint, from: Angle, to: Angle| {
            let opening = to.microdeg().saturating_sub(from.microdeg());
            let swing_toward = if opening >= 0 {
                frame.depth()
            } else {
                frame.depth().opposite()
            };
            MotionShape::Turn {
                line: TurnLine {
                    axis: frame.axis(),
                    through: moved(world(at)),
                    free_side: frame.across(),
                    swing_toward,
                },
                opening: Angle::from_microdeg(opening.saturating_abs()),
            }
        };
        for entry in &self.entries {
            match (*entry, swept) {
                (
                    MechanismEntry::Pivot { first, second, at },
                    MechanismDrive::Turn { from, to, .. },
                ) if self.joins_held([first, second]) => {
                    return turn(at, from, to);
                }
                (
                    MechanismEntry::Rail {
                        runner,
                        guide,
                        direction,
                    },
                    MechanismDrive::Travel { from, to, .. },
                ) if self.joins_held([runner, guide]) => {
                    let along = match (direction.across().nm(), direction.depth().nm()) {
                        (a, 0) if a > 0 => frame.across(),
                        (a, 0) if a < 0 => frame.across().opposite(),
                        (0, d) if d > 0 => frame.depth(),
                        (0, d) if d < 0 => frame.depth().opposite(),
                        _ => return MotionShape::General,
                    };
                    let travel = to.nm().saturating_sub(from.nm());
                    // The runner's travel is measured along the rail's
                    // direction in the guide; the guide is held, so that is
                    // the world's.
                    let runner_moves = self.piece_of(runner) == self.moving();
                    let toward = if (travel >= 0) == runner_moves {
                        along
                    } else {
                        along.opposite()
                    };
                    return MotionShape::Travel {
                        toward,
                        travel: Length::from_nm(travel.saturating_abs()),
                    };
                }
                _ => {}
            }
        }
        // A pin in a held slot, with a turn and a travel: the turn is about
        // the pin where the travel has put it, the travel straight along
        // the slot.
        if self.drives.len() > 1
            && let Some((from_point, along)) = self.held_slot()
        {
            return match swept {
                MechanismDrive::Turn { from, to, .. } => turn(from_point, from, to),
                MechanismDrive::Travel { from, to, .. } => {
                    let travel = to.nm().saturating_sub(from.nm());
                    MotionShape::Travel {
                        toward: if travel >= 0 { along } else { along.opposite() },
                        travel: Length::from_nm(travel.saturating_abs()),
                    }
                }
            };
        }
        MotionShape::General
    }

    /// For a turning drive: the frame's first direction (from a hinge's
    /// edge toward the middle of the door) and the way it turns as the
    /// drive opens; `None` for a travelling drive.
    pub fn swing(&self) -> Option<(Direction, Direction)> {
        match *self.drive() {
            MechanismDrive::Turn { from, to, .. } => {
                let toward = if to.microdeg() >= from.microdeg() {
                    self.frame.depth()
                } else {
                    self.frame.depth().opposite()
                };
                Some((self.frame.across(), toward))
            }
            MechanismDrive::Travel { .. } => None,
        }
    }
}

/// The line a member turns about when a mechanism's driven member is
/// pinned straight to a held one (a pin between two leaves).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct TurnLine {
    /// The world axis the line runs along.
    #[serde(serialize_with = "serialize_axis")]
    pub axis: Axis,
    /// A point on the line; its coordinate along `axis` is the mechanism
    /// frame's origin's, and carries no meaning.
    pub through: Point,
    /// The direction from the line toward the rest of the assembly: the
    /// frame's first direction.
    #[serde(serialize_with = "serialize_direction")]
    pub free_side: Direction,
    /// The direction the free side moves as the drive opens.
    #[serde(serialize_with = "serialize_direction")]
    pub swing_toward: Direction,
}

/// The exact shape of a mechanism's motion, where it has one: chosen by the
/// mechanism's entries, never by a kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum MotionShape {
    /// The driven member is pinned to a held one: the assembly turns about
    /// this line, as far as `opening`.
    Turn {
        /// The line.
        line: TurnLine,
        /// How far it turns.
        opening: Angle,
    },
    /// The driven member runs on a rail along a held one: the assembly
    /// travels this way, as far as `travel`.
    Travel {
        /// The world direction it moves as it opens.
        #[serde(serialize_with = "serialize_direction")]
        toward: Direction,
        /// How far it moves.
        travel: Length,
    },
    /// Anything else: the members' poses come from the planar solver.
    General,
}

/// `point` moved `amount` along a world direction, saturating.
fn stepped(point: Point, direction: Direction, amount: Length) -> Point {
    let signed = if direction.is_positive() {
        amount.nm()
    } else {
        amount.nm().saturating_neg()
    };
    let axis = direction.axis();
    point.with(
        axis,
        Length::from_nm(point.get(axis).nm().saturating_add(signed)),
    )
}

fn serialize_axis<S: serde::Serializer>(value: &Axis, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(value.word())
}

fn serialize_direction<S: serde::Serializer>(
    value: &Direction,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(value.word())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn member(name: &str, holding: Holding) -> MechanismMember {
        MechanismMember {
            hardware: HardwareId::from_usize(0),
            body: MemberBody::Body(BodyId::new(0).unwrap()),
            name: name.to_owned(),
            holding,
        }
    }

    fn index(i: usize) -> MemberIndex {
        MemberIndex::new(i).unwrap()
    }

    fn frame() -> MechanismFrame {
        MechanismFrame::new(
            Axis::Up,
            Point::default(),
            Direction::Right,
            Direction::Front,
        )
        .unwrap()
    }

    fn turn(member: usize) -> MechanismDrive {
        MechanismDrive::Turn {
            member: index(member),
            from: Angle::ZERO,
            to: Angle::from_deg(90),
        }
    }

    fn pivot(a: usize, b: usize) -> MechanismEntry {
        MechanismEntry::Pivot {
            first: index(a),
            second: index(b),
            at: FramePoint::default(),
        }
    }

    #[test]
    fn a_butt_hinge_is_one_pivot() {
        let mechanism = Mechanism::new(
            frame(),
            vec![
                member("leaf", Holding::Free),
                member("fixed", Holding::Held),
            ],
            vec![pivot(0, 1)],
            turn(0),
            None,
            vec![HardwareId::from_usize(0)],
        )
        .unwrap();
        assert_eq!(mechanism.moving(), index(0));
    }

    #[test]
    fn freedoms_are_counted_exactly() {
        let made = Mechanism::new(
            frame(),
            vec![
                member("leaf", Holding::Free),
                member("fixed", Holding::Held),
            ],
            vec![],
            turn(0),
            None,
            vec![],
        );
        assert_eq!(made, Err(MechanismError::Freedom { found: 3 }));
    }

    #[test]
    fn a_pin_inside_one_piece_is_refused() {
        let made = Mechanism::new(
            frame(),
            vec![
                member("leaf", Holding::Free),
                member("pin", Holding::Carried(index(0))),
                member("fixed", Holding::Held),
            ],
            vec![pivot(0, 1), pivot(0, 2)],
            turn(0),
            None,
            vec![],
        );
        assert!(matches!(made, Err(MechanismError::OnePiece { .. })));
    }

    #[test]
    fn a_carried_member_rides_on_an_earlier_one() {
        let made = Mechanism::new(
            frame(),
            vec![
                member("leaf", Holding::Carried(index(1))),
                member("fixed", Holding::Held),
            ],
            vec![],
            turn(0),
            None,
            vec![],
        );
        assert_eq!(made, Err(MechanismError::UnknownMember { index: index(1) }));
    }

    #[test]
    fn the_drive_moves_a_free_member() {
        let made = Mechanism::new(
            frame(),
            vec![
                member("leaf", Holding::Free),
                member("fixed", Holding::Held),
            ],
            vec![pivot(0, 1)],
            turn(1),
            None,
            vec![],
        );
        assert!(matches!(made, Err(MechanismError::DriveNotFree { .. })));
    }

    #[test]
    fn a_value_along_the_range_follows_its_sense() {
        let out = MechanismDrive::Travel {
            member: index(0),
            from: Length::ZERO,
            to: Length::from_mm(-450),
        };
        assert_eq!(
            out.value_at(DriveValue::Travel(Length::from_mm(200))),
            Some(DriveValue::Travel(Length::from_mm(-200)))
        );
        assert_eq!(out.extent(), DriveValue::Travel(Length::from_mm(450)));
    }

    #[test]
    fn a_reflected_frame_turns_its_directions_round() {
        let reflected = frame().reflected(Axis::Across, Length::from_mm(10));
        assert_eq!(reflected.across(), Direction::Left);
        assert_eq!(reflected.depth(), Direction::Front);
        assert_eq!(reflected.origin().across, Length::from_mm(10));
    }
}
