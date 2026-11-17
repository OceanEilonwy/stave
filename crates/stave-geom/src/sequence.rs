//! Assembly sequences (grammar, "Assembly steps"): a joint's `insert`
//! steps run backwards from the finished joint, last step first, each in
//! the direction that is free, and every move swept, not only checked at
//! its ends.
//!
//! A push along one of the part's axes (or turned by a multiple of 90deg)
//! from an unturned position is **exact**, like a slide: every piece of
//! wood ahead of the moving part across the direction of travel is in its
//! way, except where a hole of one holds the other along that direction (a
//! round tenon pulled out of its hole, a pin through a bore). A push
//! without `by:` goes until the part's box is clear of the others'.
//!
//! A turn (`rotate`), a push on a diagonal (`angle:`), or any step of a
//! part already turned is **sampled**: points on the boundaries of the
//! moving part and of the others near its path ([`Shape::surface`], every
//! 1mm) are tested against the other solid ([`Shape::contains`], deeper
//! than the contact tolerance), at positions no more than 0.4mm of travel
//! apart. A sliver of wood thinner than that spacing can slip between the
//! samples; anything that blocks a step for real is far larger. That is how the chidori's quarter turn and its slide out along
//! the diagonal are proven (`stave-app/tests/oracle_chidori.rs`).

use glam::{DMat4, DVec3};
use stave_lang::placement::{Axis, Direction, PartAxis};
use stave_lang::units::{Angle, Length};
use stave_model::solved::{
    JointPair, Part, PartId, Point, SolvedDesign, StepAmount, StepKind, WorldBox,
};

use crate::collide::{
    CONTACT_TOLERANCE, Placement, WorldRod, WorldSolid, boxes_overlap, direction_vector,
    disc_into_rectangle, placed_bounds, point_vector, translate_box,
};
use crate::render;
use crate::sample::Shape;
use crate::solid::Solid;

/// Boundary samples this far apart (nanometres).
const SPACING: f64 = 1_000_000.0;
/// Positions of a sampled move at most this much travel apart.
const STEP: f64 = 400_000.0;

/// A part taking part in a joint, with its cut solid.
#[derive(Debug, Clone, Copy)]
pub struct Member<'a> {
    /// The part.
    pub part: &'a Part,
    /// Its solid.
    pub solid: &'a Solid,
}

/// What undoing one step does to its part (a step is undone in the
/// opposite direction to the one it was done in; whichever is free).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Undo {
    /// Move along a direction, turned by `angle` toward `toward` when
    /// given, exactly `by` or until clear.
    Push {
        /// The part's axis, as placed in the finished joint.
        along: Direction,
        /// A diagonal: the axis it turns toward and how far.
        turned: Option<(Direction, Angle)>,
        /// How far, when the step says.
        by: Option<Length>,
    },
    /// Turn about the part's own axis, through the middle of its section.
    Rotate {
        /// The axis.
        about: Direction,
        /// How far.
        by: Angle,
    },
}

/// One step: which member moves and how.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Step {
    /// Index into the members.
    pub member: usize,
    /// The move.
    pub undo: Undo,
}

/// A step that can't be undone either way: the step and the member in
/// its way.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Blocked {
    /// Index into the steps.
    pub step: usize,
    /// Index into the members.
    pub by: usize,
}

/// How undoing a step moves its part, in world nanometres, applied after
/// where the part is when the step is done.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Moved {
    /// Slid by a vector.
    Slide(DVec3),
    /// Turned about an axis through a point, by radians.
    Turn {
        /// A point on the axis.
        centre: DVec3,
        /// The unit axis.
        axis: DVec3,
        /// How far, radians (signed about `axis`).
        angle: f64,
    },
}

impl Moved {
    /// The move part way: `0` not moved, `1` the whole move.
    pub fn at(&self, amount: f64) -> DMat4 {
        match *self {
            Moved::Slide(vector) => DMat4::from_translation(vector * amount),
            Moved::Turn {
                centre,
                axis,
                angle,
            } => {
                DMat4::from_translation(centre)
                    * DMat4::from_axis_angle(axis, angle * amount)
                    * DMat4::from_translation(-centre)
            }
        }
    }

    /// How far the move goes: nanometres for a slide, radians for a turn.
    pub fn amount(&self) -> f64 {
        match *self {
            Moved::Slide(vector) => vector.length(),
            Moved::Turn { angle, .. } => angle.abs(),
        }
    }
}

/// One step of a sequence as it is undone: which member moves, where it is
/// when the step is done (world nanometres to world nanometres), and the
/// move that undoes it. Doing the step is the move backwards.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Traced {
    /// Index into the members.
    pub member: usize,
    /// The member's placement with this step done.
    pub done: DMat4,
    /// The move that undoes the step.
    pub moved: Moved,
}

impl Traced {
    /// The member's placement part way through the step: `0` not yet
    /// done (apart), `1` done.
    pub fn placement(&self, progress: f64) -> DMat4 {
        self.moved.at(1.0 - progress.clamp(0.0, 1.0)) * self.done
    }
}

/// Runs `steps` backwards from the finished joint. `Err` names the first
/// step (counting from the last) that is blocked both ways.
pub fn disassemble(members: &[Member<'_>], steps: &[Step]) -> Result<(), Blocked> {
    trace(members, steps).map(|_| ())
}

/// Runs `steps` backwards like [`disassemble`] and records how each one
/// moves its part, in step order (the first step first). A viewer plays
/// them forwards to show the joint going together.
pub fn trace(members: &[Member<'_>], steps: &[Step]) -> Result<Vec<Traced>, Blocked> {
    let worlds: Vec<WorldSolid> = members
        .iter()
        .map(|m| WorldSolid::of_part(m.part, m.solid))
        .collect();
    let shapes: Vec<Shape<'_>> = members
        .iter()
        .map(|m| Shape::new(m.part, m.solid))
        .collect();
    let mut at = vec![Placement::REST; members.len()];
    let state = State {
        worlds: &worlds,
        shapes: &shapes,
        members,
    };
    let mut traced = Vec::with_capacity(steps.len());
    for (index, step) in steps.iter().enumerate().rev() {
        let done = at.get(step.member).copied().unwrap_or(Placement::REST);
        let (placement, moved) = state
            .undo(step, &at)
            .map_err(|by| Blocked { step: index, by })?;
        traced.push(Traced {
            member: step.member,
            done: done.matrix(),
            moved,
        });
        if let Some(slot) = at.get_mut(step.member) {
            *slot = placement;
        }
    }
    traced.reverse();
    Ok(traced)
}

/// The parts of a joint pair its sequence moves and checks: its roles'
/// parts, then the parts it makes, each once.
pub fn pair_members(pair: &JointPair) -> Vec<PartId> {
    let mut all: Vec<PartId> = Vec::new();
    for part in pair
        .roles
        .iter()
        .map(|r| r.part)
        .chain(pair.made_parts.iter().copied())
    {
        if !all.contains(&part) {
            all.push(part);
        }
    }
    all
}

/// The axis a push's `angle:` turns toward: length → width → thickness →
/// length.
fn next_axis(axis: PartAxis) -> PartAxis {
    match axis {
        PartAxis::Length => PartAxis::Width,
        PartAxis::Width => PartAxis::Thickness,
        PartAxis::Thickness => PartAxis::Length,
    }
}

/// A joint pair's `insert` steps for [`trace`] and [`disassemble`], each
/// step's axes as its part is placed in the finished joint; `members` is
/// [`pair_members`]. `replace` swaps one step's axis (to find a push that
/// would come apart, for a help line). `None` when a step names a part
/// that isn't a member or a turn has no angle.
pub fn pair_steps(
    design: &SolvedDesign,
    pair: &JointPair,
    members: &[PartId],
    replace: Option<(usize, PartAxis)>,
) -> Option<Vec<Step>> {
    pair.insert
        .iter()
        .enumerate()
        .map(|(index, step)| {
            let member = members.iter().position(|p| *p == step.part)?;
            let orientation = design.part(step.part)?.pose.orientation;
            let axis = match replace {
                Some((at, axis)) if at == index => axis,
                _ => step.axis,
            };
            let undo = match step.kind {
                StepKind::Push | StepKind::Drive => Undo::Push {
                    along: orientation.direction(axis),
                    turned: step
                        .angle
                        .map(|angle| (orientation.direction(next_axis(axis)), angle)),
                    by: match step.by {
                        Some(StepAmount::Length(length)) => Some(length),
                        Some(StepAmount::Angle(_)) | None => None,
                    },
                },
                StepKind::Rotate => Undo::Rotate {
                    about: orientation.direction(axis),
                    by: match step.by {
                        Some(StepAmount::Angle(angle)) => angle,
                        Some(StepAmount::Length(_)) | None => return None,
                    },
                },
            };
            Some(Step { member, undo })
        })
        .collect()
}

/// A push turned by a multiple of 90deg is along an axis.
fn axis_direction(along: Direction, turned: Option<(Direction, Angle)>) -> Option<Direction> {
    let Some((toward, angle)) = turned else {
        return Some(along);
    };
    let quarter = Angle::from_deg(90).microdeg();
    if angle.microdeg().rem_euclid(quarter) != 0 {
        return None;
    }
    Some(match angle.microdeg().div_euclid(quarter).rem_euclid(4) {
        0 => along,
        1 => toward,
        2 => along.opposite(),
        _ => toward.opposite(),
    })
}

struct State<'s, 'a> {
    worlds: &'s [WorldSolid],
    shapes: &'s [Shape<'a>],
    members: &'s [Member<'a>],
}

impl State<'_, '_> {
    fn bounds(&self, index: usize, at: &[Placement]) -> Option<WorldBox> {
        let bounds = self.worlds.get(index)?.bounds()?;
        Some(placed_bounds(&bounds, at.get(index)?))
    }

    fn undo(&self, step: &Step, at: &[Placement]) -> Result<(Placement, Moved), usize> {
        let m = step.member;
        let current = at.get(m).copied().unwrap_or(Placement::REST);
        match (step.undo, current) {
            (Undo::Push { along, turned, by }, Placement::Exact(offset)) => {
                if let Some(direction) = axis_direction(along, turned) {
                    return self.exact_push(m, offset, direction, by, at);
                }
                let (toward, angle) = turned.unwrap_or((along, Angle::ZERO));
                let radians = render::radians_f64(angle);
                let v = direction_vector(along) * radians.cos()
                    + direction_vector(toward) * radians.sin();
                self.sampled_push(m, v, by, at)
            }
            (Undo::Push { along, turned, by }, Placement::Turned(_)) => {
                let (toward, angle) = turned.unwrap_or((along, Angle::ZERO));
                let radians = render::radians_f64(angle);
                let v = direction_vector(along) * radians.cos()
                    + direction_vector(toward) * radians.sin();
                self.sampled_push(m, v, by, at)
            }
            (Undo::Rotate { about, by }, _) => self.sampled_turn(m, about, by, at),
        }
    }

    /// The others, where they are, as solids exact enough for an exact
    /// push: moved exactly, or (turned) the boxes around their pieces.
    fn others_exact(&self, m: usize, at: &[Placement]) -> Vec<(usize, WorldSolid)> {
        self.worlds
            .iter()
            .enumerate()
            .filter(|(j, _)| *j != m)
            .filter_map(|(j, world)| {
                let placed = match at.get(j)? {
                    Placement::Exact(offset) => world.translated(*offset),
                    placement @ Placement::Turned(_) => WorldSolid::from_boxes(
                        world
                            .boxes()
                            .iter()
                            .copied()
                            .chain(world.rods().iter().map(WorldRod::bounds))
                            .map(|b| placed_bounds(&b, placement))
                            .collect(),
                    ),
                };
                Some((j, placed))
            })
            .collect()
    }

    fn exact_push(
        &self,
        m: usize,
        offset: Point,
        along: Direction,
        by: Option<Length>,
        at: &[Placement],
    ) -> Result<(Placement, Moved), usize> {
        let mover = self.worlds.get(m).ok_or(m)?.translated(offset);
        let others = self.others_exact(m, at);
        let tolerance = CONTACT_TOLERANCE.nm();
        let mut first_blocker = None;
        for direction in [along, along.opposite()] {
            let amount = match by {
                Some(by) => by.nm(),
                None => clear_along(&mover, &others, direction),
            };
            let blocker = others
                .iter()
                .filter_map(|(j, other)| push_contact(&mover, other, direction).map(|d| (d, *j)))
                .filter(|(distance, _)| distance.saturating_add(tolerance) < amount)
                .min();
            match blocker {
                None => {
                    let step = if direction.is_positive() {
                        amount
                    } else {
                        amount.saturating_neg()
                    };
                    let mut moved = offset;
                    moved = moved.with(
                        direction.axis(),
                        Length::from_nm(moved.get(direction.axis()).nm().saturating_add(step)),
                    );
                    let vector = direction_vector(direction) * render::nm(Length::from_nm(amount));
                    return Ok((Placement::Exact(moved), Moved::Slide(vector)));
                }
                Some((_, j)) => {
                    first_blocker.get_or_insert(j);
                }
            }
        }
        Err(first_blocker.unwrap_or(m))
    }

    fn sampled_push(
        &self,
        m: usize,
        v: DVec3,
        by: Option<Length>,
        at: &[Placement],
    ) -> Result<(Placement, Moved), usize> {
        let current = at.get(m).copied().unwrap_or(Placement::REST).matrix();
        let mut first_blocker = None;
        for direction in [v, -v] {
            let amount = match by {
                Some(by) => render::nm(by),
                None => self.clear_vector(m, direction, at),
            };
            let count = (amount / STEP).ceil().max(1.0);
            let motion = |s: f64| DMat4::from_translation(direction * (amount * s / count));
            match self.sweep(m, &motion, count, at) {
                None => {
                    return Ok((
                        Placement::Turned(motion(count) * current),
                        Moved::Slide(direction * amount),
                    ));
                }
                Some(j) => {
                    first_blocker.get_or_insert(j);
                }
            }
        }
        Err(first_blocker.unwrap_or(m))
    }

    fn sampled_turn(
        &self,
        m: usize,
        about: Direction,
        by: Angle,
        at: &[Placement],
    ) -> Result<(Placement, Moved), usize> {
        let current = at.get(m).copied().unwrap_or(Placement::REST).matrix();
        let part = self.members.get(m).ok_or(m)?.part;
        let rest = part.cut_box();
        let centre =
            current.transform_point3((point_vector(rest.min()) + point_vector(rest.max())) / 2.0);
        let axis = direction_vector(about);
        let radius = self
            .bounds(m, at)
            .map(|b| {
                corners(&b)
                    .iter()
                    .map(|c| {
                        let r = *c - centre;
                        (r - axis * r.dot(axis)).length()
                    })
                    .fold(0.0, f64::max)
            })
            .unwrap_or(0.0);
        let total = render::radians_f64(by);
        let count = (total.abs() * radius / STEP).ceil().max(1.0);
        let mut first_blocker = None;
        for sign in [1.0, -1.0] {
            let motion = |s: f64| {
                DMat4::from_translation(centre)
                    * DMat4::from_axis_angle(axis, sign * total * s / count)
                    * DMat4::from_translation(-centre)
            };
            match self.sweep(m, &motion, count, at) {
                None => {
                    return Ok((
                        Placement::Turned(motion(count) * current),
                        Moved::Turn {
                            centre,
                            axis,
                            angle: sign * total,
                        },
                    ));
                }
                Some(j) => {
                    first_blocker.get_or_insert(j);
                }
            }
        }
        Err(first_blocker.unwrap_or(m))
    }

    /// How far the mover goes along `v` until its box is clear of every
    /// other's (nanometres).
    fn clear_vector(&self, m: usize, v: DVec3, at: &[Placement]) -> f64 {
        let Some(mover) = self.bounds(m, at) else {
            return 0.0;
        };
        let (low, high) = (point_vector(mover.min()), point_vector(mover.max()));
        let tolerance = render::nm(CONTACT_TOLERANCE);
        let mut needed: f64 = 0.0;
        for j in (0..self.worlds.len()).filter(|j| *j != m) {
            let Some(other) = self.bounds(j, at) else {
                continue;
            };
            if !boxes_overlap(&mover, &other) {
                continue;
            }
            let (o_low, o_high) = (point_vector(other.min()), point_vector(other.max()));
            let mut soonest = f64::INFINITY;
            for i in 0..3 {
                let t = if v[i] > 1e-12 {
                    (o_high[i] - low[i] - tolerance) / v[i]
                } else if v[i] < -1e-12 {
                    (high[i] - o_low[i] - tolerance) / -v[i]
                } else {
                    continue;
                };
                soonest = soonest.min(t.max(0.0));
            }
            if soonest.is_finite() {
                needed = needed.max(soonest);
            }
        }
        needed
    }

    /// Sweeps the mover through `motion(s)` (world, applied after its
    /// current placement) for `s` in `1..=count`; the first other it
    /// would pass through, if any.
    fn sweep(
        &self,
        m: usize,
        motion: &dyn Fn(f64) -> DMat4,
        count: f64,
        at: &[Placement],
    ) -> Option<usize> {
        let mover_shape = self.shapes.get(m)?;
        let current = at.get(m)?.matrix();
        let start = self.bounds(m, at)?;
        // The box the mover passes through.
        let mut swept = start;
        let mut k = 0.0;
        while k <= count {
            let moved = placed_bounds(&start, &Placement::Turned(motion(k)));
            swept = crate::collide::union(&swept, &moved);
            k += (count / 16.0).max(1.0);
        }
        let end = placed_bounds(&start, &Placement::Turned(motion(count)));
        swept = crate::collide::union(&swept, &end);
        let grow = render::length_from_nm(2.0 * STEP + 1_000_000.0).unwrap_or(Length::ZERO);
        let swept = grown(&swept, grow);
        let tolerance = render::nm(CONTACT_TOLERANCE);
        for (j, shape) in self.shapes.iter().enumerate() {
            if j == m {
                continue;
            }
            let Some(other) = self.bounds(j, at) else {
                continue;
            };
            let Some(region) = crate::collide::intersection(&swept, &other) else {
                continue;
            };
            let other_world = at.get(j)?.matrix() * shape.matrix();
            let other_local = other_world.inverse();
            // The other's boundary near the path, in world coordinates.
            let (lo, hi) = local_box(&region, &other_local);
            let fixed: Vec<DVec3> = shape
                .surface(SPACING, lo, hi)
                .into_iter()
                .map(|p| other_world.transform_point3(p))
                .collect();
            // The mover's boundary that can reach the region.
            let mut mover_lo = DVec3::splat(f64::INFINITY);
            let mut mover_hi = DVec3::splat(f64::NEG_INFINITY);
            let mut k = 0.0;
            while k <= count {
                let inverse = (motion(k) * current * mover_shape.matrix()).inverse();
                let (lo, hi) = local_box(&region, &inverse);
                mover_lo = mover_lo.min(lo);
                mover_hi = mover_hi.max(hi);
                k += (count / 16.0).max(1.0);
            }
            let inverse = (motion(count) * current * mover_shape.matrix()).inverse();
            let (lo, hi) = local_box(&region, &inverse);
            let margin = DVec3::splat(4.0 * STEP);
            let moving = mover_shape.surface(
                SPACING,
                mover_lo.min(lo) - margin,
                mover_hi.max(hi) + margin,
            );
            let mut k = 1.0;
            while k <= count {
                let world = motion(k) * current * mover_shape.matrix();
                let to_other = other_local * world;
                if moving
                    .iter()
                    .any(|p| shape.contains(to_other.transform_point3(*p), tolerance))
                {
                    return Some(j);
                }
                let to_mover = world.inverse();
                let reach = placed_bounds(&start, &Placement::Turned(motion(k)));
                let (lo, hi) = (point_vector(reach.min()), point_vector(reach.max()));
                if fixed
                    .iter()
                    .filter(|p| (0..3).all(|i| p[i] >= lo[i] && p[i] <= hi[i]))
                    .any(|p| mover_shape.contains(to_mover.transform_point3(*p), tolerance))
                {
                    return Some(j);
                }
                k += 1.0;
            }
        }
        None
    }
}

fn corners(region: &WorldBox) -> [DVec3; 8] {
    let (a, b) = (point_vector(region.min()), point_vector(region.max()));
    [
        DVec3::new(a.x, a.y, a.z),
        DVec3::new(b.x, a.y, a.z),
        DVec3::new(a.x, b.y, a.z),
        DVec3::new(b.x, b.y, a.z),
        DVec3::new(a.x, a.y, b.z),
        DVec3::new(b.x, a.y, b.z),
        DVec3::new(a.x, b.y, b.z),
        DVec3::new(b.x, b.y, b.z),
    ]
}

/// The box around a world box's corners mapped by a matrix.
fn local_box(region: &WorldBox, matrix: &DMat4) -> (DVec3, DVec3) {
    corners(region).iter().fold(
        (DVec3::splat(f64::INFINITY), DVec3::splat(f64::NEG_INFINITY)),
        |(lo, hi), c| {
            let p = matrix.transform_point3(*c);
            (lo.min(p), hi.max(p))
        },
    )
}

fn grown(region: &WorldBox, by: Length) -> WorldBox {
    let low = translate_box(
        region,
        Point {
            across: Length::from_nm(by.nm().saturating_neg()),
            up: Length::from_nm(by.nm().saturating_neg()),
            deep: Length::from_nm(by.nm().saturating_neg()),
        },
    );
    let high = translate_box(
        region,
        Point {
            across: by,
            up: by,
            deep: by,
        },
    );
    WorldBox::new(low.min(), high.max())
}

/// How far the mover goes along `direction` until its box is clear of
/// every other's box (0 when it already is), nanometres.
fn clear_along(mover: &WorldSolid, others: &[(usize, WorldSolid)], direction: Direction) -> i64 {
    let Some(bounds) = mover.bounds() else {
        return 0;
    };
    let axis = direction.axis();
    let tolerance = CONTACT_TOLERANCE.nm();
    others
        .iter()
        .filter_map(|(_, other)| other.bounds())
        .filter(|other| boxes_overlap(&bounds, other))
        .map(|other| {
            let need = if direction.is_positive() {
                other
                    .max()
                    .get(axis)
                    .nm()
                    .saturating_sub(bounds.min().get(axis).nm())
            } else {
                bounds
                    .max()
                    .get(axis)
                    .nm()
                    .saturating_sub(other.min().get(axis).nm())
            };
            need.saturating_sub(tolerance).max(0)
        })
        .max()
        .unwrap_or(0)
}

/// A piece of a solid seen along an axis: its cross-section and its range.
#[derive(Debug, Clone, Copy)]
enum Section {
    /// A rectangle on the two other axes.
    Rect { low: [f64; 2], high: [f64; 2] },
    /// A disc (a rod along the axis).
    Disc { centre: [f64; 2], radius: f64 },
}

fn pieces(solid: &WorldSolid, axis: Axis) -> Vec<(Section, (i64, i64))> {
    let others = axis.others();
    let rect = |b: &WorldBox| Section::Rect {
        low: others.map(|o| render::nm(b.min().get(o))),
        high: others.map(|o| render::nm(b.max().get(o))),
    };
    let range = |b: &WorldBox| (b.min().get(axis).nm(), b.max().get(axis).nm());
    let mut out: Vec<(Section, (i64, i64))> =
        solid.boxes().iter().map(|b| (rect(b), range(b))).collect();
    for rod in solid.rods() {
        if rod.axis == axis {
            out.push((
                Section::Disc {
                    centre: rod.centre.map(render::nm),
                    radius: render::nm(rod.diameter) / 2.0,
                },
                (rod.start.nm(), rod.end.nm()),
            ));
        } else {
            let b = rod.bounds();
            out.push((rect(&b), range(&b)));
        }
    }
    out
}

/// Do two sections overlap by more than the tolerance?
fn sections_meet(a: &Section, b: &Section, axes: [Axis; 2]) -> bool {
    let tolerance = render::nm(CONTACT_TOLERANCE);
    match (a, b) {
        (Section::Rect { low: al, high: ah }, Section::Rect { low: bl, high: bh }) => {
            (0..2).all(|i| ah[i].min(bh[i]) - al[i].max(bl[i]) > tolerance)
        }
        (Section::Disc { centre, radius }, Section::Rect { low, high })
        | (Section::Rect { low, high }, Section::Disc { centre, radius }) => {
            disc_into_rectangle(*centre, *radius, *low, *high, axes).0 > tolerance
        }
        (
            Section::Disc {
                centre: c,
                radius: r,
            },
            Section::Disc {
                centre: d,
                radius: s,
            },
        ) => r + s - ((c[0] - d[0]).powi(2) + (c[1] - d[1]).powi(2)).sqrt() > tolerance,
    }
}

/// Does a hole along the axis hold a section (all of it inside the hole)?
fn holds(hole: &WorldRod, section: &Section) -> bool {
    let tolerance = render::nm(CONTACT_TOLERANCE);
    let centre = hole.centre.map(render::nm);
    let radius = render::nm(hole.diameter) / 2.0 + tolerance;
    match section {
        Section::Disc {
            centre: c,
            radius: r,
        } => ((c[0] - centre[0]).powi(2) + (c[1] - centre[1]).powi(2)).sqrt() + r <= radius,
        Section::Rect { low, high } => [low[0], high[0]].iter().all(|x| {
            [low[1], high[1]]
                .iter()
                .all(|y| (x - centre[0]).powi(2) + (y - centre[1]).powi(2) <= radius * radius)
        }),
    }
}

/// A range less the ranges of the holes (along the axis) that hold a
/// section.
fn less_holes(
    range: (i64, i64),
    holes: &[WorldRod],
    axis: Axis,
    held: &Section,
) -> Vec<(i64, i64)> {
    let mut parts = vec![range];
    for hole in holes.iter().filter(|h| h.axis == axis && holds(h, held)) {
        let (hs, he) = (hole.start.nm(), hole.end.nm());
        parts = parts
            .into_iter()
            .flat_map(|(s, e)| {
                if hs >= e || he <= s {
                    return vec![(s, e)];
                }
                let mut out = Vec::new();
                if s < hs {
                    out.push((s, hs));
                }
                if he < e {
                    out.push((he, e));
                }
                out
            })
            .collect();
    }
    parts
}

/// How far `mover` can go along `direction` before its wood would go
/// into `other`'s, exactly; `None` when nothing is in its way.
fn push_contact(mover: &WorldSolid, other: &WorldSolid, direction: Direction) -> Option<i64> {
    let axis = direction.axis();
    let axes = axis.others();
    let tolerance = CONTACT_TOLERANCE.nm();
    let mut best: Option<i64> = None;
    for (p, p_range) in pieces(mover, axis) {
        for (q, q_range) in pieces(other, axis) {
            if !sections_meet(&p, &q, axes) {
                continue;
            }
            let p_wood = less_holes(p_range, mover.holes(), axis, &q);
            let q_wood = less_holes(q_range, other.holes(), axis, &p);
            for (ps, pe) in &p_wood {
                for (qs, qe) in &q_wood {
                    // Already sharing wood: the static checks say so.
                    if pe.min(qe) - ps.max(qs) > tolerance {
                        continue;
                    }
                    let gap = if direction.is_positive() {
                        qs - pe
                    } else {
                        ps - qe
                    };
                    if gap >= -tolerance {
                        let gap = gap.max(0);
                        if best.is_none_or(|b| gap < b) {
                            best = Some(gap);
                        }
                    }
                }
            }
        }
    }
    best
}
