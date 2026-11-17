//! An assembly's mechanism at the geometry boundary (grammar, "How a
//! mechanism is solved"): the exact [`Mechanism`] of the solved design run
//! through its range by the general planar solver ([`crate::planar`]), and
//! every member's pose as a world transform.
//!
//! A [`MechanismPath`] is built once per assembly. Its drive is measured
//! from the closed pose as an **amount**: radians turned or nanometres
//! travelled from `from` toward `to`, from zero to [`MechanismPath::limit`],
//! so a sweep starts at zero whichever way the kind's range runs. The
//! assembly moves with the driven member ([`MechanismPath::moving`]).
//!
//! Two shapes of motion have exact fast paths, chosen by the mechanism's
//! shape and never by a kind ([`Shape`]): a driven member pinned to a held
//! one turns about one fixed line, and a driven member on a rail along a
//! held one travels in one world direction.

use glam::{DMat3, DMat4, DVec2, DVec3};
use stave_lang::placement::{Axis, Direction};
use stave_lang::units::{Angle, Length};
use stave_model::mechanism::{
    DriveValue, Holding, Mechanism, MechanismDrive, MechanismEntry, MemberIndex,
};
use stave_model::solved::{Pivot, Point, PoseValue};

use crate::collide::{direction_vector, point_vector};
use crate::planar::{self, Declaration, MechanismError, MemberId, MemberName, PlanarPose};
use crate::render;

/// The exact shape of an assembly's motion, where it has one.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Shape {
    /// The driven member is pinned to a held one: the assembly turns about
    /// this line, `free_side` turning toward `swing_toward`, as far as
    /// `opening`.
    Turn {
        /// The line.
        pivot: Pivot,
        /// How far it turns.
        opening: Angle,
    },
    /// The driven member runs on a rail along a held one: the assembly
    /// travels this way, as far as `travel`.
    Travel {
        /// The direction it moves.
        toward: Direction,
        /// How far it moves.
        travel: Length,
    },
    /// Anything else: the members' poses come from the solver.
    General,
}

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
    drive: MechanismDrive,
    /// `+1` when the range runs up from `from`, `-1` when it runs down.
    sense: f64,
    /// The drive's `from`, in radians or nanometres.
    from: f64,
    /// How far the range reaches from `from`, in radians or nanometres.
    limit: f64,
    shape: Shape,
}

/// The world vector of a direction's `steps` nanometres, exactly.
fn step(point: Point, direction: Direction, amount: Length) -> Point {
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

impl MechanismPath {
    /// Runs an assembly's mechanism through its range (grammar, "How a
    /// mechanism is solved"). A mechanism with no order of steps is an
    /// error of [`planar::MechanismErrorKind::Mechanism`]; one that locks,
    /// flips or runs a pin off its slot within its range is an error of
    /// [`planar::MechanismErrorKind::Motion`].
    pub fn new(mechanism: &Mechanism) -> Result<MechanismPath, MechanismError> {
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
        let drive = *mechanism.drive();
        let planar_drive = match drive {
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
        };
        let planned = planar::Mechanism::new(declaration, planar_drive)?;
        let path = planar::Path::new(planned)?;
        let frame = mechanism.frame();
        let x = direction_vector(frame.across());
        let y = direction_vector(frame.depth());
        let amount = |value: DriveValue| planar::DriveAmount::of(to_planar(value)).value();
        let from = amount(drive.from());
        let to = amount(drive.to());
        let sense = if to >= from { 1.0 } else { -1.0 };
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
            drive,
            sense,
            from,
            limit: (to - from).abs(),
            shape: shape_of(mechanism),
        })
    }

    /// The exact shape of the motion, where it has one.
    pub fn shape(&self) -> Shape {
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
    /// is the whole range, an angle or a travel is measured from the
    /// closed pose; a value of the other kind is the closed pose.
    pub fn amount(&self, value: PoseValue) -> f64 {
        let amount = match (value, self.drive) {
            (PoseValue::Closed, _) => 0.0,
            (PoseValue::Open, _) => self.limit,
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
        if amount == 0.0 {
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
        if amount == 0.0 {
            return Some(DMat4::IDENTITY);
        }
        let pose = self.planar_pose(member, amount)?;
        let basis = DMat3::from_cols(self.x, self.y, self.normal);
        let to_world = DMat4::from_translation(self.origin) * DMat4::from_mat3(basis);
        let to_plane = DMat4::from_mat3(basis.transpose()) * DMat4::from_translation(-self.origin);
        let offset = pose.offset();
        let local = DMat4::from_translation(DVec3::new(offset.x, offset.y, 0.0))
            * DMat4::from_rotation_z(pose.angle());
        Some(to_world * local * to_plane)
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

/// An exact drive value of the solved design as the solver's.
fn to_planar(value: DriveValue) -> planar::DriveValue {
    match value {
        DriveValue::Turn(angle) => planar::DriveValue::Turn(angle),
        DriveValue::Travel(length) => planar::DriveValue::Travel(length),
    }
}

/// The exact shape of a mechanism's motion: a turn about one fixed pin, a
/// travel along one rail, or neither.
fn shape_of(mechanism: &Mechanism) -> Shape {
    let moving = mechanism.moving();
    let held = |index: MemberIndex| {
        mechanism
            .members()
            .get(mechanism.piece_of(index).index())
            .is_some_and(|member| member.holding == Holding::Held)
    };
    let joins_held = |[a, b]: [MemberIndex; 2]| {
        let (pa, pb) = (mechanism.piece_of(a), mechanism.piece_of(b));
        (pa == moving && held(b)) || (pb == moving && held(a))
    };
    let frame = mechanism.frame();
    let drive = *mechanism.drive();
    for entry in mechanism.entries() {
        match (*entry, drive) {
            (
                MechanismEntry::Pivot { first, second, at },
                MechanismDrive::Turn { from, to, .. },
            ) if joins_held([first, second]) => {
                let through = step(
                    step(frame.origin(), frame.across(), at.across),
                    frame.depth(),
                    at.depth,
                );
                let opening = to.microdeg().saturating_sub(from.microdeg());
                let swing_toward = if opening >= 0 {
                    frame.depth()
                } else {
                    frame.depth().opposite()
                };
                return Shape::Turn {
                    pivot: Pivot {
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
                    _ => return Shape::General,
                };
                let travel = to.nm().saturating_sub(from.nm());
                // The runner's travel is measured along the rail's direction
                // in the guide; the guide is held, so that is the world's.
                let runner_moves = mechanism.piece_of(runner) == moving;
                let toward = if (travel >= 0) == runner_moves {
                    along
                } else {
                    along.opposite()
                };
                return Shape::Travel {
                    toward,
                    travel: Length::from_nm(travel.saturating_abs()),
                };
            }
            _ => {}
        }
    }
    Shape::General
}
