//! Mechanisms (grammar, "Mechanisms" and "How a mechanism is solved"): a
//! hardware kind's `mechanism { }` evaluated exactly for one piece of
//! hardware ([`HardwareMechanism`]), and the one mechanism of an assembly
//! made from every piece of hardware on it (the rule for several items).
//!
//! Nothing here knows a kind, a base or a member by name. A piece of
//! hardware's mechanism is counted on its own as soon as it is placed (one
//! freedom for each drive, none for a mechanism that follows), and
//! reported once per kind. An assembly's mechanism is then the first
//! driving item's, every other driving item must move alike (the same
//! members, entries and drives, drive for drive, seen in the first one's
//! plane), and the followers' members and entries are added. The exact
//! [`stave_model::mechanism::Mechanism`] is what the solved design holds;
//! the order of steps and the poses are worked out in `stave-geom`.

use std::collections::BTreeMap;

use stave_lang::placement::{Axis, Direction, PerAxis};
use stave_lang::units::{ExactAngle, ExactLength, Length, Rational};
use stave_lang::{Code, Diagnostic, Label};
use stave_model::defs::{
    BodyId, Carrier, ConstraintDef, DriveDef, EntryPair, EntrySide, FrameAxis, KindDef,
    MechanismDef, RoleIndex, SetAtDef,
};
use stave_model::ids::{AssemblyId, HardwareId, KindId};
use stave_model::mechanism::{
    DriveIndex, DriveValue, FrameDirection, FramePoint, Holding, Mechanism, MechanismDrive,
    MechanismEntry, MechanismError, MechanismFrame, MechanismMember, MechanismStop, MemberBody,
    MemberIndex, NamedDrive, SetPositions,
};
use stave_model::solved::Point;
use stave_model::value::Value;

use crate::hardware::{Frame, ORIGIN, Placed, RoleBound};
use crate::joints::Binding;
use crate::{Failure, Fault, Solved, Solver};

/// A point in the kind's frame, exact.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ExactPoint {
    /// Along the piece's length.
    pub along: ExactLength,
    /// Across the mount face, toward the middle of the piece.
    pub across: ExactLength,
    /// From the mount face into the piece.
    pub depth: ExactLength,
}

impl ExactPoint {
    /// Its coordinate on a frame axis.
    pub fn on(&self, axis: FrameAxis) -> ExactLength {
        match axis {
            FrameAxis::Along => self.along,
            FrameAxis::Across => self.across,
            FrameAxis::Depth => self.depth,
        }
    }
}

/// A point or a vector in a mechanism's plane, exact: `[across, depth]`.
pub(crate) type PlanePoint = [ExactLength; 2];

/// The plane one piece of hardware's mechanism moves in, placed in the
/// world: the frame's origin, the world axis of its pins and the world
/// directions of the plane's two coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Plane {
    pub origin: PerAxis<ExactLength>,
    pub axis: Axis,
    pub across: Direction,
    pub depth: Direction,
}

impl Plane {
    /// The plane a piece of hardware's mechanism moves in, placed by its
    /// frame, with its pins along `pins`: its two coordinates are the
    /// other two frame directions, in the order a positive turn carries
    /// the first toward the second (grammar, "How a mechanism is solved").
    pub fn of_frame(frame: &Frame, pins: FrameAxis) -> Plane {
        let [first, second] = pins.plane();
        Plane {
            origin: frame.origin,
            axis: frame.direction(pins).axis(),
            across: frame.direction(first),
            depth: frame.direction(second),
        }
    }

    /// The plane reflected across the world plane at `plane` on `axis`.
    pub fn reflected(&self, axis: Axis, plane: ExactLength) -> Option<Plane> {
        let mut origin = self.origin;
        origin[axis] = plane.checked_add(plane)?.checked_sub(self.origin[axis])?;
        let flip = |direction: Direction| {
            if direction.axis() == axis {
                direction.opposite()
            } else {
                direction
            }
        };
        Some(Plane {
            origin,
            axis: self.axis,
            across: flip(self.across),
            depth: flip(self.depth),
        })
    }

    /// The world vector of a vector in the plane.
    fn world(&self, [across, depth]: PlanePoint) -> Option<PerAxis<ExactLength>> {
        let mut out = ORIGIN;
        for (direction, amount) in [(self.across, across), (self.depth, depth)] {
            let signed = if direction.is_positive() {
                amount
            } else {
                amount.checked_neg()?
            };
            let slot = &mut out[direction.axis()];
            *slot = slot.checked_add(signed)?;
        }
        Some(out)
    }

    /// A world vector's coordinates in the plane.
    fn coordinates(&self, vector: PerAxis<ExactLength>) -> Option<PlanePoint> {
        let along = |direction: Direction| -> Option<ExactLength> {
            let value = vector[direction.axis()];
            if direction.is_positive() {
                Some(value)
            } else {
                value.checked_neg()
            }
        };
        Some([along(self.across)?, along(self.depth)?])
    }

    /// A point of another plane (on the same axis) in this one's
    /// coordinates.
    fn point_of(&self, other: &Plane, point: PlanePoint) -> Option<PlanePoint> {
        let offset = other.world(point)?;
        let mut vector = PerAxis::try_from_fn(|axis| {
            other.origin[axis]
                .checked_add(offset[axis])
                .and_then(|from| from.checked_sub(self.origin[axis]))
                .ok_or(())
        })
        .ok()?;
        // Along the pins the coordinate carries no meaning.
        vector[self.axis] = ExactLength::ZERO;
        self.coordinates(vector)
    }

    /// A vector of another plane (on the same axis) in this one's
    /// coordinates.
    fn vector_of(&self, other: &Plane, vector: PlanePoint) -> Option<PlanePoint> {
        self.coordinates(other.world(vector)?)
    }

    /// `true` when a turn that is positive in `other` is positive here too.
    fn same_hand(&self, other: &Plane) -> Option<bool> {
        let one = Length::from_nm(1).to_exact();
        let [aa, ad] = self.vector_of(other, [one, ExactLength::ZERO])?;
        let [da, dd] = self.vector_of(other, [ExactLength::ZERO, one])?;
        let cross = aa
            .value()
            .checked_mul(dd.value())?
            .checked_sub(ad.value().checked_mul(da.value())?)?;
        Some(cross > Rational::ZERO)
    }

    /// The solved frame, its origin to the nearest nanometre.
    fn frame(&self) -> Option<MechanismFrame> {
        let origin = Point {
            across: self.origin.across.to_nearest_length()?,
            up: self.origin.up.to_nearest_length()?,
            deep: self.origin.deep.to_nearest_length()?,
        };
        MechanismFrame::new(self.axis, origin, self.across, self.depth)
    }
}

/// What a member of a kind's mechanism is `with:`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MemberWith {
    /// A role of the kind.
    Role(RoleIndex),
    /// Another member: it rides on it.
    Member(BodyId),
    /// Nothing: the mechanism places it.
    Placed,
}

/// A member of a kind's mechanism, for one piece of hardware.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct KindMember {
    pub body: BodyId,
    pub name: String,
    pub with: MemberWith,
}

/// An entry of a kind's mechanism, exact, in its plane: a member, then a
/// member or a role's part (a couple's are both members).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Entry {
    Pivot {
        bodies: EntryPair,
        at: PlanePoint,
    },
    Slot {
        bodies: EntryPair,
        from: PlanePoint,
        to: PlanePoint,
    },
    Rail {
        bodies: EntryPair,
        direction: PlanePoint,
    },
    Couple {
        bodies: EntryPair,
        ratio: Rational,
    },
}

impl Entry {
    fn bodies(&self) -> EntryPair {
        match self {
            Entry::Pivot { bodies, .. }
            | Entry::Slot { bodies, .. }
            | Entry::Rail { bodies, .. }
            | Entry::Couple { bodies, .. } => *bodies,
        }
    }
}

/// A kind's drive, exact.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Drive {
    Turn {
        body: BodyId,
        from: ExactAngle,
        to: ExactAngle,
    },
    Travel {
        body: BodyId,
        from: ExactLength,
        to: ExactLength,
    },
}

impl Drive {
    pub fn body(&self) -> BodyId {
        match self {
            Drive::Turn { body, .. } | Drive::Travel { body, .. } => *body,
        }
    }
}

/// A kind's drive with its name, when written, and its set positions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct KindDrive {
    pub name: Option<String>,
    pub drive: Drive,
    pub set_at: Option<KindSetAt>,
}

impl KindDrive {
    /// An unnamed, continuous drive.
    #[cfg(test)]
    pub fn unnamed(drive: Drive) -> KindDrive {
        KindDrive {
            name: None,
            drive,
            set_at: None,
        }
    }
}

/// A drive's set positions, exact, in micro-degrees for a turn or
/// nanometres for a travel (grammar, "Mechanisms", set positions): every
/// one within its range.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct KindSetAt {
    pub values: Vec<Rational>,
    /// The step of `every(d)`, more than nothing; `None` for a list.
    pub every: Option<Rational>,
}

/// The most set positions a drive may have.
const MOST_SET_POSITIONS: i128 = 1000;

/// A drive's `set_at:` as written, evaluated: in micro-degrees or
/// nanometres.
enum WrittenSetAt {
    List(Vec<Rational>, stave_lang::ast::Span),
    Every(Rational, stave_lang::ast::Span),
}

/// Where a kind's `stop` stops the drive, exact.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StopValue {
    Angle(ExactAngle),
    Length(ExactLength),
}

/// One piece of hardware's mechanism, exact, in its own plane.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct HardwareMechanism {
    pub kind: KindId,
    pub plane: Plane,
    /// Its members with a body, in the kind's order.
    pub members: Vec<KindMember>,
    /// The names of its kind's roles, in order: an entry may name one.
    pub roles: Vec<String>,
    pub entries: Vec<Entry>,
    /// Empty for a mechanism that follows its assembly's; each acts on a
    /// member the mechanism places.
    pub drives: Vec<KindDrive>,
    pub stop: Option<(StopValue, String)>,
}

impl HardwareMechanism {
    /// The member a body is, by its place in the members.
    fn member(&self, body: BodyId) -> Option<&KindMember> {
        self.members.iter().find(|member| member.body == body)
    }

    /// Reflected for a mirrored piece of hardware: the plane is; the points
    /// in it stay.
    pub fn reflected(&self, axis: Axis, plane: ExactLength) -> Option<HardwareMechanism> {
        Some(HardwareMechanism {
            plane: self.plane.reflected(axis, plane)?,
            ..self.clone()
        })
    }
}

/// The members, entries and drive of one piece of hardware seen in a
/// plane, with which member each body became.
struct Declared {
    members: Vec<MechanismMember>,
    entries: Vec<MechanismEntry>,
    /// Empty for a mechanism that follows.
    drives: Vec<NamedDrive>,
    /// The member the assembly moves with, for a mechanism with drives.
    moving: Option<MemberIndex>,
    stop: Option<MechanismStop>,
    bodies: BTreeMap<BodyId, MemberIndex>,
}

/// Why a piece of hardware's mechanism can't be declared.
enum DeclareError {
    /// A number too large, or a body that isn't a member (never a design's).
    Internal,
    /// A drive moves a member `with:` a part that stays put.
    DriveHeld(String),
    /// No member is `with:` a part that moves, so nothing carries the
    /// assembly.
    NothingMoves,
    /// A member rides on itself.
    InCircle(String),
}

/// How each member of a piece of hardware is held, given which roles move
/// with the assembly, its members declared after `offset` members (in an
/// order where every member a member rides on comes first), and its
/// entries and drive in `base`'s plane.
fn declare(
    hardware: HardwareId,
    mechanism: &HardwareMechanism,
    moving: &dyn Fn(RoleIndex) -> bool,
    base: &Plane,
    driven: Option<MemberIndex>,
    offset: usize,
) -> Result<Declared, DeclareError> {
    let mut bodies: BTreeMap<BodyId, MemberIndex> = BTreeMap::new();
    let mut members: Vec<MechanismMember> = Vec::new();
    let index = |count: usize| MemberIndex::new(offset + count).ok_or(DeclareError::Internal);
    let push = |members: &mut Vec<MechanismMember>,
                bodies: &mut BTreeMap<BodyId, MemberIndex>,
                member: &KindMember,
                holding: Holding|
     -> Result<MemberIndex, DeclareError> {
        let at = index(members.len())?;
        members.push(MechanismMember {
            hardware,
            body: MemberBody::Body(member.body),
            name: member.name.clone(),
            holding,
        });
        bodies.insert(member.body, at);
        Ok(at)
    };
    // A drive acts on any member the mechanism places, never on one with:
    // a part that stays put.
    for named in &mechanism.drives {
        let member = mechanism
            .member(named.drive.body())
            .ok_or(DeclareError::Internal)?;
        if let MemberWith::Role(role) = member.with
            && !moving(role)
        {
            return Err(DeclareError::DriveHeld(member.name.clone()));
        }
    }
    // The member the assembly moves with first: the one with: the part
    // that moves (of several, the first a drive acts on, else the first
    // written); every other member with: a part that moves rides on it.
    let carries =
        |member: &&KindMember| matches!(member.with, MemberWith::Role(role) if moving(role));
    let own_moving = if mechanism.drives.is_empty() {
        None
    } else {
        let driven = mechanism.drives.iter().find_map(|named| {
            mechanism
                .member(named.drive.body())
                .filter(|member| carries(member))
        });
        let first = driven
            .or_else(|| mechanism.members.iter().find(|member| carries(member)))
            .ok_or(DeclareError::NothingMoves)?;
        Some(first.body)
    };
    let driven = match own_moving {
        Some(body) => {
            let member = mechanism.member(body).ok_or(DeclareError::Internal)?;
            Some(push(&mut members, &mut bodies, member, Holding::Free)?)
        }
        None => driven,
    };
    // Then every other member once what it rides on is declared.
    let mut left: Vec<&KindMember> = mechanism
        .members
        .iter()
        .filter(|member| Some(member.body) != own_moving)
        .collect();
    while !left.is_empty() {
        let before = left.len();
        let mut next = Vec::new();
        for member in left {
            let holding = match member.with {
                MemberWith::Role(role) if moving(role) => match driven {
                    Some(driven) => Holding::Carried(driven),
                    // Nothing drives the assembly: it stays put.
                    None => Holding::Held,
                },
                MemberWith::Role(_) => Holding::Held,
                MemberWith::Placed => Holding::Free,
                MemberWith::Member(other) => match bodies.get(&other) {
                    Some(carrier) => Holding::Carried(*carrier),
                    None => {
                        next.push(member);
                        continue;
                    }
                },
            };
            push(&mut members, &mut bodies, member, holding)?;
        }
        if next.len() == before {
            let name = next.first().map(|m| m.name.clone()).unwrap_or_default();
            return Err(DeclareError::InCircle(name));
        }
        left = next;
    }
    // Then each role an entry names: its part, held or riding on the
    // driven member as a member with: it would. It adds no freedom.
    let mut role_members: BTreeMap<RoleIndex, MemberIndex> = BTreeMap::new();
    for entry in &mechanism.entries {
        let EntrySide::Role(role) = entry.bodies().other else {
            continue;
        };
        if role_members.contains_key(&role) {
            continue;
        }
        let holding = match driven {
            Some(driven) if moving(role) => Holding::Carried(driven),
            Some(_) | None => Holding::Held,
        };
        let name = mechanism
            .roles
            .get(role.index())
            .ok_or(DeclareError::Internal)?;
        role_members.insert(role, index(members.len())?);
        members.push(MechanismMember {
            hardware,
            body: MemberBody::Role { role },
            name: name.clone(),
            holding,
        });
    }
    let to = |body: &BodyId| bodies.get(body).copied().ok_or(DeclareError::Internal);
    let side = |side: EntrySide| match side {
        EntrySide::Member(body) => to(&body),
        EntrySide::Role(role) => role_members
            .get(&role)
            .copied()
            .ok_or(DeclareError::Internal),
    };
    let point = |p: PlanePoint| -> Result<FramePoint, DeclareError> {
        let [across, depth] = base
            .point_of(&mechanism.plane, p)
            .ok_or(DeclareError::Internal)?;
        Ok(FramePoint::new(
            across.to_nearest_length().ok_or(DeclareError::Internal)?,
            depth.to_nearest_length().ok_or(DeclareError::Internal)?,
        ))
    };
    let mut entries = Vec::new();
    for entry in &mechanism.entries {
        let pair = entry.bodies();
        let (a, b) = (to(&pair.member)?, side(pair.other)?);
        entries.push(match *entry {
            Entry::Pivot { at, .. } => MechanismEntry::Pivot {
                first: a,
                second: b,
                at: point(at)?,
            },
            Entry::Slot { from, to, .. } => MechanismEntry::Slot {
                pin: a,
                guide: b,
                from: point(from)?,
                to: point(to)?,
            },
            Entry::Rail { direction, .. } => {
                let [across, depth] = base
                    .vector_of(&mechanism.plane, direction)
                    .ok_or(DeclareError::Internal)?;
                MechanismEntry::Rail {
                    runner: a,
                    guide: b,
                    direction: FrameDirection::new(
                        across.to_nearest_length().ok_or(DeclareError::Internal)?,
                        depth.to_nearest_length().ok_or(DeclareError::Internal)?,
                    )
                    .ok_or(DeclareError::Internal)?,
                }
            }
            Entry::Couple { ratio, .. } => MechanismEntry::Couple {
                follower: a,
                leader: b,
                ratio,
            },
        });
    }
    let same_hand = base
        .same_hand(&mechanism.plane)
        .ok_or(DeclareError::Internal)?;
    let angle = |angle: ExactAngle| -> Result<stave_lang::units::Angle, DeclareError> {
        let angle = if same_hand {
            angle
        } else {
            angle.checked_neg().ok_or(DeclareError::Internal)?
        };
        angle.to_nearest_angle().ok_or(DeclareError::Internal)
    };
    let length = |length: ExactLength| length.to_nearest_length().ok_or(DeclareError::Internal);
    let mut drives = Vec::with_capacity(mechanism.drives.len());
    for named in &mechanism.drives {
        let drive = match named.drive {
            Drive::Turn {
                body,
                from,
                to: end,
            } => MechanismDrive::Turn {
                member: to(&body)?,
                from: angle(from)?,
                to: angle(end)?,
            },
            Drive::Travel {
                body,
                from,
                to: end,
            } => MechanismDrive::Travel {
                member: to(&body)?,
                from: length(from)?,
                to: length(end)?,
            },
        };
        // A set value is a drive value, turned the other way with the
        // plane as `from` and `to` are; a step is how far apart they are.
        let value = |value: Rational| -> Result<DriveValue, DeclareError> {
            Ok(match drive {
                MechanismDrive::Turn { .. } => DriveValue::Turn(angle(ExactAngle::new(value))?),
                MechanismDrive::Travel { .. } => {
                    DriveValue::Travel(length(ExactLength::new(value))?)
                }
            })
        };
        let step = |step: Rational| -> Result<DriveValue, DeclareError> {
            Ok(match drive {
                MechanismDrive::Turn { .. } => DriveValue::Turn(
                    ExactAngle::new(step)
                        .to_nearest_angle()
                        .ok_or(DeclareError::Internal)?,
                ),
                MechanismDrive::Travel { .. } => {
                    DriveValue::Travel(length(ExactLength::new(step))?)
                }
            })
        };
        let set_at = match &named.set_at {
            Some(set) => {
                let values = set
                    .values
                    .iter()
                    .map(|v| value(*v))
                    .collect::<Result<Vec<_>, _>>()?;
                let every = set.every.map(step).transpose()?;
                Some(SetPositions::new(&drive, values, every).ok_or(DeclareError::Internal)?)
            }
            None => None,
        };
        drives.push(NamedDrive {
            name: named.name.clone(),
            drive,
            set_at,
        });
    }
    let stop = match &mechanism.stop {
        Some((StopValue::Angle(at), message)) => Some(MechanismStop {
            at: DriveValue::Turn(angle(*at)?),
            message: message.clone(),
        }),
        Some((StopValue::Length(at), message)) => Some(MechanismStop {
            at: DriveValue::Travel(length(*at)?),
            message: message.clone(),
        }),
        None => None,
    };
    Ok(Declared {
        members,
        entries,
        drives,
        moving: own_moving.and(driven),
        stop,
        bodies,
    })
}

/// The free members no chain of entries reaches from the members already
/// placed (held ones, carried ones and the driven one).
fn unreachable(members: &[MechanismMember], entries: &[MechanismEntry]) -> Vec<String> {
    let mut known: Vec<bool> = members
        .iter()
        .map(|member| member.holding != Holding::Free)
        .collect();
    let piece = |at: MemberIndex| {
        let mut at = at;
        for _ in 0..members.len() {
            match members.get(at.index()).map(|m| m.holding) {
                Some(Holding::Carried(carrier)) => at = carrier,
                Some(Holding::Held | Holding::Free) | None => break,
            }
        }
        at.index()
    };
    // The driven member (the first free one declared) is placed by the
    // drive.
    if let Some(first) = members.iter().position(|m| m.holding == Holding::Free)
        && let Some(slot) = known.get_mut(first)
    {
        *slot = true;
    }
    loop {
        let mut changed = false;
        for entry in entries {
            let [a, b] = entry.members().map(piece);
            let (ka, kb) = (
                known.get(a).copied().unwrap_or(false),
                known.get(b).copied().unwrap_or(false),
            );
            if ka != kb {
                for at in [a, b] {
                    if let Some(slot) = known.get_mut(at) {
                        *slot = true;
                    }
                }
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    members
        .iter()
        .zip(&known)
        .filter(|(_, known)| !**known)
        .map(|(member, _)| member.name.clone())
        .collect()
}

/// Three freedoms for each free member, less two for each pivot or rail
/// and one for each slot or couple.
fn freedoms(members: &[MechanismMember], entries: &[MechanismEntry]) -> Option<i64> {
    let free = members
        .iter()
        .filter(|m| m.holding == Holding::Free)
        .count();
    let taken = entries
        .iter()
        .try_fold(0i64, |sum, entry| sum.checked_add(entry.takes()))?;
    i64::try_from(free).ok()?.checked_mul(3)?.checked_sub(taken)
}

/// The value of a drive between `from` and a candidate end that is nearer
/// `from`, of two.
fn nearer(from: DriveValue, a: DriveValue, b: DriveValue) -> DriveValue {
    let distance = |value: DriveValue| match (from, value) {
        (DriveValue::Turn(f), DriveValue::Turn(v)) => {
            v.microdeg().saturating_sub(f.microdeg()).saturating_abs()
        }
        (DriveValue::Travel(f), DriveValue::Travel(v)) => {
            v.nm().saturating_sub(f.nm()).saturating_abs()
        }
        (DriveValue::Turn(_), DriveValue::Travel(_))
        | (DriveValue::Travel(_), DriveValue::Turn(_)) => i64::MAX,
    };
    if distance(b) < distance(a) { b } else { a }
}

/// An assembly's mechanism: the solved one, which member each body of its
/// hardware became, and the hardware it was made from (the first driver
/// first).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct AssemblyMechanism {
    pub mechanism: Mechanism,
    pub bodies: BTreeMap<(HardwareId, BodyId), MemberIndex>,
}

impl Solver<'_> {
    /// Which roles of a placed piece of hardware move with `assembly`:
    /// every role bound to something inside it. A piece of hardware no
    /// assembly moves is counted with its piece (the first role) moving,
    /// so that a mechanism is checked wherever it is used.
    fn moving_roles(
        &self,
        roles: &[(String, RoleBound)],
        assembly: Option<AssemblyId>,
    ) -> Vec<bool> {
        roles
            .iter()
            .enumerate()
            .map(|(index, (_, bound))| match assembly {
                None => index == 0,
                Some(assembly) => {
                    let inside = match *bound {
                        RoleBound::Part(part) => {
                            self.model.parts.get(part.index()).and_then(|p| p.assembly)
                        }
                        RoleBound::Region(region) => self
                            .model
                            .regions
                            .get(region.index())
                            .and_then(|r| r.assembly),
                    };
                    self.within(inside, assembly)
                }
            })
            .collect()
    }

    /// Where a kind's mechanism is written.
    fn mechanism_place(&self, kind: KindId) -> Option<(stave_lang::FileId, stave_lang::ast::Span)> {
        let def: &KindDef = self.model.defs.sound_kind(kind).ok()?;
        Some((def.decl.file, def.mechanism()?.span))
    }

    /// Where a piece of hardware is declared.
    fn hardware_call(
        &self,
        hardware: HardwareId,
    ) -> Option<(stave_lang::FileId, stave_lang::ast::Span)> {
        let inst = self.model.hardware.get(hardware.index())?;
        let def = self.model.defs.sound_hardware_def(inst.def).ok()?;
        Some((def.decl.file, def.call.span))
    }

    /// Reports a mechanism error at its kind, once per kind: after the
    /// first, the failure is the one reported earlier.
    fn mechanism_error(
        &mut self,
        kind: KindId,
        message: String,
        note: Option<String>,
        help: &str,
    ) -> Failure {
        if !self.reported_mechanisms.insert(kind) {
            return Fault::Earlier.into();
        }
        let Some((file, span)) = self.mechanism_place(kind) else {
            return Fault::Internal("where a hardware kind's mechanism is written").into();
        };
        let mut diagnostic =
            Diagnostic::error(Code::Mechanism, message, Label::new(file, span)).with_help(help);
        if let Some(note) = note {
            diagnostic = diagnostic.with_note(note);
        }
        self.report(diagnostic).into()
    }

    /// Counts one piece of hardware's mechanism on its own, as soon as it
    /// is placed: one freedom with a drive, none without, every free
    /// member reached from what is placed. Reported once per kind. A
    /// mechanism with a drive comes back counted, in its own plane, for
    /// the checks to run through its range even when no assembly moves it.
    pub(crate) fn check_mechanism(
        &mut self,
        hardware: HardwareId,
        roles: &[(String, RoleBound)],
        mechanism: &HardwareMechanism,
        kind_name: &str,
    ) -> Solved<Option<Mechanism>> {
        let assembly = self
            .model
            .hardware
            .get(hardware.index())
            .and_then(|h| h.assembly);
        let moving = self.moving_roles(roles, assembly);
        let is_moving = |role: RoleIndex| moving.get(role.index()).copied().unwrap_or(false);
        let declared = match declare(hardware, mechanism, &is_moving, &mechanism.plane, None, 0) {
            Ok(declared) => declared,
            Err(error) => {
                return Err(self.declare_error(mechanism.kind, kind_name, error));
            }
        };
        let Some(found) = freedoms(&declared.members, &declared.entries) else {
            return Err(Fault::Internal("a mechanism's freedoms").into());
        };
        let wanted = i64::try_from(declared.drives.len())
            .map_err(|_| Fault::Internal("a mechanism's drives"))?;
        if found != wanted {
            let (message, help) = if declared.drives.len() > 1 {
                (
                    format!(
                        "{kind_name}'s mechanism has {found} freedoms; it needs exactly {wanted}, one for each drive"
                    ),
                    "count three for the driven member and for each member with no 'with:', less two for each pivot or rail and one for each slot or couple; add or remove pins until one is left for each drive",
                )
            } else if !declared.drives.is_empty() {
                (
                    format!("{kind_name}'s mechanism has {found} freedoms; it needs exactly one"),
                    "count three for the driven member and for each member with no 'with:', less two for each pivot or rail and one for each slot or couple; add or remove pins until one is left",
                )
            } else {
                (
                    format!(
                        "{kind_name}'s mechanism has {found} freedoms once its assembly moves; a mechanism without a drive needs none"
                    ),
                    "count three for each member with no 'with:', less two for each pivot or rail and one for each slot or couple; add or remove pins until none is left",
                )
            };
            return Err(self.mechanism_error(mechanism.kind, message, None, help));
        }
        let left = unreachable(&declared.members, &declared.entries);
        if !left.is_empty() {
            return Err(self.mechanism_error(
                mechanism.kind,
                format!(
                    "{kind_name}'s mechanism can't be solved one step at a time from its drive"
                ),
                Some(format!("nothing places {}", left.join(" and "))),
                "pin each of them to a member that is held or driven, or to one pinned to those",
            ));
        }
        if declared.drives.is_empty() {
            return Ok(None);
        }
        let frame = mechanism
            .plane
            .frame()
            .ok_or(Fault::Internal("a mechanism's plane"))?;
        let Some(moving) = declared.moving else {
            return Err(Fault::Internal("the member a mechanism moves with").into());
        };
        match Mechanism::with_drives(
            frame,
            declared.members,
            declared.entries,
            declared.drives,
            moving,
            declared.stop,
            vec![hardware],
        ) {
            Ok(counted) => Ok(Some(counted)),
            Err(error) => Err(self.counted_error(mechanism.kind, kind_name, &error)),
        }
    }

    fn declare_error(&mut self, kind: KindId, kind_name: &str, error: DeclareError) -> Failure {
        match error {
            DeclareError::Internal => Fault::Internal("a mechanism's members").into(),
            DeclareError::DriveHeld(member) => self.mechanism_error(
                kind,
                format!(
                    "{kind_name}'s drive moves '{member}', which is with: a part that stays put"
                ),
                None,
                "drive a member the mechanism places: one with no with:, or the one with: the part that moves",
            ),
            DeclareError::NothingMoves => self.mechanism_error(
                kind,
                format!("{kind_name}'s mechanism has no member with: the part that moves"),
                None,
                "give one member with: the piece that moves (the door, the lid, the drawer side): the assembly moves with it",
            ),
            DeclareError::InCircle(member) => self.mechanism_error(
                kind,
                format!("{kind_name}'s member '{member}' rides on itself"),
                None,
                "give one of the members it rides on with: a role",
            ),
        }
    }

    fn counted_error(&mut self, kind: KindId, kind_name: &str, error: &MechanismError) -> Failure {
        let help = match error {
            MechanismError::OnePiece { .. } | MechanismError::BothHeld { .. } => {
                "join a member that moves to one that holds it, or to another that moves"
            }
            MechanismError::ZeroRatio { .. } => "give the couple a share that isn't zero",
            MechanismError::Freedom { .. } => {
                "add or remove pins until one freedom is left for the drive"
            }
            MechanismError::DriveFreedom { .. } => {
                "add or remove pins until one freedom is left for each drive"
            }
            MechanismError::TwoTurns { .. } => {
                "turn the member with one drive, or turn another member with the second"
            }
            MechanismError::TwoTravels { .. } => {
                "move the member with one drive, or move another member with the second"
            }
            MechanismError::DriveNotFree { .. } | MechanismError::MovingNotFree { .. } => {
                "drive a member the mechanism places: one with no with:, or the one with: the part that moves"
            }
            MechanismError::EmptyRange => "give the drive a from: and a to: that differ",
            // Checked when the kind is resolved.
            MechanismError::UnknownMember { .. }
            | MechanismError::NoMembers
            | MechanismError::NoDrives => {
                return Fault::Earlier.into();
            }
            MechanismError::Overflow => return Fault::Overflow.into(),
        };
        self.mechanism_error(
            kind,
            format!("{kind_name}'s mechanism can't move: {error}"),
            None,
            help,
        )
    }

    /// Every assembly's mechanism, from its hardware (grammar, "Mechanisms",
    /// several items on one assembly), indexed by assembly; `None` for an
    /// assembly nothing drives. Reports pins that aren't parallel and
    /// drivers that don't move alike.
    pub(crate) fn assembly_mechanisms(
        &mut self,
        placed: &[Option<Placed>],
    ) -> Vec<Option<AssemblyMechanism>> {
        let assemblies: Vec<AssemblyId> = self.model.assemblies.iter().map(|a| a.id).collect();
        assemblies
            .into_iter()
            .map(|assembly| {
                let result = self.assembly_mechanism(assembly, placed);
                let (file, span) = match self.model.assemblies.get(assembly.index()) {
                    Some(inst) => self.place_of(stave_model::defs::DefId::Assembly(inst.def)),
                    None => (self.project_file, self.project_span),
                };
                self.settled(result, file, span).flatten()
            })
            .collect()
    }

    fn assembly_mechanism(
        &mut self,
        assembly: AssemblyId,
        placed: &[Option<Placed>],
    ) -> Solved<Option<AssemblyMechanism>> {
        let items: Vec<(HardwareId, &Placed, &HardwareMechanism)> = self
            .model
            .hardware
            .iter()
            .filter(|h| h.assembly == Some(assembly))
            .filter_map(|h| {
                let placed = placed.get(h.id.index())?.as_ref()?;
                Some((h.id, placed, placed.mechanism.as_ref()?))
            })
            .collect();
        let (drivers, followers): (Vec<_>, Vec<_>) = items
            .into_iter()
            .partition(|(_, _, mechanism)| !mechanism.drives.is_empty());
        let Some(&(first, first_placed, base)) = drivers.first() else {
            return Ok(None);
        };
        let assembly_name = self
            .model
            .assemblies
            .get(assembly.index())
            .map(|a| a.name.clone())
            .unwrap_or_default();
        let hardware_name = |solver: &Self, id: HardwareId| {
            solver
                .model
                .hardware
                .get(id.index())
                .map(|h| h.name.clone())
                .unwrap_or_default()
        };
        // Every pin parallel to one axis.
        for (id, _, mechanism) in drivers.iter().chain(&followers) {
            if mechanism.plane.axis != base.plane.axis {
                let (file, span) = self
                    .hardware_call(*id)
                    .ok_or(Fault::Internal("where a piece of hardware is written"))?;
                return Err(self.error(
                    Code::Motion,
                    format!(
                        "{}'s pins run {}, but {}'s run {}: {assembly_name} can't move on both",
                        hardware_name(self, *id),
                        mechanism.plane.axis.word(),
                        hardware_name(self, first),
                        base.plane.axis.word()
                    ),
                    file,
                    span,
                    "mount the hardware of one assembly so that it all turns or runs the same way",
                ).into());
            }
        }
        // A drive's name is read through the assembly, so no item of its
        // component has it.
        for (id, _, mechanism) in &drivers {
            self.drive_names_clash(assembly, *id, mechanism);
        }
        let moving = self.moving_roles(&first_placed.roles, Some(assembly));
        let is_moving = |role: RoleIndex| moving.get(role.index()).copied().unwrap_or(false);
        // Each piece of hardware's own mechanism was checked when placed.
        let Ok(mut declared) = declare(first, base, &is_moving, &base.plane, None, 0) else {
            return Err(Fault::Earlier.into());
        };
        let mut drives = std::mem::take(&mut declared.drives);
        let Some(driven) = declared.moving else {
            return Err(Fault::Internal("the member a mechanism moves with").into());
        };
        let mut stop = declared.stop.take();
        let mut bodies: BTreeMap<(HardwareId, BodyId), MemberIndex> = declared
            .bodies
            .iter()
            .map(|(body, index)| ((first, *body), *index))
            .collect();
        let mut hardware = vec![first];
        // Every other driver must move alike: the same members, entries
        // and drive, seen in the first one's plane.
        for &(id, item_placed, mechanism) in drivers.iter().skip(1) {
            let moving = self.moving_roles(&item_placed.roles, Some(assembly));
            let is_moving = |role: RoleIndex| moving.get(role.index()).copied().unwrap_or(false);
            let Ok(other) = declare(id, mechanism, &is_moving, &base.plane, None, 0) else {
                return Err(Fault::Earlier.into());
            };
            let start = |named: &NamedDrive| {
                (
                    named.name.clone(),
                    named.drive.member(),
                    named.drive.from(),
                    named.drive.turns(),
                    named.set_at.clone(),
                )
            };
            let alike = other
                .members
                .iter()
                .map(|m| (&m.name, m.holding))
                .eq(declared.members.iter().map(|m| (&m.name, m.holding)))
                && other.entries == declared.entries
                && other.drives.iter().map(start).eq(drives.iter().map(start));
            if !alike {
                let (file, span) = self
                    .hardware_call(id)
                    .ok_or(Fault::Internal("where a piece of hardware is written"))?;
                let first_name = hardware_name(self, first);
                let other_name = hardware_name(self, id);
                let reported = self.report(
                    Diagnostic::error(
                        Code::Motion,
                        format!("{assembly_name}'s hardware doesn't move alike"),
                        Label::new(file, span),
                    )
                    .with_note(format!(
                        "{first_name} and {other_name} would move {assembly_name} in different ways"
                    ))
                    .with_help(format!(
                        "use one kind of hinge or slide, set the same way, for everything that moves {assembly_name}"
                    )),
                );
                return Err(reported.into());
            }
            for (named, other_named) in drives.iter_mut().zip(&other.drives) {
                if let Some(narrower) = narrower(&named.drive, other_named.drive.to()) {
                    named.drive = narrower;
                }
            }
            if let Some(other_stop) = other.stop
                && let Some(first_drive) = drives.first()
            {
                let from = first_drive.drive.from();
                stop = Some(match stop {
                    Some(current) if nearer(from, current.at, other_stop.at) == current.at => {
                        current
                    }
                    Some(_) | None => other_stop,
                });
            }
            for (body, index) in other.bodies {
                bodies.insert((id, body), index);
            }
            hardware.push(id);
        }
        // The followers: their members and entries added, each riding on
        // the driven member where it is with: a part that moves.
        for &(id, item_placed, mechanism) in &followers {
            let moving = self.moving_roles(&item_placed.roles, Some(assembly));
            let is_moving = |role: RoleIndex| moving.get(role.index()).copied().unwrap_or(false);
            let offset = declared.members.len();
            let Ok(follower) =
                declare(id, mechanism, &is_moving, &base.plane, Some(driven), offset)
            else {
                return Err(Fault::Earlier.into());
            };
            if let Some(follower_stop) = follower.stop {
                // An angle stops the drive that turns, a length the one
                // that travels.
                let stopped = drives
                    .iter()
                    .position(|named| {
                        named.drive.turns() == matches!(follower_stop.at, DriveValue::Turn(_))
                    })
                    .unwrap_or(0);
                let Some(drive) = drives.get(stopped).map(|named| named.drive) else {
                    return Err(Fault::Internal("the drive of a mechanism").into());
                };
                let at = match (follower_stop.at, drive) {
                    (DriveValue::Turn(_), MechanismDrive::Turn { .. })
                    | (DriveValue::Travel(_), MechanismDrive::Travel { .. }) => follower_stop.at,
                    (DriveValue::Turn(_), MechanismDrive::Travel { .. })
                    | (DriveValue::Travel(_), MechanismDrive::Turn { .. }) => {
                        let (file, span) = self
                            .hardware_call(id)
                            .ok_or(Fault::Internal("where a piece of hardware is written"))?;
                        return Err(self.error(
                            Code::TypeMismatch,
                            format!(
                                "{}'s stop is {}, but {assembly_name} {}",
                                hardware_name(self, id),
                                if drive.turns() { "a length" } else { "an angle" },
                                if drive.turns() { "turns" } else { "travels" }
                            ),
                            file,
                            span,
                            "give the stop an angle for something that turns, a length for something that slides",
                        ).into());
                    }
                };
                if let Some(narrower) = narrower(&drive, at)
                    && let Some(named) = drives.get_mut(stopped)
                {
                    named.drive = narrower;
                }
                // What stops a mechanism with several drives is its
                // drives' own ends.
                if drives.len() == 1 {
                    stop = Some(match stop {
                        Some(current) if nearer(drive.from(), current.at, at) == current.at => {
                            current
                        }
                        Some(_) | None => MechanismStop {
                            at,
                            message: follower_stop.message,
                        },
                    });
                }
            }
            declared.members.extend(follower.members);
            declared.entries.extend(follower.entries);
            for (body, index) in follower.bodies {
                bodies.insert((id, body), index);
            }
            hardware.push(id);
        }
        let frame = base
            .plane
            .frame()
            .ok_or(Fault::Internal("a mechanism's plane"))?;
        match Mechanism::with_drives(
            frame,
            declared.members,
            declared.entries,
            drives,
            driven,
            stop,
            hardware,
        ) {
            Ok(mechanism) => Ok(Some(AssemblyMechanism { mechanism, bodies })),
            Err(error) => {
                let kind_name = self.def_name(stave_model::defs::DefId::Kind(base.kind));
                Err(self.counted_error(base.kind, &kind_name, &error))
            }
        }
    }
}

/// `a`, `a or b`, `a, b or c`.
fn and_list(items: &[String], word: &str) -> String {
    match items {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} {word} {last}", rest.join(", ")),
    }
}

/// A drive whose range ends at `to` when that is nearer its `from`
/// than its own end; `None` when it isn't.
fn narrower(drive: &MechanismDrive, to: DriveValue) -> Option<MechanismDrive> {
    if nearer(drive.from(), drive.to(), to) == drive.to() {
        return None;
    }
    Some(match (*drive, to) {
        (MechanismDrive::Turn { member, from, .. }, DriveValue::Turn(to)) => {
            MechanismDrive::Turn { member, from, to }
        }
        (MechanismDrive::Travel { member, from, .. }, DriveValue::Travel(to)) => {
            MechanismDrive::Travel { member, from, to }
        }
        (MechanismDrive::Turn { .. }, DriveValue::Travel(_))
        | (MechanismDrive::Travel { .. }, DriveValue::Turn(_)) => return None,
    })
}

impl Solver<'_> {
    /// A drive name of a piece of hardware that moves `assembly` which is
    /// also the name of an item of the assembly's component is
    /// `error[duplicate-name]` at the hardware (grammar, "Mechanisms",
    /// drives), once for each component and name, at the first piece of
    /// hardware with that drive.
    fn drive_names_clash(
        &mut self,
        assembly: AssemblyId,
        hardware: HardwareId,
        mechanism: &HardwareMechanism,
    ) {
        let Some(inst) = self.model.assemblies.get(assembly.index()) else {
            return;
        };
        let assembly_name = inst.name.clone();
        let Some(hardware_inst) = self.model.hardware.get(hardware.index()) else {
            return;
        };
        let hardware_def = hardware_inst.def;
        // The component the hardware is declared in.
        let Some(stave_model::defs::Scope::Component(component)) = self
            .model
            .defs
            .decl(stave_model::defs::DefId::Hardware(hardware_def))
            .map(|decl| decl.scope)
        else {
            return;
        };
        let hardware_name = hardware_inst.name.clone();
        let items: Vec<String> = self
            .model
            .defs
            .sound_component(component)
            .map(|def| def.items.iter().map(|item| self.def_name(*item)).collect())
            .unwrap_or_default();
        for named in &mechanism.drives {
            let Some(name) = &named.name else {
                continue;
            };
            if !items.contains(name) || !self.reported_drive_names.insert((component, name.clone()))
            {
                continue;
            }
            let Some((file, span)) = self.hardware_call(hardware) else {
                continue;
            };
            let kind_name = self.def_name(stave_model::defs::DefId::Kind(mechanism.kind));
            self.report(
                Diagnostic::error(
                    Code::DuplicateName,
                    format!(
                        "`{name}` is a drive of {hardware_name}'s kind and an item of {assembly_name}"
                    ),
                    Label::new(file, span),
                )
                .with_note(format!(
                    "pose {assembly_name}.{name} would name both; {kind_name} calls one of its drives `{name}`"
                ))
                .with_help("rename the item, or name the drive another way in its kind"),
            );
        }
    }

    /// How a scene's pose names a drive (grammar, "Scenes", a pose names a
    /// drive): `Some` for one drive of an assembly with several, `None`
    /// for every drive (the one drive, or `open` and `closed` for all).
    /// A name that isn't one of the assembly's drives is
    /// `error[unknown-name]`, a value of the other kind for the drive
    /// `error[type-mismatch]`, and an angle or a length without a drive's
    /// name for an assembly with several `error[motion]`.
    pub(crate) fn pose_drive(
        &mut self,
        assembly: AssemblyId,
        mechanism: Option<&Mechanism>,
        pose: &stave_model::defs::PoseDef,
        value: stave_model::solved::PoseValue,
        file: stave_lang::FileId,
    ) -> Solved<Option<DriveIndex>> {
        use stave_model::solved::PoseValue;
        let name = self
            .model
            .assemblies
            .get(assembly.index())
            .map(|a| a.name.clone())
            .unwrap_or_default();
        let named: Vec<String> = mechanism
            .map(|m| m.drives().iter().filter_map(|d| d.name.clone()).collect())
            .unwrap_or_default();
        let drive = match (&pose.drive, mechanism) {
            (Some((drive_name, span)), None) => {
                return Err(self
                    .error(
                        Code::UnknownName,
                        format!("`{name}` doesn't move, so it has no drive `{drive_name}`"),
                        file,
                        *span,
                        format!("pose an assembly its hardware moves, or write pose {name} = ..."),
                    )
                    .into());
            }
            (Some((drive_name, span)), Some(mechanism)) => {
                match mechanism.drive_named(drive_name) {
                    Some(drive) => drive,
                    None => {
                        let help = if named.is_empty() {
                            format!("{name} moves one way: write pose {name} = ...")
                        } else {
                            let choices: Vec<String> =
                                named.iter().map(|d| format!("{name}.{d}")).collect();
                            format!("its drives are {}", and_list(&choices, "and"))
                        };
                        return Err(self
                            .error(
                                Code::UnknownName,
                                format!("`{name}` has no drive `{drive_name}`"),
                                file,
                                *span,
                                help,
                            )
                            .into());
                    }
                }
            }
            (None, Some(mechanism)) if mechanism.drives().len() > 1 => {
                if matches!(value, PoseValue::Open | PoseValue::Closed) {
                    return Ok(None);
                }
                let ways = match mechanism.drives().len() {
                    2 => "two".to_owned(),
                    3 => "three".to_owned(),
                    other => other.to_string(),
                };
                let choices: Vec<String> = named.iter().map(|d| format!("{name}.{d}")).collect();
                return Err(self
                    .error(
                        Code::Motion,
                        format!("`{name}` moves {ways} ways; write {}", and_list(&choices, "or")),
                        file,
                        pose.span,
                        "name the drive the pose sets after the assembly, as in pose frame.tilt = 15deg",
                    )
                    .into());
            }
            (None, Some(_)) => DriveIndex::FIRST,
            (None, None) => return Ok(None),
        };
        let Some(mechanism) = mechanism else {
            return Ok(None);
        };
        let Some(found) = mechanism.drive_at(drive) else {
            return Err(Fault::Internal("a drive of a mechanism").into());
        };
        let label = match (
            &found.name,
            mechanism.drives().len() > 1 || pose.drive.is_some(),
        ) {
            (Some(drive_name), true) => format!("{name}.{drive_name}"),
            (Some(_) | None, _) => name,
        };
        let mismatch = match (found.drive.turns(), value) {
            (true, PoseValue::Travel(_)) => Some(("turns", "an angle", "a length", "15deg")),
            (false, PoseValue::Angle(_)) => Some(("travels", "a length", "an angle", "150mm")),
            (
                _,
                PoseValue::Open | PoseValue::Closed | PoseValue::Angle(_) | PoseValue::Travel(_),
            ) => None,
        };
        if let Some((moves, wanted, found_kind, example)) = mismatch {
            return Err(self
                .error(
                    Code::TypeMismatch,
                    format!("`{label}` {moves}, so its pose is {wanted}, found {found_kind}"),
                    file,
                    pose.value.span,
                    format!("write {wanted} such as {example}, or open or closed"),
                )
                .into());
        }
        // One drive is every drive.
        Ok((mechanism.drives().len() > 1).then_some(drive))
    }

    /// An assembly, or one drive of it, as a message names it.
    pub(crate) fn pose_name(
        &self,
        assembly: AssemblyId,
        mechanism: Option<&Mechanism>,
        drive: Option<DriveIndex>,
    ) -> String {
        let name = self
            .model
            .assemblies
            .get(assembly.index())
            .map(|a| a.name.clone())
            .unwrap_or_default();
        match drive
            .and_then(|drive| mechanism?.drive_at(drive))
            .and_then(|named| named.name.as_ref())
        {
            Some(drive_name) => format!("{name}.{drive_name}"),
            None => name,
        }
    }

    /// A scene's pose of a drive of an assembly asks no more of it than it
    /// reaches: an angle or a travel past it is `error[motion]`, with the
    /// stop's message when a `stop` ends the range.
    pub(crate) fn pose_within(
        &mut self,
        assembly: AssemblyId,
        mechanism: &Mechanism,
        drive: DriveIndex,
        value: stave_model::solved::PoseValue,
        file: stave_lang::FileId,
        span: stave_lang::ast::Span,
    ) -> Solved<()> {
        use stave_model::solved::PoseValue;
        let Some(named) = mechanism.drive_at(drive) else {
            return Ok(());
        };
        let extent = named.drive.extent();
        let asked = match (value, extent) {
            (PoseValue::Angle(angle), DriveValue::Turn(reach)) if angle > reach => {
                Some((angle.to_string(), reach.to_string(), "open"))
            }
            (PoseValue::Travel(length), DriveValue::Travel(reach)) if length > reach => {
                Some((length.to_string(), reach.to_string(), "pull out"))
            }
            _ => None,
        };
        let Some((asked, reach, verb)) = asked else {
            return Ok(());
        };
        let assembly_name = self
            .model
            .assemblies
            .get(assembly.index())
            .map(|a| a.name.clone())
            .unwrap_or_default();
        let name = match &named.name {
            Some(drive_name) if mechanism.drives().len() > 1 => {
                format!("{assembly_name}.{drive_name}")
            }
            Some(_) | None => assembly_name,
        };
        let why = match mechanism.stop() {
            Some(stop) if stop.at == named.drive.to() => stop.message.clone(),
            Some(_) | None => format!("its hardware goes {reach} at most"),
        };
        Err(self
            .error(
                Code::Motion,
                format!("{name} can't {verb} {asked}: {why}"),
                file,
                span,
                format!("pose {name} at most {reach}, or write open"),
            )
            .into())
    }

    /// A drive's set positions, checked (grammar, "Mechanisms", set
    /// positions): a step of `every(d)` not more than nothing, or one
    /// that makes more than 1000 of them, is `error[out-of-range]`; a
    /// value outside the range is `error[motion]` at the `set_at:`.
    fn set_positions(
        &mut self,
        kind_name: &str,
        file: stave_lang::FileId,
        name: Option<&str>,
        drive: &Drive,
        written: WrittenSetAt,
    ) -> Solved<KindSetAt> {
        let (from, to) = match *drive {
            Drive::Turn { from, to, .. } => (from.value(), to.value()),
            Drive::Travel { from, to, .. } => (from.value(), to.value()),
        };
        let turns = matches!(drive, Drive::Turn { .. });
        let text = |value: Rational| -> String {
            if turns {
                ExactAngle::new(value)
                    .to_nearest_angle()
                    .map(|angle| angle.to_string())
                    .unwrap_or_default()
            } else {
                ExactLength::new(value)
                    .to_nearest_length()
                    .map(|length| length.to_string())
                    .unwrap_or_default()
            }
        };
        let (low, high) = (from.min(to), from.max(to));
        match written {
            WrittenSetAt::Every(step, span) => {
                if step <= Rational::ZERO {
                    let zero = text(Rational::ZERO);
                    return Err(self
                        .error(
                            Code::OutOfRange,
                            format!(
                                "every(...) needs a step more than {zero}, found {}",
                                text(step)
                            ),
                            file,
                            span,
                            "write how far apart the set positions are, such as every(50mm) or every(15deg)",
                        )
                        .into());
                }
                let reach = high.checked_sub(low).ok_or(Fault::Overflow)?;
                let steps = reach.checked_div(step).ok_or(Fault::Overflow)?.floor();
                if steps >= MOST_SET_POSITIONS {
                    return Err(self
                        .error(
                            Code::OutOfRange,
                            format!(
                                "every({}) makes {} set positions; a drive has at most {MOST_SET_POSITIONS}",
                                text(step),
                                steps.saturating_add(1)
                            ),
                            file,
                            span,
                            "make the step larger, or list the values instead",
                        )
                        .into());
                }
                let toward = if to >= from {
                    step
                } else {
                    step.checked_neg().ok_or(Fault::Overflow)?
                };
                let mut values = Vec::new();
                for count in 0..=steps {
                    let by = toward
                        .checked_mul(Rational::from_int(count))
                        .ok_or(Fault::Overflow)?;
                    values.push(from.checked_add(by).ok_or(Fault::Overflow)?);
                }
                Ok(KindSetAt {
                    values,
                    every: Some(step),
                })
            }
            WrittenSetAt::List(values, span) => {
                if let Some(outside) = values.iter().find(|value| **value < low || **value > high) {
                    let drive_name = match name {
                        Some(name) => format!("drive {name}"),
                        None => "drive".to_owned(),
                    };
                    return Err(self
                        .error(
                            Code::Motion,
                            format!(
                                "{kind_name}'s {drive_name} can't be set at {}: it runs from {} to {}",
                                text(*outside),
                                text(from),
                                text(to)
                            ),
                            file,
                            span,
                            "set it only at values from its from: to its to:",
                        )
                        .into());
                }
                Ok(KindSetAt {
                    values,
                    every: None,
                })
            }
        }
    }

    /// A scene's pose of a drive with set positions lands on one of them
    /// (grammar, "Scenes"): an angle or a travel between them is
    /// `error[motion]`, with the nearest set values in the help.
    pub(crate) fn pose_sets(
        &mut self,
        assembly: AssemblyId,
        mechanism: &Mechanism,
        drive: DriveIndex,
        value: stave_model::solved::PoseValue,
        file: stave_lang::FileId,
        span: stave_lang::ast::Span,
    ) -> Solved<()> {
        use stave_model::solved::PoseValue;
        let Some(named) = mechanism.drive_at(drive) else {
            return Ok(());
        };
        let Some(set) = &named.set_at else {
            return Ok(());
        };
        let amount = match value {
            PoseValue::Angle(angle) => DriveValue::Turn(angle),
            PoseValue::Travel(length) => DriveValue::Travel(length),
            PoseValue::Open | PoseValue::Closed => return Ok(()),
        };
        let Some(at) = named.drive.value_at(amount) else {
            return Ok(());
        };
        if named.sets_at(at) {
            return Ok(());
        }
        let text = |value: DriveValue| match value {
            DriveValue::Turn(angle) => angle.to_string(),
            DriveValue::Travel(length) => length.to_string(),
        };
        let amount_text =
            |value: DriveValue| named.drive.amount_of(value).map(text).unwrap_or_default();
        let assembly_name = self
            .model
            .assemblies
            .get(assembly.index())
            .map(|a| a.name.clone())
            .unwrap_or_default();
        let name = match &named.name {
            Some(drive_name) if mechanism.drives().len() > 1 => {
                format!("{assembly_name}.{drive_name}")
            }
            Some(_) | None => assembly_name,
        };
        let sets = match set.every() {
            Some(step) => format!("it sets every {}", text(step)),
            None => {
                let values: Vec<String> = named.set_values().map(amount_text).collect();
                format!("it sets at {}", and_list(&values, "and"))
            }
        };
        let nearest: Vec<String> = {
            let (below, above) = named.nearest_set(at);
            below.into_iter().chain(above).map(amount_text).collect()
        };
        Err(self
            .error(
                Code::Motion,
                format!("{name} can't be set at {}: {sets}", text(amount)),
                file,
                span,
                format!("pose {name} at {}", and_list(&nearest, "or")),
            )
            .into())
    }

    /// A kind's mechanism for one piece of hardware, every value worked
    /// out exactly in the plane it moves in: the two frame directions
    /// other than the one its pins run along (`pins:`, or by default
    /// `along`, or `across` when a rail runs `along`), in the order a
    /// positive turn carries the first toward the second.
    pub(crate) fn kind_mechanism(
        &mut self,
        binding: &Binding,
        kind: &KindDef,
        kind_name: &str,
        def: &MechanismDef,
        frame: &Frame,
    ) -> Solved<HardwareMechanism> {
        let file = kind.decl.file;
        let rails: Vec<FrameAxis> = def
            .constraints
            .iter()
            .filter_map(|constraint| match constraint {
                ConstraintDef::Rail { along, .. } => Some(*along),
                ConstraintDef::Pivot { .. } | ConstraintDef::Slot { .. } => None,
            })
            .collect();
        let runs_along = rails.contains(&FrameAxis::Along);
        let pins = match def.pins {
            Some(pins) => pins,
            None if runs_along => FrameAxis::Across,
            None => FrameAxis::Along,
        };
        let [first, second] = pins.plane();
        // A rail along pins that are written is reported where it is
        // lowered; left out, rails along `along` and `across` are reported
        // here.
        if def.pins.is_none() && rails.contains(&pins) {
            return Err(self.error(
                Code::Mechanism,
                format!(
                    "{kind_name}'s mechanism has rails along {} and along {}: it doesn't move in one plane",
                    FrameAxis::Along.word(),
                    pins.word()
                ),
                file,
                def.span,
                "a mechanism moves in one plane: run its rails along depth and one of along or across",
            ).into());
        }
        let plane_point = |point: ExactPoint| -> PlanePoint { [point.on(first), point.on(second)] };
        let direction = |axis: FrameAxis| -> PlanePoint {
            let one = Length::from_nm(1).to_exact();
            if axis == first {
                [one, ExactLength::ZERO]
            } else {
                [ExactLength::ZERO, one]
            }
        };
        let evaluated = self.in_kind(binding, kind, |solver, env| {
            let mut entries = Vec::new();
            for constraint in &def.constraints {
                entries.push(match constraint {
                    ConstraintDef::Pivot { bodies, at, .. } => Entry::Pivot {
                        bodies: *bodies,
                        at: plane_point(solver.kind_point(at, env)?),
                    },
                    ConstraintDef::Slot {
                        bodies, from, to, ..
                    } => Entry::Slot {
                        bodies: *bodies,
                        from: plane_point(solver.kind_point(from, env)?),
                        to: plane_point(solver.kind_point(to, env)?),
                    },
                    ConstraintDef::Rail { bodies, along, .. } => Entry::Rail {
                        bodies: *bodies,
                        direction: direction(*along),
                    },
                });
            }
            for coupling in &def.couplings {
                #[expect(clippy::wildcard_enum_match_arm, reason = "takes only the values it names: any other, a new variant included, is a type error")]
                let ratio = match solver.eval(&coupling.ratio, env)? {
                    Value::Ratio(ratio) | Value::Count(ratio) => ratio,
                    _ => return Err(Fault::Earlier.into()),
                };
                entries.push(Entry::Couple {
                    bodies: EntryPair {
                        member: coupling.body,
                        other: EntrySide::Member(coupling.to),
                    },
                    ratio,
                });
            }
            let mut drives = Vec::with_capacity(def.drives.len());
            for named in &def.drives {
                let turns = named.drive.turns();
                let drive = match &named.drive {
                    DriveDef::Turn { body, from, to, .. } => Drive::Turn {
                        body: *body,
                        from: solver.kind_angle(from, env)?,
                        to: solver.kind_angle(to, env)?,
                    },
                    DriveDef::Travel { body, from, to, .. } => Drive::Travel {
                        body: *body,
                        from: solver.kind_length(from, env)?,
                        to: solver.kind_length(to, env)?,
                    },
                };
                let mut amount = |expr: &stave_model::defs::RExpr| -> Solved<Rational> {
                    Ok(if turns {
                        solver.kind_angle(expr, env)?.value()
                    } else {
                        solver.kind_length(expr, env)?.value()
                    })
                };
                let written = match &named.set_at {
                    Some(SetAtDef::List { values, span }) => {
                        let mut exact = Vec::with_capacity(values.len());
                        for value in values.iter() {
                            exact.push(amount(value)?);
                        }
                        Some(WrittenSetAt::List(exact, *span))
                    }
                    Some(SetAtDef::Every { step, .. }) => {
                        Some(WrittenSetAt::Every(amount(step)?, step.span))
                    }
                    None => None,
                };
                drives.push((named.name.clone(), drive, written));
            }
            let stop = match &def.stop {
                Some(stop) => Some((solver.eval(&stop.value, env)?, stop)),
                None => None,
            };
            Ok((entries, drives, stop))
        })?;
        let (entries, written_drives, stop) = evaluated;
        let mut drives = Vec::with_capacity(written_drives.len());
        for (name, drive, written) in written_drives {
            let set_at = match written {
                Some(written) => {
                    Some(self.set_positions(kind_name, file, name.as_deref(), &drive, written)?)
                }
                None => None,
            };
            drives.push(KindDrive {
                name,
                drive,
                set_at,
            });
        }
        let drive = drives.first().map(|named| named.drive);
        let stop = match stop {
            None => None,
            Some((value, stop)) => {
                #[expect(
                    clippy::wildcard_enum_match_arm,
                    reason = "takes only the values it names: any other, a new variant included, is a type error"
                )]
                let at = match value {
                    Value::Angle(angle) => StopValue::Angle(angle),
                    Value::Length(length) => StopValue::Length(length),
                    _ => {
                        return Err(self.error(
                            Code::TypeMismatch,
                            format!("kind '{kind_name}': a stop is an angle or a length"),
                            file,
                            stop.span,
                            "write stop: 100deg \"...\" for something that turns, stop: 300mm \"...\" for something that slides",
                        ).into());
                    }
                };
                // The drive goes no further than what stops it.
                let beyond =
                    match (drive, at) {
                        (Some(Drive::Turn { from, to, .. }), StopValue::Angle(at)) => {
                            if to >= from { to > at } else { to < at }
                        }
                        (Some(Drive::Travel { from, to, .. }), StopValue::Length(at)) => {
                            if to >= from { to > at } else { to < at }
                        }
                        _ => false,
                    };
                if beyond {
                    return Err(self
                        .error(
                            Code::Motion,
                            format!(
                                "kind '{kind_name}' is driven past its stop: {}",
                                stop.message.literal()
                            ),
                            file,
                            stop.span,
                            "make the drive's to: no more than the stop",
                        )
                        .into());
                }
                Some((at, stop.message.literal()))
            }
        };
        let members = kind
            .bodies
            .iter()
            .enumerate()
            .filter_map(|(index, body)| {
                Some(KindMember {
                    body: BodyId::new(index)?,
                    name: body.name.as_str().to_owned(),
                    with: match &body.with {
                        Carrier::Role(role) => MemberWith::Role(*role),
                        Carrier::Body(other) => MemberWith::Member(*other),
                        Carrier::Mechanism => MemberWith::Placed,
                        // A joint's member: never in a hardware kind.
                        Carrier::Contact(_) => return None,
                    },
                })
            })
            .collect();
        Ok(HardwareMechanism {
            kind: binding.kind,
            plane: Plane::of_frame(frame, pins),
            members,
            roles: kind.roles.iter().map(|role| role.name.clone()).collect(),
            entries,
            drives,
            stop,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use stave_lang::units::Angle;
    use stave_model::mechanism::MotionShape;

    fn body(index: usize) -> BodyId {
        BodyId::new(index).unwrap()
    }

    fn pair(a: usize, b: usize) -> EntryPair {
        EntryPair {
            member: body(a),
            other: EntrySide::Member(body(b)),
        }
    }

    fn mm(value: i64) -> ExactLength {
        Length::from_mm(value).to_exact()
    }

    fn plane(origin: [i64; 3], across: Direction, depth: Direction) -> Plane {
        Plane {
            origin: {
                let [across, up, deep] = origin.map(mm);
                PerAxis { across, up, deep }
            },
            axis: Axis::Up,
            across,
            depth,
        }
    }

    /// A cup hinge's kind: the arm `with: on` (role 1), the cup
    /// `with: piece` (role 0, driven), two links placed by the mechanism.
    fn cup_hinge(at: Plane) -> HardwareMechanism {
        let member = |index: usize, name: &str, with| KindMember {
            body: body(index),
            name: name.to_owned(),
            with,
        };
        let pivot = |a: usize, b: usize, across: i64, depth: i64| Entry::Pivot {
            bodies: pair(a, b),
            at: [mm(across), mm(depth)],
        };
        HardwareMechanism {
            kind: KindId::from_usize(0),
            plane: at,
            members: vec![
                member(0, "arm", MemberWith::Role(RoleIndex::new(1).unwrap())),
                member(1, "cup", MemberWith::Role(RoleIndex::new(0).unwrap())),
                member(2, "inner_link", MemberWith::Placed),
                member(3, "outer_link", MemberWith::Placed),
            ],
            roles: vec!["piece".to_owned(), "on".to_owned()],
            entries: vec![
                pivot(0, 2, 13, 11),
                pivot(1, 2, -9, 7),
                pivot(0, 3, 13, 4),
                pivot(1, 3, -10, 1),
            ],
            drives: vec![KindDrive::unnamed(Drive::Turn {
                body: body(1),
                from: ExactAngle::ZERO,
                to: Angle::from_deg(110).to_exact(),
            })],
            stop: None,
        }
    }

    fn door() -> Plane {
        plane([0, 0, 19], Direction::Right, Direction::Front)
    }

    #[test]
    fn a_cup_hinge_is_a_driven_member_hung_on_two_links() {
        let mechanism = cup_hinge(door());
        let piece_moves = |role: RoleIndex| role.index() == 0;
        let Ok(declared) = declare(
            HardwareId::from_usize(0),
            &mechanism,
            &piece_moves,
            &door(),
            None,
            0,
        ) else {
            panic!("declared");
        };
        // The driven cup first, then the held arm and the two free links.
        let holdings: Vec<(&str, Holding)> = declared
            .members
            .iter()
            .map(|m| (m.name.as_str(), m.holding))
            .collect();
        assert_eq!(
            holdings,
            [
                ("cup", Holding::Free),
                ("arm", Holding::Held),
                ("inner_link", Holding::Free),
                ("outer_link", Holding::Free),
            ]
        );
        assert_eq!(freedoms(&declared.members, &declared.entries), Some(1));
        assert!(unreachable(&declared.members, &declared.entries).is_empty());
    }

    #[test]
    fn a_member_nothing_reaches_has_no_order() {
        let mut mechanism = cup_hinge(door());
        // Two more links pinned only to each other.
        for (index, name) in [(4, "loose"), (5, "looser")] {
            mechanism.members.push(KindMember {
                body: body(index),
                name: name.to_owned(),
                with: MemberWith::Placed,
            });
        }
        mechanism.entries.push(Entry::Pivot {
            bodies: pair(4, 5),
            at: [mm(1), mm(1)],
        });
        let piece_moves = |role: RoleIndex| role.index() == 0;
        let Ok(declared) = declare(
            HardwareId::from_usize(0),
            &mechanism,
            &piece_moves,
            &door(),
            None,
            0,
        ) else {
            panic!("declared");
        };
        assert_eq!(
            unreachable(&declared.members, &declared.entries),
            ["loose", "looser"]
        );
    }

    #[test]
    fn a_second_hinge_is_seen_in_the_first_ones_plane() {
        // The same hinge 300mm higher up the door: its points are the
        // first one's (along the pins carries no meaning).
        let lower = door();
        let upper = plane([0, 300, 19], Direction::Right, Direction::Front);
        let point = [mm(13), mm(11)];
        assert_eq!(lower.point_of(&upper, point), Some(point));
        assert_eq!(lower.same_hand(&upper), Some(true));
        // A mirrored hinge's plane turns the other way round.
        let mirrored = upper.reflected(Axis::Across, mm(250)).unwrap();
        assert_eq!(mirrored.across, Direction::Left);
        assert_eq!(lower.same_hand(&mirrored), Some(false));
        assert_eq!(
            lower.point_of(&mirrored, point),
            Some([mm(500 - 13), mm(11)])
        );
    }

    #[test]
    fn a_pivot_mirrored_along_its_own_pins_moves_alike() {
        // Two pivot bolts on one frame, one through each upright, their
        // pins on one line running across: the right one is the left one
        // reflected across the middle of the frame, along the pins. The
        // reflection leaves the plane's own directions alone, so the two
        // are seen at the same points and turn the same way round.
        let left = Plane {
            origin: PerAxis {
                across: mm(0),
                up: mm(1400),
                deep: mm(200),
            },
            axis: Axis::Across,
            across: Direction::Up,
            depth: Direction::Back,
        };
        let right = left.reflected(Axis::Across, mm(575)).unwrap();
        assert_eq!(right.axis, Axis::Across);
        assert_eq!(left.same_hand(&right), Some(true));
        let point = [mm(-150), mm(0)];
        assert_eq!(left.point_of(&right, point), Some(point));
        assert_eq!(left.vector_of(&right, point), Some(point));
    }

    /// A frame on the left face of an upright standing up, its width
    /// running back: `along` up, `across` back, `depth` right, into it.
    fn upright_frame() -> Frame {
        Frame {
            origin: PerAxis {
                across: mm(0),
                up: mm(850),
                deep: mm(200),
            },
            along: Direction::Up,
            across: Direction::Back,
            depth: Direction::Right,
            across_edge: None,
        }
    }

    #[test]
    fn pins_choose_the_plane_and_the_way_a_turn_goes_round() {
        let frame = upright_frame();
        // Each `pins:` value: the pins' world axis, then the plane's first
        // and second directions, a positive turn carrying the first toward
        // the second, cyclically along, across, depth.
        let cases = [
            (
                FrameAxis::Along,
                Axis::Up,
                Direction::Back,
                Direction::Right,
            ),
            (
                FrameAxis::Across,
                Axis::Deep,
                Direction::Right,
                Direction::Up,
            ),
            (
                FrameAxis::Depth,
                Axis::Across,
                Direction::Up,
                Direction::Back,
            ),
        ];
        for (pins, axis, first, second) in cases {
            let plane = Plane::of_frame(&frame, pins);
            assert_eq!(
                (plane.axis, plane.across, plane.depth),
                (axis, first, second),
                "pins along {}",
                pins.word()
            );
            assert_eq!(plane.origin, frame.origin);
            assert!(plane.frame().is_some(), "pins along {}", pins.word());
        }
    }

    #[test]
    fn a_pivot_bolt_into_the_face_turns_its_frame_along_toward_across() {
        // A bolt through the upright (pins along depth), pinned to the
        // upright itself (a role, held) and driven a quarter turn: the
        // frame turns about the bolt's line, from up toward back.
        let plane = Plane::of_frame(&upright_frame(), FrameAxis::Depth);
        let piece = RoleIndex::new(0).unwrap();
        let into = RoleIndex::new(1).unwrap();
        let mechanism = HardwareMechanism {
            kind: KindId::from_usize(0),
            plane,
            members: vec![KindMember {
                body: body(0),
                name: "bolt".to_owned(),
                with: MemberWith::Role(into),
            }],
            roles: vec!["piece".to_owned(), "into".to_owned()],
            entries: vec![Entry::Pivot {
                bodies: EntryPair {
                    member: body(0),
                    other: EntrySide::Role(piece),
                },
                at: [mm(0), mm(0)],
            }],
            drives: vec![KindDrive::unnamed(Drive::Turn {
                body: body(0),
                from: ExactAngle::ZERO,
                to: Angle::from_deg(90).to_exact(),
            })],
            stop: None,
        };
        let into_moves = |role: RoleIndex| role == into;
        let Ok(declared) = declare(
            HardwareId::from_usize(0),
            &mechanism,
            &into_moves,
            &plane,
            None,
            0,
        ) else {
            panic!("declared");
        };
        // The driven bolt, then the upright's part, held: one freedom.
        let holdings: Vec<(&str, Holding, MemberBody)> = declared
            .members
            .iter()
            .map(|m| (m.name.as_str(), m.holding, m.body))
            .collect();
        assert_eq!(
            holdings,
            [
                ("bolt", Holding::Free, MemberBody::Body(body(0))),
                ("piece", Holding::Held, MemberBody::Role { role: piece }),
            ]
        );
        assert_eq!(freedoms(&declared.members, &declared.entries), Some(1));
        assert!(unreachable(&declared.members, &declared.entries).is_empty());
        let counted = Mechanism::new(
            plane.frame().unwrap(),
            declared.members,
            declared.entries,
            declared.drives.into_iter().next().unwrap().drive,
            None,
            vec![HardwareId::from_usize(0)],
        )
        .unwrap();
        let MotionShape::Turn { line, opening } = counted.shape() else {
            panic!("a turn about the bolt");
        };
        assert_eq!(line.axis, Axis::Across);
        assert_eq!(
            (line.free_side, line.swing_toward),
            (Direction::Up, Direction::Back)
        );
        assert_eq!(opening, Angle::from_deg(90));
    }

    #[test]
    fn a_role_that_moves_rides_on_the_driven_member() {
        // The same bolt pinned to a role whose part moves with the
        // assembly: the part rides on the bolt, one rigid piece with it.
        let plane = Plane::of_frame(&upright_frame(), FrameAxis::Depth);
        let into = RoleIndex::new(1).unwrap();
        let mechanism = HardwareMechanism {
            kind: KindId::from_usize(0),
            plane,
            members: vec![KindMember {
                body: body(0),
                name: "bolt".to_owned(),
                with: MemberWith::Role(into),
            }],
            roles: vec!["piece".to_owned(), "into".to_owned()],
            entries: vec![Entry::Pivot {
                bodies: EntryPair {
                    member: body(0),
                    other: EntrySide::Role(into),
                },
                at: [mm(0), mm(0)],
            }],
            drives: vec![KindDrive::unnamed(Drive::Turn {
                body: body(0),
                from: ExactAngle::ZERO,
                to: Angle::from_deg(90).to_exact(),
            })],
            stop: None,
        };
        let into_moves = |role: RoleIndex| role == into;
        let Ok(declared) = declare(
            HardwareId::from_usize(0),
            &mechanism,
            &into_moves,
            &plane,
            None,
            0,
        ) else {
            panic!("declared");
        };
        let driven = MemberIndex::new(0).unwrap();
        assert_eq!(
            declared.members.get(1).map(|m| m.holding),
            Some(Holding::Carried(driven))
        );
        // A pin between the bolt and what rides on it holds nothing.
        assert!(matches!(
            Mechanism::new(
                plane.frame().unwrap(),
                declared.members,
                declared.entries,
                declared.drives.into_iter().next().unwrap().drive,
                None,
                vec![HardwareId::from_usize(0)],
            ),
            Err(MechanismError::OnePiece { .. })
        ));
    }

    #[test]
    fn two_pivot_bolts_on_one_line_through_facing_uprights_move_alike() {
        // One bolt on the left face of the left upright, the other on the
        // right face of the right upright, each placed by its own mount:
        // their depths point opposite ways, but their pins lie on one
        // line, so they are seen at the same points and turn the same way.
        let left = Plane::of_frame(&upright_frame(), FrameAxis::Depth);
        let right = Plane::of_frame(
            &Frame {
                origin: PerAxis {
                    across: mm(700),
                    up: mm(850),
                    deep: mm(200),
                },
                depth: Direction::Left,
                ..upright_frame()
            },
            FrameAxis::Depth,
        );
        assert_eq!(left.axis, right.axis);
        assert_eq!(left.same_hand(&right), Some(true));
        let point = [mm(0), mm(0)];
        assert_eq!(left.point_of(&right, point), Some(point));
    }

    #[test]
    fn a_drive_narrows_to_the_nearer_end() {
        let drive = MechanismDrive::Turn {
            member: MemberIndex::new(0).unwrap(),
            from: Angle::ZERO,
            to: Angle::from_deg(110),
        };
        let narrowed = narrower(&drive, DriveValue::Turn(Angle::from_deg(95))).unwrap();
        assert_eq!(narrowed.to(), DriveValue::Turn(Angle::from_deg(95)));
        assert_eq!(
            narrower(&drive, DriveValue::Turn(Angle::from_deg(120))),
            None
        );
    }
}
