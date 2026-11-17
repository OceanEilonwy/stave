//! Running a [`Mechanism`] through its drive's range in `f64` nanometres.

use glam::DVec2;

use super::plan::{Body, Dyad, Locus, Mechanism, SlotInLink, Step, Unknown};
use super::{
    DEAD_POINT_MARGIN, Drive, DriveAmount, DriveValue, MechanismError, MemberId,
    PROOF_STEP_MICRODEG, PROOF_STEP_NM, SAMPLE_STEP_MICRODEG, SAMPLE_STEP_NM, proof,
};
use crate::render;

/// A pin may sit this far past an end of its slot before that is an
/// error, for rounding (a thousandth of a millimetre).
const SLOT_END_TOLERANCE_NM: f64 = 1_000.0;

/// Where one member is: turned by `angle` (radians, a positive turn
/// carrying `across` toward `depth`) about the plane's origin, then moved
/// by `offset` (nanometres). A point of the member at `p` when closed is at
/// [`PlanarPose::apply`]`(p)`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlanarPose {
    angle: f64,
    offset: DVec2,
}

impl PlanarPose {
    /// Where everything is when closed.
    pub const REST: PlanarPose = PlanarPose {
        angle: 0.0,
        offset: DVec2::ZERO,
    };

    /// How far it has turned, in radians.
    pub fn angle(&self) -> f64 {
        self.angle
    }

    /// How far it has moved after turning, in nanometres.
    pub fn offset(&self) -> DVec2 {
        self.offset
    }

    /// Where a point of the member (closed, nanometres in the plane) is.
    pub fn apply(&self, point: DVec2) -> DVec2 {
        self.rotate(point) + self.offset
    }

    /// A vector of the member turned with it.
    pub fn rotate(&self, vector: DVec2) -> DVec2 {
        DVec2::from_angle(self.angle).rotate(vector)
    }

    /// The pose that carries `pivot` to `pivot_now` and `joint` to (the
    /// direction of) `joint_now`.
    fn through(pivot: DVec2, pivot_now: DVec2, joint: DVec2, joint_now: DVec2) -> PlanarPose {
        let angle = (joint - pivot).angle_to(joint_now - pivot_now);
        PlanarPose {
            angle,
            offset: pivot_now - DVec2::from_angle(angle).rotate(pivot),
        }
    }
}

/// Every member's pose at one drive value.
#[derive(Debug, Clone, PartialEq)]
pub struct Pose {
    members: Vec<PlanarPose>,
    margin: Option<f64>,
}

impl Pose {
    /// A member's pose (`None` for an id from another declaration).
    pub fn member(&self, member: MemberId) -> Option<PlanarPose> {
        self.members.get(member.index()).copied()
    }

    /// Every member's pose, in the order declared.
    pub fn members(&self) -> &[PlanarPose] {
        &self.members
    }

    /// How far, in nanometres, the step nearest a dead point is from it
    /// here (see [`DEAD_POINT_MARGIN`]); `None` when no step has two
    /// answers.
    pub fn margin(&self) -> Option<f64> {
        self.margin
    }
}

/// How fast a member can move over the drive's range, per unit of drive
/// (radian or nanometre), with a tenth to spare for what lies between
/// samples.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Speed {
    /// Radians of turn per unit of drive.
    pub turn: f64,
    /// Nanometres of offset per unit of drive.
    pub offset: f64,
}

impl Speed {
    /// The fastest any point of the member within `radius` nanometres of
    /// the plane's origin moves, in nanometres per unit of drive.
    pub fn bound(&self, radius: f64) -> f64 {
        self.turn * radius + self.offset
    }
}

/// A [`Mechanism`] checked through its whole range: no dead point within
/// [`DEAD_POINT_MARGIN`], no branch flip, every pin inside its slot. See
/// the module docs.
#[derive(Debug, Clone, PartialEq)]
pub struct Path {
    mechanism: Mechanism,
    speeds: Vec<Speed>,
    proof: Proof,
}

/// What [`Path::new`]'s proof found over the whole range (see
/// `proof.rs`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Proof {
    /// The least margin, in nanometres, proven for any step anywhere in
    /// the range: never less than [`DEAD_POINT_MARGIN`]. `None` when no
    /// step has two answers.
    pub margin: Option<f64>,
    /// The most times an interval between two samples was halved before
    /// it was proven (0 when every one was proven at once).
    pub halvings: u32,
}

/// One drive value and what the steps give there.
#[derive(Debug, Clone)]
struct Sample {
    /// Exact units (micro-degrees or nanometres), absolute.
    units: i64,
    /// Radians or nanometres past the drive's start.
    relative: f64,
    evaluation: Evaluation,
}

impl Sample {
    /// The steps at `units`; a step whose paths don't meet is a dead
    /// point there.
    fn at(mechanism: &Mechanism, units: i64) -> Result<Sample, MechanismError> {
        let (from, _) = mechanism.range_units();
        let relative_units = units.checked_sub(from).ok_or(MechanismError::Overflow)?;
        let relative = relative_amount(mechanism, relative_units);
        let evaluation = evaluate(mechanism, relative).map_err(|failure| match failure {
            Failure::Unreachable(index) => {
                dead_point(mechanism, index, mechanism.value_of_units(units))
            }
            Failure::Internal => MechanismError::Internal,
        })?;
        Ok(Sample {
            units,
            relative,
            evaluation,
        })
    }
}

/// The least margin of any step at the samples alone, every
/// [`SAMPLE_STEP_MICRODEG`] or [`SAMPLE_STEP_NM`], as the check was before
/// the proof between samples: for tests that show what sampling misses.
#[cfg(test)]
pub(super) fn least_sampled_margin(mechanism: &Mechanism) -> Option<f64> {
    let (from, to) = mechanism.range_units();
    let span = i128::from(to) - i128::from(from);
    let step = i128::from(match mechanism.drive() {
        Drive::Turn { .. } => SAMPLE_STEP_MICRODEG,
        Drive::Travel { .. } => SAMPLE_STEP_NM,
    });
    let samples = ((span.abs() + step - 1) / step).max(1);
    (0..=samples)
        .filter_map(|k| {
            let units = i64::try_from(span * k / samples).ok()?;
            let evaluation = evaluate(mechanism, relative_amount(mechanism, units)).ok()?;
            evaluation
                .crossings
                .iter()
                .flatten()
                .map(|crossing| crossing.margin)
                .reduce(f64::min)
        })
        .reduce(f64::min)
}

/// Proves the interval between two samples clear of every dead point,
/// halving it where it can't, the first half first (see
/// `proof.rs`).
fn prove_between(
    mechanism: &Mechanism,
    start: &Sample,
    end: &Sample,
    halvings: u32,
    found: &mut Proof,
) -> Result<(), MechanismError> {
    let margin = render::nm(DEAD_POINT_MARGIN);
    let width = (end.relative - start.relative).abs();
    let outcome = proof::prove(
        mechanism,
        width,
        margin,
        &start.evaluation.crossings,
        &end.evaluation.crossings,
    )
    .map_err(|proof::Fault| MechanismError::Internal)?;
    match outcome {
        proof::Outcome::Proven(least) => {
            found.margin = match (found.margin, least) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (a, b) => a.or(b),
            };
            found.halvings = found.halvings.max(halvings);
            Ok(())
        }
        proof::Outcome::Unproven { step, at_end } => {
            let finest = match mechanism.drive() {
                Drive::Turn { .. } => PROOF_STEP_MICRODEG,
                Drive::Travel { .. } => PROOF_STEP_NM,
            };
            let apart = i128::from(end.units) - i128::from(start.units);
            if apart.abs() <= i128::from(finest) {
                let nearer = if at_end { end.units } else { start.units };
                return Err(MechanismError::NearDeadPoint {
                    members: step_members(mechanism, step),
                    at: mechanism.value_of_units(nearer),
                });
            }
            let middle = i128::from(start.units) + apart / 2;
            let middle = i64::try_from(middle).map_err(|_| MechanismError::Overflow)?;
            let middle = Sample::at(mechanism, middle)?;
            let deeper = halvings.saturating_add(1);
            prove_between(mechanism, start, &middle, deeper, found)?;
            prove_between(mechanism, &middle, end, deeper, found)
        }
    }
}

/// One step's two crossings and how far it is from a dead point.
#[derive(Debug, Clone, Copy)]
pub(super) struct Crossing {
    chosen: DVec2,
    other: DVec2,
    /// How far from a dead point, in nanometres (see
    /// [`DEAD_POINT_MARGIN`]).
    pub(super) margin: f64,
    /// For a circle and a line, how far the circle's centre is from the
    /// line's point (nanometres); zero otherwise.
    pub(super) spread: f64,
}

/// A pin's place along its slot: 0 at `from`, 1 at `to`.
#[derive(Debug, Clone, Copy)]
struct SlotPlace {
    pin: Body,
    guide: Body,
    along: f64,
    tolerance: f64,
}

/// What one evaluation found, for the checks.
#[derive(Debug, Clone, Default)]
struct Evaluation {
    poses: Vec<PlanarPose>,
    crossings: Vec<Option<Crossing>>,
    slots: Vec<SlotPlace>,
}

/// Why an evaluation stopped.
#[derive(Debug, Clone, Copy)]
enum Failure {
    /// The step's paths don't meet.
    Unreachable(usize),
    /// A step read something not known yet.
    Internal,
}

/// A path: a circle or a line.
#[derive(Debug, Clone, Copy)]
enum Shape {
    Circle { centre: DVec2, radius: f64 },
    Line { point: DVec2, direction: DVec2 },
}

/// The two crossings of two circles: the one on `side` of the line from
/// the first centre to the second first.
fn circles(c1: DVec2, r1: f64, c2: DVec2, r2: f64, side: f64) -> Option<Crossing> {
    let between = c2 - c1;
    let distance = between.length();
    let margin = (r1 + r2 - distance).min(distance - (r1 - r2).abs());
    if distance < 1e-9 {
        return None;
    }
    let along = (r1 * r1 - r2 * r2 + distance * distance) / (2.0 * distance);
    let across_squared = r1 * r1 - along * along;
    if across_squared < 0.0 {
        return None;
    }
    let unit = between / distance;
    let half = across_squared.sqrt();
    Some(Crossing {
        chosen: c1 + unit * along + unit.perp() * (side * half),
        other: c1 + unit * along + unit.perp() * (-side * half),
        margin,
        spread: 0.0,
    })
}

/// The two crossings of a circle and a line: the one on `side` along the
/// line's direction from the foot of the centre first.
fn circle_line(
    centre: DVec2,
    radius: f64,
    point: DVec2,
    direction: DVec2,
    side: f64,
) -> Option<Crossing> {
    let unit = direction.try_normalize()?;
    let foot = point + unit * (centre - point).dot(unit);
    let distance = (centre - foot).length();
    let half_squared = radius * radius - distance * distance;
    if half_squared < 0.0 {
        return None;
    }
    let half = half_squared.sqrt();
    Some(Crossing {
        chosen: foot + unit * (side * half),
        other: foot + unit * (-side * half),
        margin: radius - distance,
        spread: (centre - point).length(),
    })
}

/// Where along a line `point + t·direction` a point is, and the tolerance
/// in the same measure.
fn along(at: DVec2, point: DVec2, direction: DVec2) -> (f64, f64) {
    let squared = direction.length_squared();
    (
        (at - point).dot(direction) / squared,
        SLOT_END_TOLERANCE_NM / squared.sqrt(),
    )
}

/// What is known while the steps run.
struct State {
    poses: Vec<Option<PlanarPose>>,
    angles: Vec<Option<f64>>,
    travels: Vec<Option<f64>>,
}

impl State {
    fn pose(&self, body: Body) -> Result<PlanarPose, Failure> {
        self.poses
            .get(body.0)
            .copied()
            .flatten()
            .ok_or(Failure::Internal)
    }

    fn place(&mut self, body: Body, pose: PlanarPose) -> Result<(), Failure> {
        let slot = self.poses.get_mut(body.0).ok_or(Failure::Internal)?;
        *slot = Some(pose);
        Ok(())
    }

    fn angle(&self, body: Body) -> Result<f64, Failure> {
        self.angles
            .get(body.0)
            .copied()
            .flatten()
            .ok_or(Failure::Internal)
    }

    fn travel(&self, body: Body) -> Result<f64, Failure> {
        self.travels
            .get(body.0)
            .copied()
            .flatten()
            .ok_or(Failure::Internal)
    }
}

/// Runs every step with the drive `relative` (radians or nanometres) past
/// its closed value.
fn evaluate(mechanism: &Mechanism, relative: f64) -> Result<Evaluation, Failure> {
    let count = mechanism.body_count();
    let mut state = State {
        poses: (0..count)
            .map(|b| mechanism.is_held(Body(b)).then_some(PlanarPose::REST))
            .collect(),
        angles: vec![None; count],
        travels: vec![None; count],
    };
    let driven = mechanism.drive_body().0;
    let known = match mechanism.drive() {
        Drive::Turn { .. } => state.angles.get_mut(driven),
        Drive::Travel { .. } => state.travels.get_mut(driven),
    };
    *known.ok_or(Failure::Internal)? = Some(relative);

    let mut crossings = Vec::with_capacity(mechanism.steps().len());
    let mut slots = Vec::new();
    for (index, step) in mechanism.steps().iter().enumerate() {
        let crossing = match step {
            Step::Couple { from, to, factor } => {
                let travel = state.travel(*from)? * factor;
                *state.travels.get_mut(to.0).ok_or(Failure::Internal)? = Some(travel);
                None
            }
            Step::Hang { body, known, pin } => {
                let angle = state.angle(*body)?;
                let at = state.pose(*known)?.apply(*pin);
                state.place(
                    *body,
                    PlanarPose {
                        angle,
                        offset: at - DVec2::from_angle(angle).rotate(*pin),
                    },
                )?;
                None
            }
            Step::Rail {
                runner,
                guide,
                direction,
            } => {
                let travel = state.travel(*runner)?;
                let guide_pose = state.pose(*guide)?;
                let world = guide_pose.rotate(*direction);
                let along = travel - guide_pose.offset.dot(world);
                state.place(
                    *runner,
                    PlanarPose {
                        angle: guide_pose.angle,
                        offset: guide_pose.offset + world * along,
                    },
                )?;
                None
            }
            Step::Dyad(dyad) => Some(dyad_step(dyad, index, &mut state, &mut slots)?),
            Step::SlotInLink(slot) => Some(slot_in_link(slot, index, &mut state, &mut slots)?),
        };
        crossings.push(crossing);
    }
    let poses = state
        .poses
        .into_iter()
        .map(|pose| pose.ok_or(Failure::Internal))
        .collect::<Result<_, _>>()?;
    Ok(Evaluation {
        poses,
        crossings,
        slots,
    })
}

/// A locus as a path: in the world for a joint, in offsets for a member
/// turned by `rotation`.
fn shape(locus: Locus, state: &State, rotation: Option<DVec2>) -> Result<Shape, Failure> {
    let shift = |joint: DVec2| rotation.map_or(DVec2::ZERO, |rotation| rotation.rotate(joint));
    Ok(match locus {
        Locus::Link {
            known,
            pivot,
            joint,
            radius,
            ..
        } => Shape::Circle {
            centre: state.pose(known)?.apply(pivot) - shift(joint),
            radius,
        },
        Locus::Rail {
            guide,
            direction,
            joint,
            ..
        } => {
            let guide = state.pose(guide)?;
            Shape::Line {
                point: guide.apply(joint) - shift(joint),
                direction: guide.rotate(direction),
            }
        }
        Locus::Slot {
            guide, from, to, ..
        } => {
            let guide = state.pose(guide)?;
            Shape::Line {
                point: guide.apply(from) - shift(from),
                direction: guide.rotate(to - from),
            }
        }
        Locus::SlotInUnknown { pin, from, to, .. } => {
            let rotation = rotation.ok_or(Failure::Internal)?;
            Shape::Line {
                point: state.pose(pin)?.apply(from) - rotation.rotate(from),
                direction: -rotation.rotate(to - from),
            }
        }
    })
}

fn dyad_step(
    dyad: &Dyad,
    index: usize,
    state: &mut State,
    slots: &mut Vec<SlotPlace>,
) -> Result<Crossing, Failure> {
    let turned = match dyad.unknown {
        Unknown::Joint => None,
        Unknown::Offset(body) => Some(state.angle(body)?),
    };
    let rotation = turned.map(DVec2::from_angle);
    let first = shape(dyad.first, state, rotation)?;
    let second = shape(dyad.second, state, rotation)?;
    let crossing = match (first, second) {
        (
            Shape::Circle {
                centre: c1,
                radius: r1,
            },
            Shape::Circle {
                centre: c2,
                radius: r2,
            },
        ) => circles(c1, r1, c2, r2, dyad.side),
        (Shape::Circle { centre, radius }, Shape::Line { point, direction })
        | (Shape::Line { point, direction }, Shape::Circle { centre, radius }) => {
            circle_line(centre, radius, point, direction, dyad.side)
        }
        (Shape::Line { .. }, Shape::Line { .. }) => None,
    }
    .ok_or(Failure::Unreachable(index))?;
    let found = crossing.chosen;

    // The member whose offset was found, then each locus's member.
    let mover = match (dyad.unknown, turned) {
        (Unknown::Offset(body), Some(angle)) => {
            let pose = PlanarPose {
                angle,
                offset: found,
            };
            state.place(body, pose)?;
            Some(pose)
        }
        (Unknown::Joint, _) | (Unknown::Offset(_), None) => None,
    };
    for (locus, path) in [(dyad.first, first), (dyad.second, second)] {
        // The world point where this locus's member meets the unknown.
        let meet = |joint: DVec2| mover.map_or(found, |pose| pose.apply(joint));
        match locus {
            Locus::Link {
                link,
                known,
                pivot,
                joint,
                ..
            } => {
                let pivot_now = state.pose(known)?.apply(pivot);
                let pose = PlanarPose::through(pivot, pivot_now, joint, meet(joint));
                state.place(link, pose)?;
            }
            Locus::Rail {
                runner,
                guide,
                joint,
                ..
            } => {
                let guide = state.pose(guide)?;
                let pose = PlanarPose {
                    angle: guide.angle,
                    offset: meet(joint) - guide.rotate(joint),
                };
                state.place(runner, pose)?;
            }
            Locus::Slot { pin, guide, .. } | Locus::SlotInUnknown { pin, guide, .. } => {
                if let Shape::Line { point, direction } = path {
                    let (along, tolerance) = along(found, point, direction);
                    slots.push(SlotPlace {
                        pin,
                        guide,
                        along,
                        tolerance,
                    });
                }
            }
        }
    }
    Ok(crossing)
}

fn slot_in_link(
    slot: &SlotInLink,
    index: usize,
    state: &mut State,
    slots: &mut Vec<SlotPlace>,
) -> Result<Crossing, Failure> {
    let pivot_now = state.pose(slot.known)?.apply(slot.pivot);
    let pin_now = state.pose(slot.pin)?.apply(slot.from);
    let reach = (pin_now - pivot_now).length();
    // In the link's own frame: the pin is `reach` from the pivot and on
    // the slot's line.
    let direction = slot.to - slot.from;
    let crossing = circle_line(slot.pivot, reach, slot.from, direction, slot.side)
        .ok_or(Failure::Unreachable(index))?;
    let local = crossing.chosen;
    state.place(
        slot.link,
        PlanarPose::through(slot.pivot, pivot_now, local, pin_now),
    )?;
    let (along, tolerance) = along(local, slot.from, direction);
    slots.push(SlotPlace {
        pin: slot.pin,
        guide: slot.link,
        along,
        tolerance,
    });
    Ok(crossing)
}

/// An angle difference brought into `(-π, π]`.
fn wrapped(angle: f64) -> f64 {
    let turn = std::f64::consts::TAU;
    let reduced = angle.rem_euclid(turn);
    if reduced > std::f64::consts::PI {
        reduced - turn
    } else {
        reduced
    }
}

impl Path {
    /// Runs `mechanism` through its range and checks it (see the module
    /// docs): every sample, and between each two samples a proof that no
    /// step comes within [`DEAD_POINT_MARGIN`] of a dead point (see
    /// `proof.rs`), halving an interval it can't prove down
    /// to [`PROOF_STEP_MICRODEG`] or [`PROOF_STEP_NM`].
    pub fn new(mechanism: Mechanism) -> Result<Path, MechanismError> {
        let (from, to) = mechanism.range_units();
        let span = i128::from(to) - i128::from(from);
        let step = match mechanism.drive() {
            Drive::Turn { .. } => SAMPLE_STEP_MICRODEG,
            Drive::Travel { .. } => SAMPLE_STEP_NM,
        };
        let step = i128::from(step);
        let samples = ((span.abs() + step - 1) / step).max(1);
        let margin = render::nm(DEAD_POINT_MARGIN);
        let count = mechanism.body_count();
        let mut fastest = vec![(0.0f64, 0.0f64); count];
        let mut proof = Proof {
            margin: None,
            halvings: 0,
        };
        let mut previous: Option<Sample> = None;
        for k in 0..=samples {
            let relative_units = span
                .checked_mul(k)
                .map(|scaled| scaled / samples)
                .and_then(|relative| i64::try_from(relative).ok())
                .ok_or(MechanismError::Overflow)?;
            let value = from
                .checked_add(relative_units)
                .ok_or(MechanismError::Overflow)?;
            let at = mechanism.value_of_units(value);
            let sample = Sample::at(&mechanism, value)?;
            let evaluation = &sample.evaluation;
            for (index, crossing) in evaluation.crossings.iter().enumerate() {
                if let Some(crossing) = crossing
                    && crossing.margin < margin
                {
                    return Err(dead_point(&mechanism, index, at));
                }
            }
            for slot in &evaluation.slots {
                if slot.along < -slot.tolerance || slot.along > 1.0 + slot.tolerance {
                    return Err(MechanismError::SlotEnd {
                        pin: mechanism.body_name(slot.pin),
                        guide: mechanism.body_name(slot.guide),
                        at,
                    });
                }
            }
            if let Some(last) = &previous {
                for (index, (now, then)) in evaluation
                    .crossings
                    .iter()
                    .zip(&last.evaluation.crossings)
                    .enumerate()
                {
                    if let (Some(now), Some(then)) = (now, then)
                        && (now.other - then.chosen).length() < (now.chosen - then.chosen).length()
                    {
                        return Err(MechanismError::BranchFlip {
                            members: step_members(&mechanism, index),
                            at,
                        });
                    }
                }
                prove_between(&mechanism, last, &sample, 0, &mut proof)?;
                let change = (sample.relative - last.relative).abs();
                if change > 0.0 {
                    for ((speed, now), then) in fastest
                        .iter_mut()
                        .zip(&evaluation.poses)
                        .zip(&last.evaluation.poses)
                    {
                        let turn = wrapped(now.angle - then.angle).abs() / change;
                        let offset = (now.offset - then.offset).length() / change;
                        *speed = (speed.0.max(turn), speed.1.max(offset));
                    }
                }
            }
            previous = Some(sample);
        }
        let speeds = fastest
            .into_iter()
            .map(|(turn, offset)| Speed {
                turn: turn * 1.1 + 1e-9,
                offset: offset * 1.1 + 1_000.0,
            })
            .collect();
        Ok(Path {
            mechanism,
            speeds,
            proof,
        })
    }

    /// What the proof between samples found: the least margin it proved
    /// and how finely it had to halve.
    pub fn proof(&self) -> Proof {
        self.proof
    }

    /// The mechanism.
    pub fn mechanism(&self) -> &Mechanism {
        &self.mechanism
    }

    /// Every member's pose at an exact drive value; exactly
    /// [`PlanarPose::REST`] for every member at the drive's `from`. A value
    /// outside the range, or of the other kind of drive, is
    /// [`MechanismError::OutOfRange`].
    pub fn pose(&self, at: DriveValue) -> Result<Pose, MechanismError> {
        let (from, to) = self.mechanism.range_units();
        let units = match (self.mechanism.drive(), at) {
            (Drive::Turn { .. }, DriveValue::Turn(angle)) => angle.microdeg(),
            (Drive::Travel { .. }, DriveValue::Travel(length)) => length.nm(),
            (Drive::Turn { .. }, DriveValue::Travel(_))
            | (Drive::Travel { .. }, DriveValue::Turn(_)) => return Err(self.out_of_range()),
        };
        if units < from.min(to) || units > from.max(to) {
            return Err(self.out_of_range());
        }
        let relative = units.checked_sub(from).ok_or(MechanismError::Overflow)?;
        self.solve(relative_amount(&self.mechanism, relative), at)
    }

    /// Every member's pose at a drive value in floats (radians or
    /// nanometres, absolute): for sweeps.
    pub fn pose_at(&self, amount: DriveAmount) -> Result<Pose, MechanismError> {
        let drive = self.mechanism.drive();
        let (from, to) = (
            DriveAmount::of(drive.from()).value(),
            DriveAmount::of(drive.to()).value(),
        );
        let value = amount.value();
        if !(from.min(to)..=from.max(to)).contains(&value) {
            return Err(self.out_of_range());
        }
        self.solve(value - from, drive.from())
    }

    /// How fast a member can move (`None` for an id from another
    /// declaration).
    pub fn speed(&self, member: MemberId) -> Option<Speed> {
        let body = self.mechanism.member_bodies().get(member.index())?;
        self.speeds.get(body.0).copied()
    }

    fn out_of_range(&self) -> MechanismError {
        let drive = self.mechanism.drive();
        MechanismError::OutOfRange {
            from: drive.from(),
            to: drive.to(),
        }
    }

    fn solve(&self, relative: f64, at: DriveValue) -> Result<Pose, MechanismError> {
        let evaluation = evaluate(&self.mechanism, relative).map_err(|failure| match failure {
            Failure::Unreachable(index) => dead_point(&self.mechanism, index, at),
            Failure::Internal => MechanismError::Internal,
        })?;
        let margin = evaluation
            .crossings
            .iter()
            .flatten()
            .map(|crossing| crossing.margin)
            .reduce(f64::min);
        let members = self
            .mechanism
            .member_bodies()
            .iter()
            .map(|body| {
                if relative == 0.0 {
                    Ok(PlanarPose::REST)
                } else {
                    evaluation
                        .poses
                        .get(body.0)
                        .copied()
                        .ok_or(MechanismError::Internal)
                }
            })
            .collect::<Result<_, _>>()?;
        Ok(Pose { members, margin })
    }
}

/// A drive change in exact units (micro-degrees or nanometres) as radians
/// or nanometres.
fn relative_amount(mechanism: &Mechanism, units: i64) -> f64 {
    let value = mechanism.value_of_units(units);
    DriveAmount::of(value).value()
}

fn step_members(mechanism: &Mechanism, index: usize) -> Vec<super::MemberName> {
    let bodies: Vec<Body> = match mechanism.steps().get(index) {
        Some(Step::Dyad(dyad)) => dyad.members.clone(),
        Some(Step::SlotInLink(slot)) => vec![slot.link, slot.pin],
        Some(Step::Hang { body, .. }) => vec![*body],
        Some(Step::Rail { runner, .. }) => vec![*runner],
        Some(Step::Couple { to, .. }) => vec![*to],
        None => Vec::new(),
    };
    bodies
        .into_iter()
        .map(|body| mechanism.body_name(body))
        .collect()
}

fn dead_point(mechanism: &Mechanism, index: usize, at: DriveValue) -> MechanismError {
    MechanismError::DeadPoint {
        members: step_members(mechanism, index),
        at,
    }
}
