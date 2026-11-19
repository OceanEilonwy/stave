//! Mechanisms (grammar, "Mechanisms" and "How a mechanism is solved"): a
//! hardware kind's `mechanism { }` evaluated exactly for one piece of
//! hardware ([`HardwareMechanism`]), and the one mechanism of an assembly
//! made from every piece of hardware on it (the rule for several items).
//!
//! Nothing here knows a kind, a base or a member by name. A piece of
//! hardware's mechanism is counted on its own as soon as it is placed (one
//! freedom for a mechanism with a drive, none for one that follows), and
//! reported once per kind. An assembly's mechanism is then the first
//! driving item's, every other driving item must move alike (the same
//! members, entries and drive, seen in the first one's plane), and the
//! followers' members and entries are added. The exact
//! [`stave_model::mechanism::Mechanism`] is what the solved design holds;
//! the order of steps and the poses are worked out in `stave-geom`.

use std::collections::BTreeMap;

use stave_lang::placement::{Axis, Direction};
use stave_lang::units::{ExactAngle, ExactLength, Length, Rational};
use stave_lang::{Code, Diagnostic, Label};
use stave_model::defs::{
    BodyId, Carrier, ConstraintDef, DriveDef, FrameAxis, KindDef, MechanismDef, RoleIndex,
};
use stave_model::ids::{AssemblyId, HardwareId, KindId};
use stave_model::mechanism::{
    DriveValue, FrameDirection, FramePoint, Holding, Mechanism, MechanismDrive, MechanismEntry,
    MechanismError, MechanismFrame, MechanismMember, MechanismStop, MemberIndex,
};
use stave_model::solved::Point;
use stave_model::value::Value;

use crate::hardware::{Frame, Placed, RoleBound, world_index};
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
    pub origin: [ExactLength; 3],
    pub axis: Axis,
    pub across: Direction,
    pub depth: Direction,
}

impl Plane {
    /// The plane reflected across the world plane at `plane` on `axis`.
    pub fn reflected(&self, axis: Axis, plane: ExactLength) -> Option<Plane> {
        let mut origin = self.origin;
        if let Some(slot) = origin.get_mut(world_index(axis)) {
            *slot = plane.checked_add(plane)?.checked_sub(*slot)?;
        }
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
    fn world(&self, [across, depth]: PlanePoint) -> Option<[ExactLength; 3]> {
        let mut out = [ExactLength::ZERO; 3];
        for (direction, amount) in [(self.across, across), (self.depth, depth)] {
            let slot = out.get_mut(world_index(direction.axis()))?;
            let signed = if direction.is_positive() {
                amount
            } else {
                amount.checked_neg()?
            };
            *slot = slot.checked_add(signed)?;
        }
        Some(out)
    }

    /// A world vector's coordinates in the plane.
    fn coordinates(&self, vector: [ExactLength; 3]) -> Option<PlanePoint> {
        let along = |direction: Direction| -> Option<ExactLength> {
            let value = *vector.get(world_index(direction.axis()))?;
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
        let mut vector = [ExactLength::ZERO; 3];
        for (index, slot) in vector.iter_mut().enumerate() {
            let from = other.origin.get(index)?.checked_add(*offset.get(index)?)?;
            *slot = from.checked_sub(*self.origin.get(index)?)?;
        }
        // Along the pins the coordinate carries no meaning.
        if let Some(slot) = vector.get_mut(world_index(self.axis)) {
            *slot = ExactLength::ZERO;
        }
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
        let near = |index: usize| self.origin.get(index)?.to_nearest_length();
        let origin = Point {
            across: near(0)?,
            up: near(1)?,
            deep: near(2)?,
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

/// An entry of a kind's mechanism, exact, in its plane.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Entry {
    Pivot {
        bodies: [BodyId; 2],
        at: PlanePoint,
    },
    Slot {
        bodies: [BodyId; 2],
        from: PlanePoint,
        to: PlanePoint,
    },
    Rail {
        bodies: [BodyId; 2],
        direction: PlanePoint,
    },
    Couple {
        bodies: [BodyId; 2],
        ratio: Rational,
    },
}

impl Entry {
    fn bodies(&self) -> [BodyId; 2] {
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
    pub entries: Vec<Entry>,
    /// `None` for a mechanism that follows its assembly's.
    pub drive: Option<Drive>,
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
    drive: Option<MechanismDrive>,
    stop: Option<MechanismStop>,
    bodies: BTreeMap<BodyId, MemberIndex>,
}

/// Why a piece of hardware's mechanism can't be declared.
enum DeclareError {
    /// A number too large, or a body that isn't a member (never a design's).
    Internal,
    /// The drive moves a member that isn't `with:` a part that moves.
    DriveNotCarried(String),
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
            body: member.body,
            name: member.name.clone(),
            holding,
        });
        bodies.insert(member.body, at);
        Ok(at)
    };
    // The driven member first: what the assembly moves with.
    let own_drive = mechanism.drive.map(|drive| drive.body());
    let driven = match own_drive {
        Some(body) => {
            let member = mechanism.member(body).ok_or(DeclareError::Internal)?;
            match member.with {
                MemberWith::Role(role) if moving(role) => {}
                MemberWith::Role(_) | MemberWith::Member(_) | MemberWith::Placed => {
                    return Err(DeclareError::DriveNotCarried(member.name.clone()));
                }
            }
            Some(push(&mut members, &mut bodies, member, Holding::Free)?)
        }
        None => driven,
    };
    // Then every other member once what it rides on is declared.
    let mut left: Vec<&KindMember> = mechanism
        .members
        .iter()
        .filter(|member| Some(member.body) != own_drive)
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
    let to = |body: &BodyId| bodies.get(body).copied().ok_or(DeclareError::Internal);
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
        let [a, b] = entry.bodies();
        let (a, b) = (to(&a)?, to(&b)?);
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
    let drive = match mechanism.drive {
        Some(Drive::Turn {
            body,
            from,
            to: end,
        }) => Some(MechanismDrive::Turn {
            member: to(&body)?,
            from: angle(from)?,
            to: angle(end)?,
        }),
        Some(Drive::Travel {
            body,
            from,
            to: end,
        }) => Some(MechanismDrive::Travel {
            member: to(&body)?,
            from: length(from)?,
            to: length(end)?,
        }),
        None => None,
    };
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
        drive,
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
    /// the piece, and every role bound to something inside it.
    fn moving_roles(
        &self,
        roles: &[(String, RoleBound)],
        assembly: Option<AssemblyId>,
    ) -> Vec<bool> {
        roles
            .iter()
            .enumerate()
            .map(|(index, (_, bound))| {
                index == 0
                    || assembly.is_some_and(|assembly| {
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
                    })
            })
            .collect()
    }

    /// Where a kind's mechanism is written.
    fn mechanism_place(&self, kind: KindId) -> Option<(stave_lang::FileId, stave_lang::ast::Span)> {
        let def: &KindDef = self.model.defs.kind(kind)?;
        Some((def.decl.file, def.mechanism()?.span))
    }

    /// Where a piece of hardware is declared.
    fn hardware_call(
        &self,
        hardware: HardwareId,
    ) -> Option<(stave_lang::FileId, stave_lang::ast::Span)> {
        let inst = self.model.hardware.get(hardware.index())?;
        let def = self.model.defs.hardware_def(inst.def)?;
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
        let wanted = if declared.drive.is_some() { 1 } else { 0 };
        if found != wanted {
            let (message, help) = if declared.drive.is_some() {
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
        let Some(drive) = declared.drive else {
            return Ok(None);
        };
        let frame = mechanism
            .plane
            .frame()
            .ok_or(Fault::Internal("a mechanism's plane"))?;
        match Mechanism::new(
            frame,
            declared.members,
            declared.entries,
            drive,
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
            DeclareError::DriveNotCarried(member) => self.mechanism_error(
                kind,
                format!(
                    "{kind_name}'s drive moves '{member}', which isn't with: a part that moves"
                ),
                None,
                "drive the member that is with: the piece (the door, the lid, the drawer side)",
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
            MechanismError::DriveNotFree { .. } => {
                "drive the member that is with: the piece (the door, the lid, the drawer side)"
            }
            MechanismError::EmptyRange => "give the drive a from: and a to: that differ",
            // Checked when the kind is resolved.
            MechanismError::UnknownMember { .. } | MechanismError::NoMembers => {
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
            .partition(|(_, _, mechanism)| mechanism.drive.is_some());
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
        let moving = self.moving_roles(&first_placed.roles, Some(assembly));
        let is_moving = |role: RoleIndex| moving.get(role.index()).copied().unwrap_or(false);
        // Each piece of hardware's own mechanism was checked when placed.
        let Ok(mut declared) = declare(first, base, &is_moving, &base.plane, None, 0) else {
            return Err(Fault::Earlier.into());
        };
        let Some(mut drive) = declared.drive else {
            return Err(Fault::Internal("the drive of a mechanism").into());
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
            let start = |d: Option<MechanismDrive>| d.map(|d| (d.member(), d.from(), d.turns()));
            let alike = other
                .members
                .iter()
                .map(|m| (&m.name, m.holding))
                .eq(declared.members.iter().map(|m| (&m.name, m.holding)))
                && other.entries == declared.entries
                && start(other.drive) == start(Some(drive));
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
            if let Some(other_drive) = other.drive
                && let Some(narrower) = narrower(&drive, other_drive.to())
            {
                drive = narrower;
            }
            if let Some(other_stop) = other.stop {
                stop = Some(match stop {
                    Some(current)
                        if nearer(drive.from(), current.at, other_stop.at) == current.at =>
                    {
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
            let Ok(follower) = declare(
                id,
                mechanism,
                &is_moving,
                &base.plane,
                Some(drive.member()),
                offset,
            ) else {
                return Err(Fault::Earlier.into());
            };
            if let Some(follower_stop) = follower.stop {
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
                if let Some(narrower) = narrower(&drive, at) {
                    drive = narrower;
                }
                stop = Some(match stop {
                    Some(current) if nearer(drive.from(), current.at, at) == current.at => current,
                    Some(_) | None => MechanismStop {
                        at,
                        message: follower_stop.message,
                    },
                });
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
        match Mechanism::new(
            frame,
            declared.members,
            declared.entries,
            drive,
            stop,
            hardware,
        ) {
            Ok(mechanism) => Ok(Some(AssemblyMechanism { mechanism, bodies })),
            Err(error) => {
                let kind_name = self
                    .model
                    .defs
                    .kind(base.kind)
                    .map(|k| k.decl.name.clone())
                    .unwrap_or_default();
                Err(self.counted_error(base.kind, &kind_name, &error))
            }
        }
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
    /// A scene's pose of an assembly asks no more of its mechanism than it
    /// reaches: an angle or a travel past it is `error[motion]`, with the
    /// stop's message when a `stop` ends the range.
    pub(crate) fn pose_within(
        &mut self,
        assembly: AssemblyId,
        mechanism: &Mechanism,
        value: stave_model::solved::PoseValue,
        file: stave_lang::FileId,
        span: stave_lang::ast::Span,
    ) -> Solved<()> {
        use stave_model::solved::PoseValue;
        let extent = mechanism.drive().extent();
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
        let name = self
            .model
            .assemblies
            .get(assembly.index())
            .map(|a| a.name.clone())
            .unwrap_or_default();
        let why = match mechanism.stop() {
            Some(stop) if stop.at == mechanism.drive().to() => stop.message.clone(),
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

    /// A kind's mechanism for one piece of hardware, every value worked
    /// out exactly in the plane it moves in: across/depth with the pins
    /// along `along`, or along/depth (pins across) when a rail runs
    /// `along`.
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
        let (first, second, pins) = if runs_along {
            (FrameAxis::Along, FrameAxis::Depth, FrameAxis::Across)
        } else {
            (FrameAxis::Across, FrameAxis::Depth, FrameAxis::Along)
        };
        if rails.contains(&pins) {
            return Err(self.error(
                Code::Mechanism,
                format!(
                    "{kind_name}'s mechanism has rails along {} and along {}: it doesn't move in one plane",
                    first.word(),
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
                let ratio = match solver.eval(&coupling.ratio, env)? {
                    Value::Ratio(ratio) | Value::Count(ratio) => ratio,
                    _ => return Err(Fault::Earlier.into()),
                };
                entries.push(Entry::Couple {
                    bodies: [coupling.body, coupling.to],
                    ratio,
                });
            }
            let drive = match &def.drive {
                Some(DriveDef::Turn { body, from, to, .. }) => Some(Drive::Turn {
                    body: *body,
                    from: solver.kind_angle(from, env)?,
                    to: solver.kind_angle(to, env)?,
                }),
                Some(DriveDef::Travel { body, from, to, .. }) => Some(Drive::Travel {
                    body: *body,
                    from: solver.kind_length(from, env)?,
                    to: solver.kind_length(to, env)?,
                }),
                None => None,
            };
            let stop = match &def.stop {
                Some(stop) => Some((solver.eval(&stop.value, env)?, stop)),
                None => None,
            };
            Ok((entries, drive, stop))
        })?;
        let (entries, drive, stop) = evaluated;
        let stop = match stop {
            None => None,
            Some((value, stop)) => {
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
            plane: Plane {
                origin: frame.origin,
                axis: frame.direction(pins).axis(),
                across: frame.direction(first),
                depth: frame.direction(second),
            },
            members,
            entries,
            drive,
            stop,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use stave_lang::units::Angle;

    fn body(index: usize) -> BodyId {
        BodyId::new(index).unwrap()
    }

    fn mm(value: i64) -> ExactLength {
        Length::from_mm(value).to_exact()
    }

    fn plane(origin: [i64; 3], across: Direction, depth: Direction) -> Plane {
        Plane {
            origin: origin.map(mm),
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
            bodies: [body(a), body(b)],
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
            entries: vec![
                pivot(0, 2, 13, 11),
                pivot(1, 2, -9, 7),
                pivot(0, 3, 13, 4),
                pivot(1, 3, -10, 1),
            ],
            drive: Some(Drive::Turn {
                body: body(1),
                from: ExactAngle::ZERO,
                to: Angle::from_deg(110).to_exact(),
            }),
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
            bodies: [body(4), body(5)],
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
