//! An assembly's mechanism at the geometry boundary (grammar, "How a
//! mechanism is solved"): the exact [`Mechanism`] of the solved design run
//! through its range by the general planar solver ([`crate::planar`]), and
//! every member's pose as a world transform.
//!
//! A [`MechanismPath`] is built once per assembly, and once more for each
//! drive and each set of values the other drives are held at
//! ([`MechanismPath::sweeping`]): it runs one drive (the **swept** one)
//! through its range with every other drive held still. The swept drive is
//! measured from its `from` as an **amount**: radians turned or nanometres
//! travelled from `from` toward `to`, from zero to [`MechanismPath::limit`],
//! so a sweep starts at zero whichever way the kind's range runs. The
//! assembly moves with the driven member ([`MechanismPath::moving`]). A
//! member's transform is from the closed pose (every drive at its `from`),
//! so with a drive held away from its `from` it isn't the identity at
//! amount zero. [`MechanismPath::matrix_at`] poses every drive at once.
//!
//! Two shapes of motion have exact fast paths, chosen by the mechanism's
//! shape and never by a kind ([`MotionShape`]): a driven member pinned to a held
//! one turns about one fixed line, and a driven member on a rail along a
//! held one travels in one world direction.

use glam::{DMat3, DMat4, DVec2, DVec3};
use stave_lang::placement::{Axis, Direction};
use stave_model::mechanism::{
    DriveIndex, DriveValue, Holding, Mechanism, MechanismDrive, MechanismEntry, MemberIndex,
    MotionShape, NamedDrive,
};
use stave_model::solved::{Point, PoseValue};

use crate::collide::{direction_vector, point_vector};
use crate::planar::{self, Declaration, MechanismError, MemberId, MemberName, PlanarPose};
use crate::render;

/// An assembly's mechanism, checked through its range and ready to pose
/// in floats. See the module docs.
#[derive(Debug, Clone, PartialEq)]
pub struct MechanismPath {
    path: planar::Path,
    /// The solver's id of each member, by its place in the mechanism.
    ids: Vec<MemberId>,
    origin: DVec3,
    x: DVec3,
    y: DVec3,
    normal: DVec3,
    axis: Axis,
    across: Direction,
    depth: Direction,
    moving: MemberIndex,
    /// The exact mechanism it runs.
    model: Mechanism,
    /// Every drive, with its name.
    drives: Vec<NamedDrive>,
    /// The swept drive.
    swept: DriveIndex,
    drive: MechanismDrive,
    /// Every drive's value while the swept one runs (the swept one's is
    /// its `from`).
    held: Vec<DriveValue>,
    /// `true` when every held drive is at its `from`.
    held_closed: bool,
    /// Where the held drives have moved the driven member, when that is a
    /// translation.
    offset: Option<Point>,
    /// `+1` when the range runs up from `from`, `-1` when it runs down.
    sense: f64,
    /// The drive's `from`, in radians or nanometres.
    from: f64,
    /// How far the range reaches from `from`, in radians or nanometres.
    limit: f64,
    shape: MotionShape,
}

impl MechanismPath {
    /// Runs an assembly's mechanism through its range (grammar, "How a
    /// mechanism is solved"). A mechanism with no order of steps is an
    /// error of [`planar::MechanismErrorKind::Mechanism`]; one that locks,
    /// flips or runs a pin off its slot within its range is an error of
    /// [`planar::MechanismErrorKind::Motion`].
    pub fn new(mechanism: &Mechanism) -> Result<MechanismPath, MechanismError> {
        MechanismPath::sweeping(mechanism, DriveIndex::FIRST, &mechanism.closed())
    }

    /// Runs one drive of a mechanism through its range with every other
    /// drive held at `held` (one value for each drive, absolute; the swept
    /// one's is not read), and checks it as [`MechanismPath::new`] does.
    /// A drive the mechanism doesn't have, or held values of the wrong
    /// number or kind, are [`MechanismError::NoSuchDrive`].
    pub fn sweeping(
        mechanism: &Mechanism,
        swept: DriveIndex,
        held: &[DriveValue],
    ) -> Result<MechanismPath, MechanismError> {
        let mut declaration = Declaration::new();
        let mut ids: Vec<MemberId> = Vec::with_capacity(mechanism.members().len());
        let id = |ids: &[MemberId], index: MemberIndex| {
            ids.get(index.index())
                .copied()
                .ok_or(MechanismError::Internal)
        };
        for member in mechanism.members() {
            let name = MemberName::new(member.name.clone()).ok_or(MechanismError::Internal)?;
            let holding = match member.holding {
                Holding::Held => planar::Holding::Held,
                Holding::Free => planar::Holding::Free,
                Holding::Carried(carrier) => planar::Holding::Carried(id(&ids, carrier)?),
            };
            ids.push(declaration.member(name, holding)?);
        }
        let point =
            |p: stave_model::mechanism::FramePoint| planar::PlanarPoint::new(p.across, p.depth);
        for entry in mechanism.entries() {
            declaration.constrain(match *entry {
                MechanismEntry::Pivot { first, second, at } => planar::Constraint::Pivot {
                    first: id(&ids, first)?,
                    second: id(&ids, second)?,
                    at: point(at),
                },
                MechanismEntry::Slot {
                    pin,
                    guide,
                    from,
                    to,
                } => planar::Constraint::Slot {
                    pin: id(&ids, pin)?,
                    guide: id(&ids, guide)?,
                    from: point(from),
                    to: point(to),
                },
                MechanismEntry::Rail {
                    runner,
                    guide,
                    direction,
                } => planar::Constraint::Rail {
                    runner: id(&ids, runner)?,
                    guide: id(&ids, guide)?,
                    direction: planar::PlanarDirection::new(direction.across(), direction.depth())
                        .ok_or(MechanismError::Internal)?,
                },
                MechanismEntry::Couple {
                    follower,
                    leader,
                    ratio,
                } => planar::Constraint::Couple {
                    follower: id(&ids, follower)?,
                    leader: id(&ids, leader)?,
                    ratio: planar::Ratio::new(ratio).ok_or(MechanismError::Internal)?,
                },
            });
        }
        let drive = mechanism
            .drive_at(swept)
            .ok_or(MechanismError::NoSuchDrive)?
            .drive;
        let mut planar_drives = Vec::with_capacity(mechanism.drives().len());
        for named in mechanism.drives().iter() {
            planar_drives.push(match named.drive {
                MechanismDrive::Turn { member, from, to } => planar::Drive::Turn {
                    member: id(&ids, member)?,
                    from,
                    to,
                },
                MechanismDrive::Travel { member, from, to } => planar::Drive::Travel {
                    member: id(&ids, member)?,
                    from,
                    to,
                },
            });
        }
        if held.len() != planar_drives.len() {
            return Err(MechanismError::NoSuchDrive);
        }
        // The swept drive's own value is its `from`.
        let held: Vec<DriveValue> = mechanism
            .drives()
            .iter()
            .zip(held)
            .enumerate()
            .map(|(index, (named, value))| {
                if index == swept.index() {
                    named.drive.from()
                } else {
                    *value
                }
            })
            .collect();
        let planar_held: Vec<planar::DriveValue> = held.iter().map(|v| to_planar(*v)).collect();
        let planned = planar::Mechanism::with_drives(declaration, planar_drives)?
            .sweeping(swept.index(), &planar_held)?;
        let path = planar::Path::new(planned)?;
        let frame = mechanism.frame();
        let x = direction_vector(frame.across());
        let y = direction_vector(frame.depth());
        let amount = |value: DriveValue| planar::DriveAmount::of(to_planar(value)).value();
        let from = amount(drive.from());
        let to = amount(drive.to());
        let sense = if to >= from { 1.0 } else { -1.0 };
        let held_closed = mechanism
            .drives()
            .iter()
            .zip(&held)
            .all(|(named, value)| named.drive.from() == *value);
        Ok(MechanismPath {
            path,
            ids,
            origin: point_vector(frame.origin()),
            x,
            y,
            normal: x.cross(y),
            axis: frame.axis(),
            across: frame.across(),
            depth: frame.depth(),
            moving: mechanism.moving(),
            model: mechanism.clone(),
            drives: mechanism.drives().iter().cloned().collect(),
            swept,
            drive,
            offset: mechanism.offset_at(&held),
            shape: mechanism.shape_of(swept, &held),
            held,
            held_closed,
            sense,
            from,
            limit: (to - from).abs(),
        })
    }

    /// Every drive of the mechanism, with its name: one for most.
    pub fn drives(&self) -> &[NamedDrive] {
        &self.drives
    }

    /// The exact mechanism it runs.
    pub fn model(&self) -> &Mechanism {
        &self.model
    }

    /// The drive run through its range.
    pub fn swept(&self) -> DriveIndex {
        self.swept
    }

    /// Every drive's value while the swept one runs (the swept one's is
    /// its `from`).
    pub fn held(&self) -> &[DriveValue] {
        &self.held
    }

    /// `true` when every other drive is held at its `from`, so the swept
    /// drive starts from the closed pose.
    pub fn held_closed(&self) -> bool {
        self.held_closed
    }

    /// Where the held drives have moved the driven member from the closed
    /// pose, as one world translation; `None` when they turn it (or it
    /// isn't exact). Zero when every other drive is at its `from`.
    pub fn offset(&self) -> Option<Point> {
        self.offset
    }

    /// The exact shape of the swept drive's motion, where it has one: a
    /// turn's line is where the held drives have put it, and the moving
    /// bodies start where [`MechanismPath::offset`] puts them.
    pub fn shape(&self) -> MotionShape {
        self.shape
    }

    /// The member the assembly moves with.
    pub fn moving(&self) -> MemberIndex {
        self.moving
    }

    /// `true` when the drive turns (a door), `false` when it travels (a
    /// drawer).
    pub fn turns(&self) -> bool {
        self.drive.turns()
    }

    /// How far the range reaches from the closed pose: an angle or a
    /// length.
    pub fn extent(&self) -> DriveValue {
        self.drive.extent()
    }

    /// How far the range reaches, in radians or nanometres.
    pub fn limit(&self) -> f64 {
        self.limit
    }

    /// The world axis the pins run along.
    pub fn axis(&self) -> Axis {
        self.axis
    }

    /// For a turning drive: the plane's first direction (from a hinge's
    /// edge toward the middle of the door) and the way it turns as the
    /// drive opens.
    pub fn swing(&self) -> Option<(Direction, Direction)> {
        self.turns().then(|| {
            let toward = if self.sense > 0.0 {
                self.depth
            } else {
                self.depth.opposite()
            };
            (self.across, toward)
        })
    }

    /// The plane's origin and its two unit directions in the world.
    pub(crate) fn plane_basis(&self) -> (DVec3, DVec3, DVec3) {
        (self.origin, self.x, self.y)
    }

    /// The amount of a pose value along the range, clamped to it: `open`
    /// is the whole range (or the furthest set value, for a drive with
    /// set positions), an angle or a travel is measured from the closed
    /// pose; a value of the other kind is the closed pose.
    pub fn amount(&self, value: PoseValue) -> f64 {
        let amount = match (value, self.drive) {
            (PoseValue::Closed, _) => 0.0,
            (PoseValue::Open, _) => self
                .drives
                .get(self.swept().index())
                .map_or(self.limit, open_amount),
            (PoseValue::Angle(angle), MechanismDrive::Turn { .. }) => render::radians_f64(angle),
            (PoseValue::Travel(length), MechanismDrive::Travel { .. }) => render::nm(length),
            (PoseValue::Angle(_), MechanismDrive::Travel { .. })
            | (PoseValue::Travel(_), MechanismDrive::Turn { .. }) => 0.0,
        };
        amount.clamp(0.0, self.limit)
    }

    /// The pose value of an amount: an angle, rounded down to the
    /// micro-degree, or a travel, rounded down to the nanometre.
    pub fn value_of(&self, amount: f64) -> Option<PoseValue> {
        if self.turns() {
            render::angle_from_radians_floor(amount).map(PoseValue::Angle)
        } else {
            render::length_from_nm(amount.floor()).map(PoseValue::Travel)
        }
    }

    /// A member's pose in the plane, `amount` along the range; `None` past
    /// the range (never within it, which [`MechanismPath::new`] checked).
    pub fn planar_pose(&self, member: MemberIndex, amount: f64) -> Option<PlanarPose> {
        if amount == 0.0 && self.held_closed {
            return Some(PlanarPose::REST);
        }
        let value = self.from + self.sense * amount;
        let at = if self.turns() {
            planar::DriveAmount::radians(value)
        } else {
            planar::DriveAmount::nanometres(value)
        };
        let pose = self.path.pose_at(at).ok()?;
        pose.member(*self.ids.get(member.index())?)
    }

    /// A member's world transform (world nanometres to world nanometres),
    /// `amount` along the range.
    pub fn matrix(&self, member: MemberIndex, amount: f64) -> Option<DMat4> {
        if amount == 0.0 && self.held_closed {
            return Some(DMat4::IDENTITY);
        }
        let pose = self.planar_pose(member, amount)?;
        Some(self.world(pose))
    }

    /// The amount of a pose value along one drive's range, clamped to it
    /// (see [`MechanismPath::amount`]); zero for a drive it doesn't have.
    pub fn amount_of(&self, drive: DriveIndex, value: PoseValue) -> f64 {
        let Some(named) = self.drives.get(drive.index()) else {
            return 0.0;
        };
        let limit = match named.drive.extent() {
            DriveValue::Turn(angle) => render::radians_f64(angle),
            DriveValue::Travel(length) => render::nm(length),
        };
        let amount = match (value, named.drive) {
            (PoseValue::Closed, _) => 0.0,
            (PoseValue::Open, _) => open_amount(named),
            (PoseValue::Angle(angle), MechanismDrive::Turn { .. }) => render::radians_f64(angle),
            (PoseValue::Travel(length), MechanismDrive::Travel { .. }) => render::nm(length),
            (PoseValue::Angle(_), MechanismDrive::Travel { .. })
            | (PoseValue::Travel(_), MechanismDrive::Turn { .. }) => 0.0,
        };
        amount.clamp(0.0, limit)
    }

    /// A member's world transform with every drive set at once: `amounts`
    /// is one amount for each drive (radians or nanometres from its
    /// `from` toward its `to`, as [`MechanismPath::amount_of`] gives),
    /// each clamped to its range. The identity when every amount is zero.
    pub fn matrix_at(&self, member: MemberIndex, amounts: &[f64]) -> Option<DMat4> {
        if amounts.iter().all(|amount| *amount == 0.0) {
            return Some(DMat4::IDENTITY);
        }
        let absolute: Vec<planar::DriveAmount> = self
            .drives
            .iter()
            .enumerate()
            .map(|(index, named)| {
                let amount = amounts.get(index).copied().unwrap_or(0.0);
                let from = planar::DriveAmount::of(to_planar(named.drive.from())).value();
                let to = planar::DriveAmount::of(to_planar(named.drive.to())).value();
                let sense = if to >= from { 1.0 } else { -1.0 };
                let value = from + sense * amount.clamp(0.0, (to - from).abs());
                match named.drive {
                    MechanismDrive::Turn { .. } => planar::DriveAmount::radians(value),
                    MechanismDrive::Travel { .. } => planar::DriveAmount::nanometres(value),
                }
            })
            .collect();
        let pose = self.path.pose_all(&absolute).ok()?;
        let pose = pose.member(*self.ids.get(member.index())?)?;
        Some(self.world(pose))
    }

    /// A pose in the plane as a world transform.
    fn world(&self, pose: PlanarPose) -> DMat4 {
        let basis = DMat3::from_cols(self.x, self.y, self.normal);
        let to_world = DMat4::from_translation(self.origin) * DMat4::from_mat3(basis);
        let to_plane = DMat4::from_mat3(basis.transpose()) * DMat4::from_translation(-self.origin);
        let offset = pose.offset();
        let local = DMat4::from_translation(DVec3::new(offset.x, offset.y, 0.0))
            * DMat4::from_rotation_z(pose.angle());
        to_world * local * to_plane
    }

    /// A member's turn and offset in the plane (about the plane's origin),
    /// `amount` along the range.
    pub(crate) fn motion(&self, member: MemberIndex, amount: f64) -> Option<(DVec2, DVec2)> {
        let pose = self.planar_pose(member, amount)?;
        Some((DVec2::from_angle(pose.angle()), pose.offset()))
    }

    /// The fastest any point of a member within `radius` nanometres of the
    /// plane's origin moves, per radian or nanometre of the drive.
    pub fn speed(&self, member: MemberIndex, radius: f64) -> Option<f64> {
        let id = *self.ids.get(member.index())?;
        Some(self.path.speed(id)?.bound(radius))
    }
}

/// Where `open` puts a drive, as an amount from its `from` (radians or
/// nanometres): the whole range, or the furthest set value for a drive
/// with set positions.
fn open_amount(named: &NamedDrive) -> f64 {
    match named.drive.amount_of(named.open()) {
        Some(DriveValue::Turn(angle)) => render::radians_f64(angle),
        Some(DriveValue::Travel(length)) => render::nm(length),
        None => 0.0,
    }
}

/// An exact drive value of the solved design as the solver's.
fn to_planar(value: DriveValue) -> planar::DriveValue {
    match value {
        DriveValue::Turn(angle) => planar::DriveValue::Turn(angle),
        DriveValue::Travel(length) => planar::DriveValue::Travel(length),
    }
}
