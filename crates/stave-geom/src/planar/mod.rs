//! The general planar mechanism solver (grammar, "How a mechanism is
//! solved"): how the members of one piece of hardware move when its drive
//! runs through its range, from the mechanism its kind declares.
//!
//! # What goes in
//!
//! A [`Declaration`] is a kind's `mechanism { }` with every value worked
//! out, exact, in the kind's own frame with the hardware closed:
//!
//! - **Members** ([`Declaration::member`]), each with a [`MemberName`] and
//!   a [`Holding`]: [`Holding::Held`] for a member `with:` a role whose
//!   part stays put (a cup hinge's `plate` and `arm`, a slide's cabinet
//!   member); [`Holding::Free`] for a member the mechanism places (a
//!   member with no `with:`, and the driven member, even when it is `with:`
//!   the moving part, as a cup hinge's `cup` is `with: piece`);
//!   [`Holding::Carried`] for a member `with:` another member, which moves
//!   with it as one rigid piece.
//! - **Points** are [`PlanarPoint`]s: the `across` and `depth` of a
//!   `point(...)` (the mechanism is planar, every pin parallel to `along`).
//!   A positive turn carries `across` toward `depth`.
//! - **Constraints** ([`Constraint`]): `pivot a, b at point(...)` is
//!   [`Constraint::Pivot`]; `slot a, b from point(...) to point(...)` is
//!   [`Constraint::Slot`], the pin on `a` sitting at `from` when closed;
//!   `rail a, b along <word>` is [`Constraint::Rail`] with the word's
//!   [`PlanarDirection`]; `couple a to b: <ratio>` is
//!   [`Constraint::Couple`] with a [`Ratio`].
//! - **The drive** ([`Drive`]): `turn(m, from:, to:)` or
//!   `travel(m, from:, to:)`. The declared (closed) pose is the drive at
//!   `from`; at a value `v` the driven member has turned (or travelled) by
//!   `v - from`. A member's **travel** is how far it has moved along its
//!   rail from its closed place, measured against the held members.
//!
//! # What comes out
//!
//! [`Mechanism::new`] checks the declaration without numbers and fixes the
//! order of steps: exactly one freedom (three per free member, less two per
//! pivot or rail, less one per slot or couple, less one for the drive
//! leaves none), an order in which every member is placed one closed-form
//! step at a time, every entry used, and for each step with two answers
//! the branch the closed pose is on, worked out exactly in integers. Its
//! errors are [`MechanismErrorKind::Mechanism`]: `error[mechanism]` at the
//! kind.
//!
//! [`Path::new`] then runs the drive through its whole range in `f64`
//! nanometres, every [`SAMPLE_STEP_MICRODEG`] micro-degrees or
//! [`SAMPLE_STEP_NM`] nanometres, keeping each step on the closed pose's
//! branch, and refuses a step that comes within [`DEAD_POINT_MARGIN`] of a
//! dead point, a branch that would have to flip between samples, and a pin
//! that runs off the end of its slot. Between each two samples it proves,
//! from bounds on how fast each step's margin can change, that no step
//! comes within [`DEAD_POINT_MARGIN`] of a dead point anywhere in between
//! (the argument is in `proof.rs`); an interval it can't prove is halved,
//! down to [`PROOF_STEP_MICRODEG`] or [`PROOF_STEP_NM`], and then refused
//! as [`MechanismError::NearDeadPoint`]. Its errors are
//! [`MechanismErrorKind::Motion`]: `error[motion]` at the hardware, naming
//! the drive value. A checked [`Path`] gives a [`Pose`] (every member's
//! [`PlanarPose`]) at any drive value in range, exactly the declared pose
//! at `from`, and a [`Speed`] bound per member for sweeps.
//!
//! # The steps
//!
//! No Newton, no iteration: each step is closed form.
//!
//! - **Turn** and **travel**: the drive gives the driven member's angle,
//!   or its travel; a `couple` passes a travel on, times its ratio.
//! - **Hang**: a member whose angle is known, pinned to a placed member.
//! - **Rail**: a member whose travel is known, on a rail along a placed
//!   member that does not turn.
//! - **Two links** (a dyad): a point on one or two unplaced members lies on
//!   two paths, each a circle (the member is a link pinned to a placed
//!   member) or a straight line (a rail along, or a slot in, a placed
//!   member): circle and circle, or circle and line.
//! - **Hung on two** (the cup hinge's step): a member whose angle is known
//!   hangs on two links, or a link and a slot or rail; its offset lies on a
//!   circle and a circle, or a circle and a line.
//! - **Slot in a link**: a link pinned to a placed member, its slot over a
//!   placed pin.

mod path;
mod plan;
mod proof;
#[cfg(test)]
mod tests;

use std::fmt;

use stave_lang::units::{Angle, Length, Rational};

pub use path::{Path, PlanarPose, Pose, Proof, Speed};
pub use plan::Mechanism;

/// How close any step may come to a dead point over the drive's range:
/// for two circles, the distance between their centres stays this much
/// inside the sum of their radii and outside their difference; for a
/// circle and a line, the line stays this much inside the circle (0.1mm).
pub const DEAD_POINT_MARGIN: Length = Length::from_nm(100_000);

/// A turning drive is checked at least every 0.05deg.
pub const SAMPLE_STEP_MICRODEG: i64 = 50_000;

/// A travelling drive is checked at least every 0.05mm.
pub const SAMPLE_STEP_NM: i64 = 50_000;

/// Between two samples of a turning drive the proof halves an interval it
/// can't prove down to this (0.00005deg) before it gives up.
pub const PROOF_STEP_MICRODEG: i64 = 50;

/// Between two samples of a travelling drive the proof halves an interval
/// it can't prove down to this (50 nanometres) before it gives up.
pub const PROOF_STEP_NM: i64 = 50;

/// A point in the mechanism's plane, in the kind's frame with the hardware
/// closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct PlanarPoint {
    /// Across the mount face, from the edge the hardware is measured from
    /// toward the middle of the piece.
    pub across: Length,
    /// From the mount face into the piece.
    pub depth: Length,
}

impl PlanarPoint {
    /// A point from its two coordinates.
    pub const fn new(across: Length, depth: Length) -> PlanarPoint {
        PlanarPoint { across, depth }
    }
}

/// A direction in the mechanism's plane: never zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PlanarDirection {
    across: Length,
    depth: Length,
}

impl PlanarDirection {
    /// Along `across`.
    pub const ACROSS: PlanarDirection = PlanarDirection {
        across: Length::from_nm(1),
        depth: Length::ZERO,
    };
    /// Along `depth`.
    pub const DEPTH: PlanarDirection = PlanarDirection {
        across: Length::ZERO,
        depth: Length::from_nm(1),
    };

    /// The direction from the origin toward `(across, depth)`; `None` when
    /// both are zero.
    pub fn new(across: Length, depth: Length) -> Option<PlanarDirection> {
        (across != Length::ZERO || depth != Length::ZERO)
            .then_some(PlanarDirection { across, depth })
    }

    /// Its `across` part (any scale).
    pub fn across(self) -> Length {
        self.across
    }

    /// Its `depth` part (any scale).
    pub fn depth(self) -> Length {
        self.depth
    }
}

/// A member of a [`Declaration`]: handed out by [`Declaration::member`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MemberId(u32);

impl MemberId {
    /// Its place in the order the members were declared (from zero).
    pub fn index(self) -> usize {
        // A u32 always fits a usize on the platforms Stave builds for;
        // saturate rather than cast.
        usize::try_from(self.0).unwrap_or(usize::MAX)
    }
}

/// A member's name, for messages: never empty.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MemberName(String);

impl MemberName {
    /// A name; `None` when it is empty.
    pub fn new(name: impl Into<String>) -> Option<MemberName> {
        let name = name.into();
        (!name.is_empty()).then_some(MemberName(name))
    }

    /// The name as written.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for MemberName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// How a member is held.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Holding {
    /// It stays put: `with:` a role whose part does not move.
    Held,
    /// The mechanism places it.
    Free,
    /// It rides on another member, as one rigid piece with it.
    Carried(MemberId),
}

/// A share of another member's travel: an exact ratio, never zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Ratio(Rational);

impl Ratio {
    /// A ratio; `None` when it is zero.
    pub fn new(value: Rational) -> Option<Ratio> {
        (value != Rational::ZERO).then_some(Ratio(value))
    }

    /// The exact value.
    pub fn value(self) -> Rational {
        self.0
    }
}

/// One entry of a mechanism, between two members.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Constraint {
    /// `first` and `second` turn on one pin at `at`.
    Pivot {
        /// One member.
        first: MemberId,
        /// The other.
        second: MemberId,
        /// Where the pin is, closed.
        at: PlanarPoint,
    },
    /// A pin on `pin` runs in a straight slot in `guide`, from `from` to
    /// `to`; closed, the pin is at `from`.
    Slot {
        /// The member that carries the pin.
        pin: MemberId,
        /// The member the slot is cut in.
        guide: MemberId,
        /// The slot's end where the pin sits closed.
        from: PlanarPoint,
        /// The slot's other end.
        to: PlanarPoint,
    },
    /// `runner` runs straight along `guide` without turning.
    Rail {
        /// The member that runs.
        runner: MemberId,
        /// The member it runs along.
        guide: MemberId,
        /// Which way, in `guide`'s frame (either sense).
        direction: PlanarDirection,
    },
    /// `follower`'s travel is `ratio` times `leader`'s.
    Couple {
        /// The member that follows.
        follower: MemberId,
        /// The member it follows.
        leader: MemberId,
        /// The share.
        ratio: Ratio,
    },
}

/// The one input, and its range.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Drive {
    /// `member` turns from `from` (closed) to `to`.
    Turn {
        /// The driven member.
        member: MemberId,
        /// The closed pose's angle.
        from: Angle,
        /// The far end of the range.
        to: Angle,
    },
    /// `member` travels along its rail from `from` (closed) to `to`.
    Travel {
        /// The driven member.
        member: MemberId,
        /// The closed pose's travel.
        from: Length,
        /// The far end of the range.
        to: Length,
    },
}

impl Drive {
    /// The driven member.
    pub fn member(self) -> MemberId {
        match self {
            Drive::Turn { member, .. } | Drive::Travel { member, .. } => member,
        }
    }

    /// The closed pose's value.
    pub fn from(self) -> DriveValue {
        match self {
            Drive::Turn { from, .. } => DriveValue::Turn(from),
            Drive::Travel { from, .. } => DriveValue::Travel(from),
        }
    }

    /// The far end of the range.
    pub fn to(self) -> DriveValue {
        match self {
            Drive::Turn { to, .. } => DriveValue::Turn(to),
            Drive::Travel { to, .. } => DriveValue::Travel(to),
        }
    }
}

/// A value of a drive, exact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DriveValue {
    /// An angle of a turning drive.
    Turn(Angle),
    /// A travel of a travelling drive.
    Travel(Length),
}

impl fmt::Display for DriveValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DriveValue::Turn(angle) => angle.fmt(f),
            DriveValue::Travel(length) => length.fmt(f),
        }
    }
}

/// A value of a drive at the geometry boundary: radians for a turning
/// drive, nanometres for a travelling one, absolute (not from `from`).
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct DriveAmount(f64);

impl DriveAmount {
    /// An angle of a turning drive, in radians.
    pub const fn radians(radians: f64) -> DriveAmount {
        DriveAmount(radians)
    }

    /// A travel of a travelling drive, in nanometres.
    pub const fn nanometres(nanometres: f64) -> DriveAmount {
        DriveAmount(nanometres)
    }

    /// An exact drive value.
    pub fn of(value: DriveValue) -> DriveAmount {
        match value {
            DriveValue::Turn(angle) => DriveAmount(crate::render::radians_f64(angle)),
            DriveValue::Travel(length) => DriveAmount(crate::render::nm(length)),
        }
    }

    /// Radians or nanometres.
    pub fn value(self) -> f64 {
        self.0
    }
}

/// The members of one mechanism and its entries, exact, as a kind declares
/// them. See the module docs for how a kind's text maps onto it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Declaration {
    members: Vec<(MemberName, Holding)>,
    constraints: Vec<Constraint>,
}

impl Declaration {
    /// An empty declaration.
    pub fn new() -> Declaration {
        Declaration::default()
    }

    /// Declares a member. A [`Holding::Carried`] must name a member
    /// declared before it.
    pub fn member(
        &mut self,
        name: MemberName,
        holding: Holding,
    ) -> Result<MemberId, MechanismError> {
        if let Holding::Carried(carrier) = holding
            && carrier.index() >= self.members.len()
        {
            return Err(MechanismError::UnknownMember { member: carrier });
        }
        let id = MemberId(u32::try_from(self.members.len()).map_err(|_| MechanismError::Overflow)?);
        self.members.push((name, holding));
        Ok(id)
    }

    /// Adds an entry.
    pub fn constrain(&mut self, constraint: Constraint) {
        self.constraints.push(constraint);
    }

    /// The members, in the order declared.
    pub fn members(&self) -> &[(MemberName, Holding)] {
        &self.members
    }

    /// The entries, in the order added.
    pub fn constraints(&self) -> &[Constraint] {
        &self.constraints
    }
}

/// Which diagnostic a [`MechanismError`] becomes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MechanismErrorKind {
    /// The declaration can't be solved: `error[mechanism]` at the kind.
    Mechanism,
    /// The mechanism doesn't move through its range: `error[motion]` at
    /// the hardware.
    Motion,
}

/// Why a mechanism can't be built or doesn't move through its range.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MechanismError {
    /// A member id from another declaration.
    #[error("member #{} is not declared", member.index())]
    UnknownMember {
        /// The id.
        member: MemberId,
    },
    /// A member rides on itself, through other members.
    #[error("`{member}` rides on itself")]
    CarriedInCircle {
        /// A member of the circle.
        member: MemberName,
    },
    /// An entry joins two members that are one rigid piece.
    #[error("`{first}` and `{second}` are one piece, so nothing can move between them")]
    OnePiece {
        /// One member.
        first: MemberName,
        /// The other.
        second: MemberName,
    },
    /// An entry joins two members that are both held.
    #[error("`{first}` and `{second}` are both held, so nothing can move between them")]
    BothHeld {
        /// One member.
        first: MemberName,
        /// The other.
        second: MemberName,
    },
    /// A slot whose two ends are one point.
    #[error("the slot in `{guide}` for `{pin}`'s pin has no length")]
    SlotHasNoLength {
        /// The member with the pin.
        pin: MemberName,
        /// The member with the slot.
        guide: MemberName,
    },
    /// A link whose two pins are one point.
    #[error("`{member}`'s pins at the two ends of a link are in one place")]
    LinkHasNoLength {
        /// The link.
        member: MemberName,
    },
    /// The members don't add up to one freedom.
    #[error(
        "the mechanism has {found} freedoms; it needs exactly 1 (3 for each moving member, less 2 for each pivot or rail and 1 for each slot or couple)"
    )]
    Freedom {
        /// How many it has.
        found: i64,
    },
    /// The drive turns or moves a held member.
    #[error("the drive moves `{member}`, which is held")]
    DriveHeld {
        /// The driven member.
        member: MemberName,
    },
    /// A drive whose range is one value.
    #[error("the drive's range starts and ends at {at}")]
    EmptyRange {
        /// The value.
        at: DriveValue,
    },
    /// A travelling drive or a couple on a member with no rail.
    #[error("`{member}` travels, but runs on no rail")]
    TravelWithoutRail {
        /// The member.
        member: MemberName,
    },
    /// No order of closed-form steps places every member.
    #[error(
        "this mechanism can't be solved one step at a time from its drive; left over: {}",
        names(left)
    )]
    NoOrder {
        /// The members no step could place.
        left: Vec<MemberName>,
    },
    /// An entry left over once every member is placed: it would hold the
    /// mechanism still.
    #[error(
        "the entry between `{first}` and `{second}` is one too many: every member is placed without it"
    )]
    Overconstrained {
        /// One member of the entry.
        first: MemberName,
        /// The other.
        second: MemberName,
    },
    /// A step reaches (or comes within [`DEAD_POINT_MARGIN`] of) a dead
    /// point, where the members lock or could flip to their other branch.
    #[error("{} lock at {at}", names(members))]
    DeadPoint {
        /// The members of the step.
        members: Vec<MemberName>,
        /// The first drive value where it happens.
        at: DriveValue,
    },
    /// Between two samples that both clear [`DEAD_POINT_MARGIN`], a step
    /// can't be shown to stay that far from a dead point, even with the
    /// interval halved down to [`PROOF_STEP_MICRODEG`] or
    /// [`PROOF_STEP_NM`].
    #[error(
        "{} may come within {} of locking near {at}",
        names(members),
        DEAD_POINT_MARGIN
    )]
    NearDeadPoint {
        /// The members of the step.
        members: Vec<MemberName>,
        /// The sample nearest the dead point where the proof stopped.
        at: DriveValue,
    },
    /// Between two samples a step jumped to its other branch.
    #[error("{} flip over at {at}", names(members))]
    BranchFlip {
        /// The members of the step.
        members: Vec<MemberName>,
        /// The drive value where it was seen.
        at: DriveValue,
    },
    /// A pin runs off an end of its slot.
    #[error("`{pin}`'s pin reaches the end of its slot in `{guide}` at {at}")]
    SlotEnd {
        /// The member with the pin.
        pin: MemberName,
        /// The member with the slot.
        guide: MemberName,
        /// The first drive value where it happens.
        at: DriveValue,
    },
    /// A pose asked for outside the drive's range.
    #[error("the drive goes from {from} to {to}; a pose outside that was asked for")]
    OutOfRange {
        /// The range's start.
        from: DriveValue,
        /// Its end.
        to: DriveValue,
    },
    /// A number too large for the arithmetic (never a design's).
    #[error("the mechanism is too large to solve")]
    Overflow,
    /// A step read a member not placed yet (never for a mechanism built by
    /// [`Mechanism::new`]).
    #[error("the mechanism's steps are out of order")]
    Internal,
}

impl MechanismError {
    /// Which diagnostic it becomes.
    pub fn kind(&self) -> MechanismErrorKind {
        match self {
            MechanismError::UnknownMember { .. }
            | MechanismError::CarriedInCircle { .. }
            | MechanismError::OnePiece { .. }
            | MechanismError::BothHeld { .. }
            | MechanismError::SlotHasNoLength { .. }
            | MechanismError::LinkHasNoLength { .. }
            | MechanismError::Freedom { .. }
            | MechanismError::DriveHeld { .. }
            | MechanismError::EmptyRange { .. }
            | MechanismError::TravelWithoutRail { .. }
            | MechanismError::NoOrder { .. }
            | MechanismError::Overconstrained { .. }
            | MechanismError::Overflow
            | MechanismError::Internal => MechanismErrorKind::Mechanism,
            MechanismError::DeadPoint { .. }
            | MechanismError::NearDeadPoint { .. }
            | MechanismError::BranchFlip { .. }
            | MechanismError::SlotEnd { .. }
            | MechanismError::OutOfRange { .. } => MechanismErrorKind::Motion,
        }
    }
}

/// Names in backticks, joined with commas and "and".
fn names(members: &[MemberName]) -> String {
    let quoted: Vec<String> = members.iter().map(|name| format!("`{name}`")).collect();
    match quoted.split_last() {
        None => String::new(),
        Some((last, [])) => last.clone(),
        Some((last, rest)) => format!("{} and {last}", rest.join(", ")),
    }
}
