//! Checking a declaration and ordering its steps, without numbers but for
//! the exact branch each two-answer step takes at the closed pose.

use glam::DVec2;
use stave_lang::units::{Length, Rational};

use super::{
    Constraint, Declaration, Drive, DriveValue, Holding, MechanismError, MemberId, MemberName,
    PlanarDirection, PlanarPoint,
};
use crate::render;

/// One rigid piece: a free or held member with every member it carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(super) struct Body(pub(super) usize);

/// An exact point or vector, nanometres.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Exact {
    across: i128,
    depth: i128,
}

impl Exact {
    const ZERO: Exact = Exact {
        across: 0,
        depth: 0,
    };

    fn of(point: PlanarPoint) -> Exact {
        Exact {
            across: i128::from(point.across.nm()),
            depth: i128::from(point.depth.nm()),
        }
    }

    fn of_direction(direction: PlanarDirection) -> Exact {
        Exact {
            across: i128::from(direction.across().nm()),
            depth: i128::from(direction.depth().nm()),
        }
    }

    fn minus(self, other: Exact) -> Option<Exact> {
        Some(Exact {
            across: self.across.checked_sub(other.across)?,
            depth: self.depth.checked_sub(other.depth)?,
        })
    }

    fn negated(self) -> Option<Exact> {
        Exact::ZERO.minus(self)
    }

    fn cross(self, other: Exact) -> Option<i128> {
        self.across
            .checked_mul(other.depth)?
            .checked_sub(self.depth.checked_mul(other.across)?)
    }

    fn dot(self, other: Exact) -> Option<i128> {
        self.across
            .checked_mul(other.across)?
            .checked_add(self.depth.checked_mul(other.depth)?)
    }
}

/// A point in `f64` nanometres.
fn float(point: PlanarPoint) -> DVec2 {
    DVec2::new(render::nm(point.across), render::nm(point.depth))
}

/// A direction in `f64` nanometres (its exact scale).
fn float_direction(direction: PlanarDirection) -> DVec2 {
    DVec2::new(
        render::nm(direction.across()),
        render::nm(direction.depth()),
    )
}

/// An integer as an `f64` (the nearest one past 2^53), without a cast.
fn float_of(value: i128) -> Option<f64> {
    let value = i64::try_from(value).ok()?;
    Some(render::nm(Length::from_nm(value)))
}

/// A ratio as an `f64`.
fn float_ratio(value: Rational) -> Option<f64> {
    Some(float_of(value.numerator())? / float_of(value.denominator())?)
}

/// An entry with its members resolved to bodies.
#[derive(Debug, Clone, Copy)]
enum Entry {
    Pivot {
        first: Body,
        second: Body,
        at: PlanarPoint,
    },
    Slot {
        pin: Body,
        guide: Body,
        from: PlanarPoint,
        to: PlanarPoint,
    },
    Rail {
        runner: Body,
        guide: Body,
        direction: PlanarDirection,
    },
    Couple {
        follower: Body,
        leader: Body,
        ratio: Rational,
    },
}

impl Entry {
    fn bodies(self) -> (Body, Body) {
        match self {
            Entry::Pivot { first, second, .. } => (first, second),
            Entry::Slot { pin, guide, .. } => (pin, guide),
            Entry::Rail { runner, guide, .. } => (runner, guide),
            Entry::Couple {
                follower, leader, ..
            } => (follower, leader),
        }
    }

    /// The freedoms it takes away.
    fn takes(self) -> i64 {
        match self {
            Entry::Pivot { .. } | Entry::Rail { .. } => 2,
            Entry::Slot { .. } | Entry::Couple { .. } => 1,
        }
    }
}

/// A path a point (or a member's offset) must lie on.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum Locus {
    /// `link` is pinned to the placed member `known` at `pivot`; the point
    /// is on `link` at `joint`: a circle about `known`'s pivot.
    Link {
        link: Body,
        known: Body,
        pivot: DVec2,
        joint: DVec2,
        radius: f64,
    },
    /// The point is on `runner` at `joint`, and `runner` runs on a rail
    /// along the placed member `guide`: a line.
    Rail {
        runner: Body,
        guide: Body,
        direction: DVec2,
        joint: DVec2,
    },
    /// The point is the pin of a slot in the placed member `guide`: a line.
    Slot {
        pin: Body,
        guide: Body,
        from: DVec2,
        to: DVec2,
    },
    /// The slot is in the member whose offset is solved, over a pin on the
    /// placed member `pin` (only for an offset): a line.
    SlotInUnknown {
        pin: Body,
        guide: Body,
        from: DVec2,
        to: DVec2,
    },
}

impl Locus {
    /// The member this locus places once the point is found.
    fn places(self) -> Option<Body> {
        match self {
            Locus::Link { link, .. } => Some(link),
            Locus::Rail { runner, .. } => Some(runner),
            Locus::Slot { .. } | Locus::SlotInUnknown { .. } => None,
        }
    }

    fn is_circle(self) -> bool {
        matches!(self, Locus::Link { .. })
    }
}

/// What a two-path step finds.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum Unknown {
    /// A point where the loci's members meet.
    Joint,
    /// The offset of a member whose angle is known.
    Offset(Body),
}

/// A step that finds one point on two paths.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Dyad {
    pub(super) unknown: Unknown,
    pub(super) first: Locus,
    pub(super) second: Locus,
    /// `+1` or `-1`: which of the two crossings, as the closed pose has it.
    pub(super) side: f64,
    pub(super) members: Vec<Body>,
}

/// A link pinned to a placed member whose slot lies over a placed pin.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct SlotInLink {
    pub(super) link: Body,
    pub(super) known: Body,
    pub(super) pivot: DVec2,
    pub(super) pin: Body,
    pub(super) from: DVec2,
    pub(super) to: DVec2,
    pub(super) side: f64,
}

/// One closed-form step.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum Step {
    /// `to`'s travel is `factor` times `from`'s.
    Couple { from: Body, to: Body, factor: f64 },
    /// `body`'s angle is known and it is pinned to `known` at `pin`.
    Hang { body: Body, known: Body, pin: DVec2 },
    /// `body`'s angle and its travel along its slot in `guide` are known:
    /// its pin, at `from` when closed, is at `from + unit * travel` in
    /// `guide` (a pivot bolt raised in its slot and turned).
    HangInSlot {
        body: Body,
        guide: Body,
        from: DVec2,
        /// The unit direction a positive travel runs, in `guide`.
        unit: DVec2,
        /// Its slot's other end, for the slot-end check.
        to: DVec2,
    },
    /// `runner`'s travel is known; it runs along `guide`, which doesn't
    /// turn, in the unit `direction`.
    Rail {
        runner: Body,
        guide: Body,
        direction: DVec2,
    },
    /// A point on two paths.
    Dyad(Dyad),
    /// A slot in a link.
    SlotInLink(SlotInLink),
}

/// A declared planar mechanism with one freedom and a known order of
/// closed-form steps. See the module docs.
#[derive(Debug, Clone, PartialEq)]
pub struct Mechanism {
    names: Vec<MemberName>,
    member_bodies: Vec<Body>,
    body_names: Vec<MemberName>,
    held: Vec<bool>,
    /// Never empty; every one moves the same body.
    drives: Vec<Drive>,
    drive_body: Body,
    /// The drive run through its range, and its place in `drives`.
    drive: Drive,
    swept: usize,
    /// Every drive's value past its `from` (radians or nanometres) while
    /// the swept one runs; the swept one's is not read.
    held_values: Vec<f64>,
    steps: Vec<Step>,
}

impl Mechanism {
    /// Checks a declaration and orders its steps (see the module docs).
    pub fn new(declaration: Declaration, drive: Drive) -> Result<Mechanism, MechanismError> {
        Mechanism::with_drives(declaration, vec![drive])
    }

    /// Checks a declaration with any number of drives and orders its steps:
    /// one freedom for each drive, every drive moving the same member,
    /// once turning and once travelling. The first drive is the one run
    /// through its range, with the others at their `from`, until
    /// [`Mechanism::sweeping`] chooses another.
    pub fn with_drives(
        declaration: Declaration,
        drives: Vec<Drive>,
    ) -> Result<Mechanism, MechanismError> {
        let Some(&drive) = drives.first() else {
            return Err(MechanismError::NoSuchDrive);
        };
        let Declaration {
            members,
            constraints,
        } = declaration;
        let names: Vec<MemberName> = members.iter().map(|(name, _)| name.clone()).collect();
        let name = |id: MemberId| -> Result<MemberName, MechanismError> {
            names
                .get(id.index())
                .cloned()
                .ok_or(MechanismError::UnknownMember { member: id })
        };

        // Each member's body: the held or free member it rides on.
        let mut root_of = Vec::with_capacity(members.len());
        for (index, (member_name, _)) in members.iter().enumerate() {
            let mut at = index;
            let mut hops = 0;
            loop {
                match members.get(at).map(|(_, holding)| *holding) {
                    Some(Holding::Carried(carrier)) => at = carrier.index(),
                    Some(Holding::Held | Holding::Free) => break,
                    None => return Err(MechanismError::Internal),
                }
                hops += 1;
                if hops > members.len() {
                    return Err(MechanismError::CarriedInCircle {
                        member: member_name.clone(),
                    });
                }
            }
            root_of.push(at);
        }
        let mut roots: Vec<usize> = root_of.clone();
        roots.sort_unstable();
        roots.dedup();
        let body_of_root = |root: usize| roots.binary_search(&root).map(Body).ok();
        let member_bodies: Vec<Body> = root_of
            .iter()
            .map(|&root| body_of_root(root).ok_or(MechanismError::Internal))
            .collect::<Result<_, _>>()?;
        let body_names: Vec<MemberName> = roots
            .iter()
            .map(|&root| names.get(root).cloned().ok_or(MechanismError::Internal))
            .collect::<Result<_, _>>()?;
        let held: Vec<bool> = roots
            .iter()
            .map(|&root| matches!(members.get(root), Some((_, Holding::Held))))
            .collect();
        let body = |id: MemberId| -> Result<Body, MechanismError> {
            member_bodies
                .get(id.index())
                .copied()
                .ok_or(MechanismError::UnknownMember { member: id })
        };

        // The entries, between bodies.
        let mut entries = Vec::with_capacity(constraints.len());
        for constraint in &constraints {
            let (first, second) = match *constraint {
                Constraint::Pivot { first, second, .. } => (first, second),
                Constraint::Slot { pin, guide, .. } => (pin, guide),
                Constraint::Rail { runner, guide, .. } => (runner, guide),
                Constraint::Couple {
                    follower, leader, ..
                } => (follower, leader),
            };
            let (a, b) = (body(first)?, body(second)?);
            if a == b {
                return Err(MechanismError::OnePiece {
                    first: name(first)?,
                    second: name(second)?,
                });
            }
            if is(&held, a) && is(&held, b) {
                return Err(MechanismError::BothHeld {
                    first: name(first)?,
                    second: name(second)?,
                });
            }
            let entry = match *constraint {
                Constraint::Pivot { at, .. } => Entry::Pivot {
                    first: a,
                    second: b,
                    at,
                },
                Constraint::Slot { from, to, .. } => {
                    if from == to {
                        return Err(MechanismError::SlotHasNoLength {
                            pin: name(first)?,
                            guide: name(second)?,
                        });
                    }
                    Entry::Slot {
                        pin: a,
                        guide: b,
                        from,
                        to,
                    }
                }
                Constraint::Rail { direction, .. } => Entry::Rail {
                    runner: a,
                    guide: b,
                    direction,
                },
                Constraint::Couple { ratio, .. } => Entry::Couple {
                    follower: a,
                    leader: b,
                    ratio: ratio.value(),
                },
            };
            entries.push(entry);
        }

        // One freedom.
        let free = i64::try_from(held.iter().filter(|held| !**held).count())
            .map_err(|_| MechanismError::Overflow)?;
        let taken: i64 = entries.iter().map(|entry| entry.takes()).sum();
        let found = free
            .checked_mul(3)
            .and_then(|all| all.checked_sub(taken))
            .ok_or(MechanismError::Overflow)?;
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
        let turning = drives
            .iter()
            .filter(|d| matches!(d, Drive::Turn { .. }))
            .count();
        if drives.iter().any(|d| d.member() != drive.member())
            || turning > 1
            || drives.len() - turning > 1
        {
            return Err(MechanismError::DrivesApart);
        }

        // The drive.
        let drive_body = body(drive.member())?;
        if is(&held, drive_body) {
            return Err(MechanismError::DriveHeld {
                member: name(drive.member())?,
            });
        }
        if let Some(empty) = drives.iter().find(|d| d.from() == d.to()) {
            return Err(MechanismError::EmptyRange { at: empty.from() });
        }
        let runs_on_rail = |b: Body| {
            entries
                .iter()
                .any(|entry| matches!(entry, Entry::Rail { runner, .. } if *runner == b))
        };
        // A travelling drive runs its member along a rail, or along a slot
        // straight along one of the plane's directions (grammar,
        // "Mechanisms", a travel along a slot).
        let runs_in_slot = |b: Body| {
            entries.iter().any(|entry| {
                matches!(entry, Entry::Slot { pin, from, to, .. }
                    if *pin == b && (from.across == to.across || from.depth == to.depth))
            })
        };
        let travels = drives.iter().any(|d| matches!(d, Drive::Travel { .. }));
        if travels && !runs_on_rail(drive_body) && !runs_in_slot(drive_body) {
            return Err(MechanismError::TravelWithoutRail {
                member: name(drive.member())?,
            });
        }
        for entry in &entries {
            if let Entry::Couple {
                follower, leader, ..
            } = *entry
            {
                for member in [follower, leader] {
                    if !runs_on_rail(member) {
                        return Err(MechanismError::TravelWithoutRail {
                            member: body_names
                                .get(member.0)
                                .cloned()
                                .ok_or(MechanismError::Internal)?,
                        });
                    }
                }
            }
        }

        let mut planner = Planner::new(&entries, &held, &body_names, &drives, drive, drive_body);
        let steps = planner.plan()?;
        let held_values = vec![0.0; drives.len()];
        Ok(Mechanism {
            names,
            member_bodies,
            body_names,
            held,
            drives,
            drive,
            drive_body,
            swept: 0,
            held_values,
            steps,
        })
    }

    /// The same mechanism with drive `swept` run through its range and
    /// every other drive held at `held` (one exact value for each drive,
    /// absolute; the swept one's is not read). A drive past the list, or
    /// held values of the wrong number or kind, are
    /// [`MechanismError::NoSuchDrive`].
    pub fn sweeping(&self, swept: usize, held: &[DriveValue]) -> Result<Mechanism, MechanismError> {
        if swept >= self.drives.len() || held.len() != self.drives.len() {
            return Err(MechanismError::NoSuchDrive);
        }
        let held_values = self
            .drives
            .iter()
            .zip(held)
            .map(|(drive, value)| relative_to(*drive, *value))
            .collect::<Option<Vec<f64>>>()
            .ok_or(MechanismError::NoSuchDrive)?;
        let drive = *self.drives.get(swept).ok_or(MechanismError::NoSuchDrive)?;
        Ok(Mechanism {
            drive,
            swept,
            held_values,
            ..self.clone()
        })
    }

    /// The drive run through its range.
    pub fn drive(&self) -> Drive {
        self.drive
    }

    /// Every drive, in order.
    pub fn drives(&self) -> &[Drive] {
        &self.drives
    }

    /// Every drive's value past its `from` (radians or nanometres), with
    /// the swept one at `relative`.
    pub(super) fn relatives(&self, relative: f64) -> Vec<f64> {
        self.held_values
            .iter()
            .enumerate()
            .map(|(index, held)| if index == self.swept { relative } else { *held })
            .collect()
    }

    /// `true` when every held drive is at its `from`: the swept drive's
    /// `from` is then the closed pose.
    pub(super) fn held_closed(&self) -> bool {
        self.held_values
            .iter()
            .enumerate()
            .all(|(index, held)| index == self.swept || *held == 0.0)
    }

    /// The members' names, in the order declared.
    pub fn names(&self) -> &[MemberName] {
        &self.names
    }

    pub(super) fn member_bodies(&self) -> &[Body] {
        &self.member_bodies
    }

    pub(super) fn body_count(&self) -> usize {
        self.body_names.len()
    }

    pub(super) fn body_name(&self, body: Body) -> MemberName {
        self.body_names
            .get(body.0)
            .cloned()
            .unwrap_or_else(|| MemberName(String::from("?")))
    }

    pub(super) fn is_held(&self, body: Body) -> bool {
        is(&self.held, body)
    }

    pub(super) fn drive_body(&self) -> Body {
        self.drive_body
    }

    /// The index of the drive run through its range.
    pub(super) fn swept(&self) -> usize {
        self.swept
    }

    pub(super) fn steps(&self) -> &[Step] {
        &self.steps
    }

    /// The drive's start and end as exact integers (micro-degrees or
    /// nanometres).
    pub(super) fn range_units(&self) -> (i64, i64) {
        match self.drive() {
            Drive::Turn { from, to, .. } => (from.microdeg(), to.microdeg()),
            Drive::Travel { from, to, .. } => (from.nm(), to.nm()),
        }
    }

    /// A drive value from exact integer units.
    pub(super) fn value_of_units(&self, units: i64) -> DriveValue {
        match self.drive() {
            Drive::Turn { .. } => DriveValue::Turn(stave_lang::units::Angle::from_microdeg(units)),
            Drive::Travel { .. } => DriveValue::Travel(Length::from_nm(units)),
        }
    }
}

/// An exact value of a drive as radians or nanometres past its `from`;
/// `None` for a value of the other kind.
fn relative_to(drive: Drive, value: DriveValue) -> Option<f64> {
    let from = super::DriveAmount::of(drive.from()).value();
    match (drive, value) {
        (Drive::Turn { .. }, DriveValue::Turn(_))
        | (Drive::Travel { .. }, DriveValue::Travel(_)) => {
            Some(super::DriveAmount::of(value).value() - from)
        }
        (Drive::Turn { .. }, DriveValue::Travel(_))
        | (Drive::Travel { .. }, DriveValue::Turn(_)) => None,
    }
}

fn is(flags: &[bool], body: Body) -> bool {
    flags.get(body.0).copied().unwrap_or(false)
}

fn set(flags: &mut [bool], body: Body) {
    if let Some(flag) = flags.get_mut(body.0) {
        *flag = true;
    }
}

/// A locus found for a step, with the entries it uses and the exact shape
/// it has at the closed pose.
#[derive(Debug, Clone, Copy)]
struct Found {
    locus: Locus,
    uses: [Option<usize>; 2],
    rest: Shape,
}

/// A locus at the closed pose, exact, as much as the branch needs: a
/// circle's centre, or a line's direction (the closed point is on both).
#[derive(Debug, Clone, Copy)]
enum Shape {
    Circle { centre: Exact },
    Line { direction: Exact },
}

struct Planner<'a> {
    entries: &'a [Entry],
    names: &'a [MemberName],
    drives: &'a [Drive],
    drive: Drive,
    drive_body: Body,
    used: Vec<bool>,
    placed: Vec<bool>,
    angle_known: Vec<bool>,
    travel_known: Vec<bool>,
    non_turning: Vec<bool>,
}

impl<'a> Planner<'a> {
    fn new(
        entries: &'a [Entry],
        held: &'a [bool],
        names: &'a [MemberName],
        drives: &'a [Drive],
        drive: Drive,
        drive_body: Body,
    ) -> Planner<'a> {
        let count = held.len();
        Planner {
            entries,
            names,
            drives,
            drive,
            drive_body,
            used: vec![false; entries.len()],
            placed: held.to_vec(),
            angle_known: vec![false; count],
            travel_known: vec![false; count],
            non_turning: held.to_vec(),
        }
    }

    fn name(&self, body: Body) -> Result<MemberName, MechanismError> {
        self.names
            .get(body.0)
            .cloned()
            .ok_or(MechanismError::Internal)
    }

    fn plan(&mut self) -> Result<Vec<Step>, MechanismError> {
        for drive in self.drives {
            match drive {
                Drive::Turn { .. } => set(&mut self.angle_known, self.drive_body),
                Drive::Travel { .. } => set(&mut self.travel_known, self.drive_body),
            }
        }
        let mut steps = Vec::new();
        while self.placed.iter().any(|placed| !placed) {
            let step = match self.next_step()? {
                Some(step) => step,
                None => {
                    let left = (0..self.placed.len())
                        .filter(|&b| !is(&self.placed, Body(b)))
                        .map(|b| self.name(Body(b)))
                        .collect::<Result<_, _>>()?;
                    return Err(MechanismError::NoOrder { left });
                }
            };
            steps.push(step);
        }
        if let Some(index) = self.used.iter().position(|used| !used) {
            let entry = self.entries.get(index).ok_or(MechanismError::Internal)?;
            let (first, second) = entry.bodies();
            return Err(MechanismError::Overconstrained {
                first: self.name(first)?,
                second: self.name(second)?,
            });
        }
        Ok(steps)
    }

    fn is_used(&self, entry: usize) -> bool {
        self.used.get(entry).copied().unwrap_or(true)
    }

    fn unplaced_plain(&self, body: Body) -> bool {
        !is(&self.placed, body) && !is(&self.angle_known, body) && !is(&self.travel_known, body)
    }

    fn use_entries(&mut self, uses: &[Option<usize>]) {
        for index in uses.iter().flatten() {
            if let Some(used) = self.used.get_mut(*index) {
                *used = true;
            }
        }
    }

    fn next_step(&mut self) -> Result<Option<Step>, MechanismError> {
        if let Some(step) = self.couple()? {
            return Ok(Some(step));
        }
        if let Some(step) = self.hang() {
            return Ok(Some(step));
        }
        if let Some(step) = self.rail() {
            return Ok(Some(step));
        }
        if let Some(step) = self.offset()? {
            return Ok(Some(step));
        }
        if let Some(step) = self.joint()? {
            return Ok(Some(step));
        }
        self.slot_in_link()
    }

    /// A couple with one side's travel known passes it to the other.
    fn couple(&mut self) -> Result<Option<Step>, MechanismError> {
        for (index, entry) in self.entries.iter().enumerate() {
            if self.is_used(index) {
                continue;
            }
            let Entry::Couple {
                follower,
                leader,
                ratio,
            } = *entry
            else {
                continue;
            };
            let (from, to, factor) = match (
                is(&self.travel_known, leader),
                is(&self.travel_known, follower),
            ) {
                (true, false) => (leader, follower, float_ratio(ratio)),
                (false, true) => (
                    follower,
                    leader,
                    float_ratio(
                        Rational::ONE
                            .checked_div(ratio)
                            .ok_or(MechanismError::Overflow)?,
                    ),
                ),
                (true, true) | (false, false) => continue,
            };
            let factor = factor.ok_or(MechanismError::Overflow)?;
            set(&mut self.travel_known, to);
            self.use_entries(&[Some(index)]);
            return Ok(Some(Step::Couple { from, to, factor }));
        }
        Ok(None)
    }

    /// A member whose angle is known, pinned at a known point: by a pivot
    /// on a placed member, or by a slot in a placed member when its travel
    /// along the slot is known.
    fn hang(&mut self) -> Option<Step> {
        for (index, entry) in self.entries.iter().enumerate() {
            if self.is_used(index) {
                continue;
            }
            match *entry {
                Entry::Pivot { first, second, at } => {
                    for (body, known) in [(first, second), (second, first)] {
                        if !is(&self.placed, body)
                            && is(&self.angle_known, body)
                            && is(&self.placed, known)
                        {
                            set(&mut self.placed, body);
                            self.use_entries(&[Some(index)]);
                            return Some(Step::Hang {
                                body,
                                known,
                                pin: float(at),
                            });
                        }
                    }
                }
                Entry::Slot {
                    pin,
                    guide,
                    from,
                    to,
                } => {
                    // A positive travel runs the way the slot's frame
                    // direction does (as a rail along that word).
                    let unit = if from.depth == to.depth {
                        DVec2::X
                    } else if from.across == to.across {
                        DVec2::Y
                    } else {
                        continue;
                    };
                    if !is(&self.placed, pin)
                        && is(&self.angle_known, pin)
                        && is(&self.travel_known, pin)
                        && is(&self.placed, guide)
                    {
                        set(&mut self.placed, pin);
                        self.use_entries(&[Some(index)]);
                        return Some(Step::HangInSlot {
                            body: pin,
                            guide,
                            from: float(from),
                            unit,
                            to: float(to),
                        });
                    }
                }
                Entry::Rail { .. } | Entry::Couple { .. } => {}
            }
        }
        None
    }

    /// A member whose travel is known, on a rail along a placed member
    /// that doesn't turn.
    fn rail(&mut self) -> Option<Step> {
        for (index, entry) in self.entries.iter().enumerate() {
            if self.is_used(index) {
                continue;
            }
            let Entry::Rail {
                runner,
                guide,
                direction,
            } = *entry
            else {
                continue;
            };
            if !is(&self.placed, runner)
                && is(&self.travel_known, runner)
                && is(&self.placed, guide)
                && is(&self.non_turning, guide)
            {
                set(&mut self.placed, runner);
                set(&mut self.non_turning, runner);
                self.use_entries(&[Some(index)]);
                return Some(Step::Rail {
                    runner,
                    guide,
                    direction: float_direction(direction).normalize_or_zero(),
                });
            }
        }
        None
    }

    /// `body`'s path for a point on it at `joint`, through an unused entry
    /// other than `except` to a placed member: a circle (a pivot) or a line
    /// (a rail).
    fn link_locus(
        &self,
        body: Body,
        joint: PlanarPoint,
        except: usize,
        offset: bool,
    ) -> Result<Option<Found>, MechanismError> {
        let exact_joint = Exact::of(joint);
        let shift = if offset { exact_joint } else { Exact::ZERO };
        for (index, entry) in self.entries.iter().enumerate() {
            if index == except || self.is_used(index) {
                continue;
            }
            match *entry {
                Entry::Pivot { first, second, at } => {
                    let known = if first == body {
                        second
                    } else if second == body {
                        first
                    } else {
                        continue;
                    };
                    if !is(&self.placed, known) {
                        continue;
                    }
                    if at == joint {
                        return Err(MechanismError::LinkHasNoLength {
                            member: self.name(body)?,
                        });
                    }
                    let (pivot, joint_float) = (float(at), float(joint));
                    return Ok(Some(Found {
                        locus: Locus::Link {
                            link: body,
                            known,
                            pivot,
                            joint: joint_float,
                            radius: (joint_float - pivot).length(),
                        },
                        uses: [Some(index), None],
                        rest: Shape::Circle {
                            centre: Exact::of(at).minus(shift).ok_or(MechanismError::Overflow)?,
                        },
                    }));
                }
                Entry::Rail {
                    runner,
                    guide,
                    direction,
                } if runner == body && is(&self.placed, guide) => {
                    return Ok(Some(Found {
                        locus: Locus::Rail {
                            runner,
                            guide,
                            direction: float_direction(direction),
                            joint: float(joint),
                        },
                        uses: [Some(index), None],
                        rest: Shape::Line {
                            direction: Exact::of_direction(direction),
                        },
                    }));
                }
                Entry::Rail { .. } | Entry::Slot { .. } | Entry::Couple { .. } => {}
            }
        }
        Ok(None)
    }

    /// A member whose angle is known, hung on two paths.
    fn offset(&mut self) -> Result<Option<Step>, MechanismError> {
        for b in 0..self.placed.len() {
            let body = Body(b);
            if is(&self.placed, body) || !is(&self.angle_known, body) {
                continue;
            }
            let mut found: Vec<Found> = Vec::new();
            for (index, entry) in self.entries.iter().enumerate() {
                if self.is_used(index) {
                    continue;
                }
                match *entry {
                    Entry::Pivot { first, second, at } => {
                        let other = if first == body {
                            second
                        } else if second == body {
                            first
                        } else {
                            continue;
                        };
                        if !self.unplaced_plain(other) {
                            continue;
                        }
                        if let Some(mut link) = self.link_locus(other, at, index, true)? {
                            link.uses[1] = Some(index);
                            found.push(link);
                        }
                    }
                    Entry::Slot {
                        pin,
                        guide,
                        from,
                        to,
                    } => {
                        let (exact_from, exact_to) = (Exact::of(from), Exact::of(to));
                        let slot = exact_to.minus(exact_from).ok_or(MechanismError::Overflow)?;
                        if pin == body && is(&self.placed, guide) {
                            found.push(Found {
                                locus: Locus::Slot {
                                    pin,
                                    guide,
                                    from: float(from),
                                    to: float(to),
                                },
                                uses: [Some(index), None],
                                rest: Shape::Line { direction: slot },
                            });
                        } else if guide == body && is(&self.placed, pin) {
                            found.push(Found {
                                locus: Locus::SlotInUnknown {
                                    pin,
                                    guide,
                                    from: float(from),
                                    to: float(to),
                                },
                                uses: [Some(index), None],
                                rest: Shape::Line {
                                    direction: slot.negated().ok_or(MechanismError::Overflow)?,
                                },
                            });
                        }
                    }
                    Entry::Rail { .. } | Entry::Couple { .. } => {}
                }
            }
            if let Some((first, second)) = pair(&found) {
                return self
                    .dyad(Unknown::Offset(body), first, second, Exact::ZERO)
                    .map(Some);
            }
        }
        Ok(None)
    }

    /// A point where two unplaced members meet, or a slot's pin on an
    /// unplaced link, on two paths.
    fn joint(&mut self) -> Result<Option<Step>, MechanismError> {
        for (index, entry) in self.entries.iter().enumerate() {
            if self.is_used(index) {
                continue;
            }
            match *entry {
                Entry::Pivot { first, second, at } => {
                    if !self.unplaced_plain(first) || !self.unplaced_plain(second) {
                        continue;
                    }
                    let (Some(mut a), Some(b)) = (
                        self.link_locus(first, at, index, false)?,
                        self.link_locus(second, at, index, false)?,
                    ) else {
                        continue;
                    };
                    if !a.locus.is_circle() && !b.locus.is_circle() {
                        continue;
                    }
                    a.uses[1] = Some(index);
                    return self.dyad(Unknown::Joint, a, b, Exact::of(at)).map(Some);
                }
                Entry::Slot {
                    pin,
                    guide,
                    from,
                    to,
                } => {
                    if !self.unplaced_plain(pin) || !is(&self.placed, guide) {
                        continue;
                    }
                    let Some(link) = self.link_locus(pin, from, index, false)? else {
                        continue;
                    };
                    if !link.locus.is_circle() {
                        continue;
                    }
                    let (exact_from, exact_to) = (Exact::of(from), Exact::of(to));
                    let slot = Found {
                        locus: Locus::Slot {
                            pin,
                            guide,
                            from: float(from),
                            to: float(to),
                        },
                        uses: [Some(index), None],
                        rest: Shape::Line {
                            direction: exact_to
                                .minus(exact_from)
                                .ok_or(MechanismError::Overflow)?,
                        },
                    };
                    return self.dyad(Unknown::Joint, link, slot, exact_from).map(Some);
                }
                Entry::Rail { .. } | Entry::Couple { .. } => {}
            }
        }
        Ok(None)
    }

    /// A link pinned to a placed member, its slot over a placed pin.
    fn slot_in_link(&mut self) -> Result<Option<Step>, MechanismError> {
        for (index, entry) in self.entries.iter().enumerate() {
            if self.is_used(index) {
                continue;
            }
            let Entry::Slot {
                pin,
                guide,
                from,
                to,
            } = *entry
            else {
                continue;
            };
            if !self.unplaced_plain(guide) || !is(&self.placed, pin) {
                continue;
            }
            let Some(Found {
                locus: Locus::Link { known, pivot, .. },
                uses,
                rest: Shape::Circle { centre },
            }) = self.link_locus(guide, from, index, false)?
            else {
                continue;
            };
            let (exact_from, exact_to) = (Exact::of(from), Exact::of(to));
            let along = exact_to.minus(exact_from).ok_or(MechanismError::Overflow)?;
            let reach = exact_from.minus(centre).ok_or(MechanismError::Overflow)?;
            let side = sign(reach.dot(along).ok_or(MechanismError::Overflow)?);
            let Some(side) = side else {
                return Err(MechanismError::DeadPoint {
                    members: vec![self.name(guide)?, self.name(pin)?],
                    at: self.drive.from(),
                });
            };
            set(&mut self.placed, guide);
            self.use_entries(&[uses[0], Some(index)]);
            return Ok(Some(Step::SlotInLink(SlotInLink {
                link: guide,
                known,
                pivot,
                pin,
                from: float(from),
                to: float(to),
                side,
            })));
        }
        Ok(None)
    }

    /// Builds a two-path step: the branch the closed pose is on, exactly,
    /// and marks what it places.
    fn dyad(
        &mut self,
        unknown: Unknown,
        first: Found,
        second: Found,
        rest_point: Exact,
    ) -> Result<Step, MechanismError> {
        let side = match (first.rest, second.rest) {
            (Shape::Circle { centre: c1 }, Shape::Circle { centre: c2 }) => {
                let between = c2.minus(c1).ok_or(MechanismError::Overflow)?;
                let to_point = rest_point.minus(c1).ok_or(MechanismError::Overflow)?;
                sign(between.cross(to_point).ok_or(MechanismError::Overflow)?)
            }
            (Shape::Circle { centre }, Shape::Line { direction })
            | (Shape::Line { direction }, Shape::Circle { centre }) => {
                let to_point = rest_point.minus(centre).ok_or(MechanismError::Overflow)?;
                sign(to_point.dot(direction).ok_or(MechanismError::Overflow)?)
            }
            (Shape::Line { .. }, Shape::Line { .. }) => return Err(MechanismError::Internal),
        };
        let mut members = Vec::with_capacity(3);
        if let Unknown::Offset(body) = unknown {
            members.push(body);
        }
        members.extend(first.locus.places());
        members.extend(second.locus.places());
        let Some(side) = side else {
            return Err(MechanismError::DeadPoint {
                members: members
                    .iter()
                    .map(|&b| self.name(b))
                    .collect::<Result<_, _>>()?,
                at: self.drive.from(),
            });
        };
        let guide_turns = |locus: Locus, planner: &Planner<'_>| match locus {
            Locus::Rail { guide, .. } => !is(&planner.non_turning, guide),
            Locus::Link { .. } | Locus::Slot { .. } | Locus::SlotInUnknown { .. } => true,
        };
        for found in [first, second] {
            if let Some(body) = found.locus.places() {
                set(&mut self.placed, body);
                if !guide_turns(found.locus, self) {
                    set(&mut self.non_turning, body);
                }
            }
        }
        if let Unknown::Offset(body) = unknown {
            set(&mut self.placed, body);
        }
        self.use_entries(&first.uses);
        self.use_entries(&second.uses);
        Ok(Step::Dyad(Dyad {
            unknown,
            first: first.locus,
            second: second.locus,
            side,
            members,
        }))
    }
}

/// The first two loci, in order, that place different members, use
/// different entries, and are not both lines.
fn pair(found: &[Found]) -> Option<(Found, Found)> {
    for (i, first) in found.iter().enumerate() {
        for second in found.iter().skip(i + 1) {
            let distinct_members = match (first.locus.places(), second.locus.places()) {
                (Some(a), Some(b)) => a != b,
                _ => true,
            };
            let shares_entry = first
                .uses
                .iter()
                .flatten()
                .any(|use_a| second.uses.iter().flatten().any(|use_b| use_a == use_b));
            if distinct_members
                && !shares_entry
                && (first.locus.is_circle() || second.locus.is_circle())
            {
                return Some((*first, *second));
            }
        }
    }
    None
}

/// `+1` or `-1`; `None` for zero.
fn sign(value: i128) -> Option<f64> {
    match value.cmp(&0) {
        std::cmp::Ordering::Greater => Some(1.0),
        std::cmp::Ordering::Less => Some(-1.0),
        std::cmp::Ordering::Equal => None,
    }
}
