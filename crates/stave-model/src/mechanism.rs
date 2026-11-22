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
//! exactly one freedom, and the drive moves a free member through a range
//! that isn't one value. Which order of closed-form steps places the
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
use crate::solved::Point;

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
    drive: MechanismDrive,
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
        if found != 1 {
            return Err(MechanismError::Freedom { found });
        }
        let driven = drive.member();
        if members.get(driven.index()).map(|member| member.holding) != Some(Holding::Free) {
            return Err(MechanismError::DriveNotFree {
                member: name(driven)?,
            });
        }
        if drive.from() == drive.to() {
            return Err(MechanismError::EmptyRange);
        }
        Ok(Mechanism {
            frame,
            members,
            entries,
            drive,
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

    /// Its drive and range.
    pub fn drive(&self) -> &MechanismDrive {
        &self.drive
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
        self.drive.member()
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
            drive: self.drive,
            stop: self.stop.clone(),
        }
    }

    /// The same mechanism with a narrower range: `to` moved to `to`. `None`
    /// when `to` is the other kind of value or would leave an empty range.
    pub fn with_to(&self, to: DriveValue) -> Option<Mechanism> {
        let drive = match (self.drive, to) {
            (MechanismDrive::Turn { member, from, .. }, DriveValue::Turn(to)) => {
                MechanismDrive::Turn { member, from, to }
            }
            (MechanismDrive::Travel { member, from, .. }, DriveValue::Travel(to)) => {
                MechanismDrive::Travel { member, from, to }
            }
            (MechanismDrive::Turn { .. }, DriveValue::Travel(_))
            | (MechanismDrive::Travel { .. }, DriveValue::Turn(_)) => return None,
        };
        (drive.from() != drive.to()).then(|| Mechanism {
            drive,
            ..self.clone()
        })
    }

    /// The exact shape of its motion: a turn about one fixed pin, a travel
    /// along one rail, or neither.
    pub fn shape(&self) -> MotionShape {
        let moving = self.moving();
        let held = |index: MemberIndex| {
            self.members
                .get(self.piece_of(index).index())
                .is_some_and(|member| member.holding == Holding::Held)
        };
        let joins_held = |[a, b]: [MemberIndex; 2]| {
            let (pa, pb) = (self.piece_of(a), self.piece_of(b));
            (pa == moving && held(b)) || (pb == moving && held(a))
        };
        let frame = self.frame;
        for entry in &self.entries {
            match (*entry, self.drive) {
                (
                    MechanismEntry::Pivot { first, second, at },
                    MechanismDrive::Turn { from, to, .. },
                ) if joins_held([first, second]) => {
                    let through = stepped(
                        stepped(frame.origin(), frame.across(), at.across),
                        frame.depth(),
                        at.depth,
                    );
                    let opening = to.microdeg().saturating_sub(from.microdeg());
                    let swing_toward = if opening >= 0 {
                        frame.depth()
                    } else {
                        frame.depth().opposite()
                    };
                    return MotionShape::Turn {
                        line: TurnLine {
                            axis: frame.axis(),
                            through,
                            free_side: frame.across(),
                            swing_toward,
                        },
                        opening: Angle::from_microdeg(opening.saturating_abs()),
                    };
                }
                (
                    MechanismEntry::Rail {
                        runner,
                        guide,
                        direction,
                    },
                    MechanismDrive::Travel { from, to, .. },
                ) if joins_held([runner, guide]) => {
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
                    let runner_moves = self.piece_of(runner) == moving;
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
        MotionShape::General
    }

    /// For a turning drive: the frame's first direction (from a hinge's
    /// edge toward the middle of the door) and the way it turns as the
    /// drive opens; `None` for a travelling drive.
    pub fn swing(&self) -> Option<(Direction, Direction)> {
        match self.drive {
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
