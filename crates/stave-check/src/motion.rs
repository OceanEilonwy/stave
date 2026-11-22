//! Motion sweeps (layer 5): every drive of every assembly with a motion is
//! moved through its whole travel, from rest and from each scene's poses,
//! and the first thing it hits is reported with the angle or distance
//! where it hits.
//!
//! Motions are made one at a time (grammar, "Scenes"): the swept drive
//! starts closed, every other drive of its assembly, and every drive of
//! the moving assemblies it is inside or carries, is held still where the
//! start puts it (closed from rest, where a scene poses it from that
//! scene, at each of its set values from rest; [`Checker::sweep_starts`]
//! lists them), and everything else stands where the scene puts it. A
//! motion inside a moving assembly is swept with its parent held, in the
//! parent's place; the parent is swept carrying it where it is held. A
//! loose member is never swept; it is checked where it is set
//! ([`Checker::set_positions`]). Every body the motion moves (its parts,
//! the hardware on them, and the links its hardware's mechanism turns) is
//! swept against every body that moves differently, the same way for
//! every kind (grammar, "One collision model"). Each drive of an assembly
//! is reported once: the first start (rest, then the scenes in order) at
//! which it hits something, with its first contact. An assembly with an
//! item already reported at rest or at a scene's poses is left out.

use stave_geom::collide::{
    Placement, WorldSolid, boxes_overlap, overlaps_placed, placed_bounds, union,
};
use stave_model::mechanism::MotionShape;
use std::borrow::Cow;
use std::collections::BTreeSet;

use stave_geom::planar::{MechanismError, MechanismErrorKind};
use stave_geom::sweep::{Mover, Path, Reach, first_contact};
use stave_lang::placement::Direction;
use stave_lang::units::{Angle, Length};
use stave_lang::{Code, Severity};
use stave_model::mechanism::{DriveIndex, DriveValue, Mechanism, MemberIndex};
use stave_model::solved::{AssemblyId, BodyOwner, PartId, PoseValue, SolvedDesign, WorldBox};

use crate::fixes::OverlayChange;
use crate::names::length_text;
use crate::{Checker, Contact, ContactMotion, Item};

/// What decides which hit comes first: the distance or angle, then parts
/// before hardware (a drawer side rather than the slide on it).
fn reach_key(reach: &Reach, a: Item, b: Item) -> (i64, bool, bool, Item, Item) {
    let amount = match reach {
        Reach::Distance(hit) => hit.distance.nm(),
        Reach::Angle(angle) => angle.microdeg(),
    };
    let hardware = |item: Item| !matches!(item, Item::Part(_));
    (amount, hardware(a), hardware(b), a, b)
}

/// Where one drive is swept from: a scene (0 for rest), every drive's
/// exact value there (the swept one's is its `from`), and where the
/// moving assemblies it is inside or carries are held.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SweepStart {
    /// The scene whose placements the rest of the design takes.
    pub scene_index: usize,
    /// Every drive of the assembly, held.
    pub held: Vec<DriveValue>,
    /// Each moving assembly it is inside or carries, outermost first, with
    /// every one of its drives' poses, held.
    pub others: Vec<(AssemblyId, Vec<PoseValue>)>,
}

/// A body a sweep moves: its index, its solid where the assemblies inside
/// the swept one hold it (moved, when they only moved it), and their
/// placement when they turned it.
struct Moving<'s> {
    index: usize,
    solid: Cow<'s, WorldSolid>,
    held: Option<Placement>,
}

/// Every drive's pose for exact drive values: how far each is from its
/// `from` (closed at its `from`).
fn pose_values(mechanism: &Mechanism, values: &[DriveValue]) -> Vec<PoseValue> {
    mechanism
        .drives()
        .iter()
        .zip(values)
        .map(|(named, value)| match named.drive.amount_of(*value) {
            Some(DriveValue::Turn(angle)) if angle != Angle::ZERO => PoseValue::Angle(angle),
            Some(DriveValue::Travel(length)) if length != Length::ZERO => PoseValue::Travel(length),
            Some(DriveValue::Turn(_) | DriveValue::Travel(_)) | None => PoseValue::Closed,
        })
        .collect()
}

/// An angle rounded down to a tenth of a degree: `87.3deg`.
fn angle_text(angle: Angle) -> String {
    let tenth = 100_000;
    Angle::from_microdeg(angle.microdeg().div_euclid(tenth) * tenth).to_string()
}

impl Checker<'_> {
    /// An assembly's mechanism, ready to pose and sweep; `None` when
    /// nothing moves it.
    pub(crate) fn path(&self, assembly: AssemblyId) -> Option<&Path> {
        self.paths.get(assembly.index())?.as_ref()
    }

    /// The piece of hardware with a drive that gives an assembly its
    /// motion (the first, when several move it alike).
    fn motion_hardware(&self, assembly: AssemblyId) -> Option<stave_model::solved::HardwareId> {
        let found = self.design.assembly(assembly)?;
        found.mechanism.as_ref()?.hardware().first().copied()
    }

    /// Mechanisms that can't move through their range (grammar, "How a
    /// mechanism is solved"): a dead point, a branch that flips or a pin
    /// that runs off its slot is `error[motion]` at the hardware; a
    /// mechanism with no order of steps is `error[mechanism]` at its kind.
    pub(crate) fn mechanisms(&mut self) {
        let mut errors: Vec<(stave_model::solved::HardwareId, MechanismError)> = Vec::new();
        for (assembly, error) in &self.broken {
            if let Some(hardware) = self.motion_hardware(*assembly) {
                errors.push((hardware, error.clone()));
            }
        }
        // Hardware no assembly moves is still run through its own range.
        for hardware in self.design.hardware() {
            let Some(own) = &hardware.own_mechanism else {
                continue;
            };
            let covered = hardware.assembly.is_some_and(|assembly| {
                self.path(assembly).is_some() || self.broken.iter().any(|(a, _)| *a == assembly)
            });
            if covered {
                continue;
            }
            if let Err(error) = Path::new(own) {
                errors.push((hardware.id, error));
            }
        }
        let mut kinds: BTreeSet<String> = BTreeSet::new();
        for (hardware, error) in errors {
            let Some(found) = self.design.hardware_piece(hardware) else {
                continue;
            };
            let (hardware_name, kind) = (found.name.clone(), found.kind.clone());
            match error.kind() {
                MechanismErrorKind::Motion => {
                    let place = self.hardware_place(hardware);
                    self.report(
                        Severity::Error,
                        Code::Motion,
                        format!("{hardware_name}: {error}"),
                        place,
                        Vec::new(),
                        Some(
                            "give its kind a smaller range, or move its pins so they don't line up"
                                .to_owned(),
                        ),
                    );
                }
                MechanismErrorKind::Mechanism => {
                    if !kinds.insert(kind.clone()) {
                        continue;
                    }
                    let place = self
                        .kind_mechanism_place(hardware)
                        .or_else(|| self.hardware_place(hardware));
                    self.report(
                        Severity::Error,
                        Code::Mechanism,
                        format!("{kind}'s mechanism: {error}"),
                        place,
                        Vec::new(),
                        Some(
                            "pin each member to one that is held or driven, or to one pinned to those, so the members can be placed one step at a time"
                                .to_owned(),
                        ),
                    );
                }
            }
        }
    }

    /// Where the kind of a piece of hardware writes its mechanism.
    fn kind_mechanism_place(
        &self,
        hardware: stave_model::solved::HardwareId,
    ) -> Option<(stave_lang::FileId, stave_lang::ast::Span)> {
        let kind_id = self.design.hardware_piece(hardware)?.kind_id;
        let kind = self.model.defs.kind(kind_id)?;
        Some((kind.decl.file, kind.mechanism()?.span))
    }

    /// The assemblies with a motion that move a part, outermost first:
    /// two parts move alike when these are the same.
    fn motions_of(&self, part: PartId) -> Vec<AssemblyId> {
        self.bodies
            .iter()
            .find(|body| body.item == Item::Part(part))
            .map(|body| {
                body.chain
                    .iter()
                    .copied()
                    .filter(|assembly| {
                        self.design
                            .assemblies()
                            .get(assembly.index())
                            .is_some_and(|found| found.mechanism.is_some())
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// A joint's parts must move alike (grammar, "Scenes"): a joint
    /// between a part that moves with an assembly's motion and a part
    /// that moves differently would hold the assembly shut, which is
    /// `error[motion]` at the joint, once for each joint with the other
    /// pairs as notes. Its parts and members are then left out of the
    /// scene and sweep checks: one mistake, one error.
    pub(crate) fn joint_motions(&mut self) {
        let mut found = Vec::new();
        for joint in self.design.joints() {
            let mut crossing: Vec<(PartId, PartId)> = Vec::new();
            for pair in &joint.pairs {
                let parts: Vec<PartId> = pair.roles.iter().map(|role| role.part).collect();
                for (index, a) in parts.iter().enumerate() {
                    for b in parts.iter().skip(index + 1) {
                        if a != b
                            && self.motions_of(*a) != self.motions_of(*b)
                            && !crossing.contains(&(*a, *b))
                        {
                            crossing.push((*a, *b));
                        }
                    }
                }
            }
            if !crossing.is_empty() {
                found.push((joint.id, crossing));
            }
        }
        for (joint, crossing) in found {
            let joint_name = self.joint_name(joint);
            let sentence = |(a, b): (PartId, PartId)| {
                let (a_name, b_name) = (self.part_name(a), self.part_name(b));
                let (a_moves, b_moves) = (self.motions_of(a), self.motions_of(b));
                match (a_moves.last(), b_moves.last()) {
                    (Some(x), Some(y)) if x != y => format!(
                        "{a_name} moves with {} but {b_name} moves with {}, so the joint would hold them both still",
                        self.assembly_name(*x),
                        self.assembly_name(*y)
                    ),
                    (Some(x), _) => format!(
                        "{a_name} moves with {} but {b_name} doesn't, so the joint would hold {} shut",
                        self.assembly_name(*x),
                        self.assembly_name(*x)
                    ),
                    (None, Some(y)) => format!(
                        "{b_name} moves with {} but {a_name} doesn't, so the joint would hold {} shut",
                        self.assembly_name(*y),
                        self.assembly_name(*y)
                    ),
                    (None, None) => format!("{a_name} and {b_name} move differently"),
                }
            };
            let Some((&first, rest)) = crossing.split_first() else {
                continue;
            };
            let notes: Vec<String> = rest
                .iter()
                .map(|pair| format!("and {}", sentence(*pair)))
                .collect();
            let message = format!("{joint_name}: {}", sentence(first));
            let parts: Vec<PartId> = crossing.iter().flat_map(|(a, b)| [*a, *b]).collect();
            let items: Vec<Item> = parts.iter().map(|part| Item::Part(*part)).collect();
            self.failed.extend(items.iter().copied());
            let members: Vec<Item> = self
                .bodies
                .iter()
                .filter(|body| {
                    matches!(body.item.owner(),
                        Some(BodyOwner::Joint { joint: owner, .. }) if owner == joint)
                })
                .map(|body| body.item)
                .collect();
            self.failed.extend(members);
            self.next_contact = Some(Contact::of(self.design, &items, &[]));
            let place = self.joint_place(joint);
            self.report(
                Severity::Error,
                Code::Motion,
                message,
                place,
                notes,
                Some(format!(
                    "take '{joint_name}' out, or join these parts only to parts that move with them"
                )),
            );
        }
    }

    /// The moving assemblies an assembly is inside or carries (grammar,
    /// "Scenes", motions are made one at a time): its moving ancestors,
    /// outermost first, then the moving assemblies inside it, in the
    /// design's order.
    fn related(&self, assembly: AssemblyId) -> Vec<AssemblyId> {
        let mut outer = Vec::new();
        let mut parent = self.design.assembly(assembly).and_then(|a| a.parent);
        while let Some(id) = parent {
            if outer.contains(&id) {
                break;
            }
            outer.push(id);
            parent = self.design.assembly(id).and_then(|a| a.parent);
        }
        outer.reverse();
        let inside = |id: AssemblyId| {
            let mut parent = self.design.assembly(id).and_then(|a| a.parent);
            let mut seen = 0;
            while let Some(at) = parent {
                if at == assembly {
                    return true;
                }
                seen += 1;
                if seen > self.design.assemblies().len() {
                    return false;
                }
                parent = self.design.assembly(at).and_then(|a| a.parent);
            }
            false
        };
        outer
            .into_iter()
            .chain(
                self.design
                    .assemblies()
                    .iter()
                    .map(|a| a.id)
                    .filter(|id| inside(*id)),
            )
            .filter(|id| self.path(*id).is_some())
            .collect()
    }

    /// The starts one drive of an assembly is swept from (grammar,
    /// "Scenes", motions are made one at a time): rest, with every other
    /// drive at its `from` and the moving assemblies it is inside or
    /// carries closed, then each scene in order, with every other drive,
    /// and those assemblies, held where the scene poses them (closed where
    /// it doesn't), then each set value of each other drive, and of each
    /// of those assemblies' drives, from rest with every drive but that
    /// one closed ([`Checker::set_holds`]). A start the same as an earlier
    /// one from rest is left out.
    pub(crate) fn sweep_starts(
        &self,
        assembly: AssemblyId,
        mechanism: &Mechanism,
        drive: DriveIndex,
    ) -> Vec<SweepStart> {
        let closed = mechanism.closed();
        let related: Vec<(AssemblyId, &Mechanism)> = self
            .related(assembly)
            .into_iter()
            .filter_map(|id| Some((id, self.design.assembly(id)?.mechanism.as_ref()?)))
            .collect();
        let shut =
            |id: AssemblyId, other: &Mechanism| (id, vec![PoseValue::Closed; other.drives().len()]);
        let all_shut: Vec<(AssemblyId, Vec<PoseValue>)> =
            related.iter().map(|(id, other)| shut(*id, other)).collect();
        let mut starts = vec![SweepStart {
            scene_index: 0,
            held: closed.clone(),
            others: all_shut.clone(),
        }];
        for (scene_index, scene) in self.design.scenes.iter().enumerate().skip(1) {
            let (poses, _) = scene.poses_of(assembly, mechanism.drives().len());
            let mut held = mechanism.values_of(&poses);
            if let (Some(own), Some(from)) =
                (held.get_mut(drive.index()), closed.get(drive.index()))
            {
                *own = *from;
            }
            let others = related
                .iter()
                .map(|(id, other)| (*id, scene.poses_of(*id, other.drives().len()).0))
                .collect();
            starts.push(SweepStart {
                scene_index,
                held,
                others,
            });
        }
        let mut from_rest: Vec<SweepStart> = Self::set_holds(mechanism, Some(drive))
            .into_iter()
            .map(|held| SweepStart {
                scene_index: 0,
                held,
                others: all_shut.clone(),
            })
            .collect();
        for (index, (id, other)) in related.iter().enumerate() {
            for values in Self::set_holds(other, None) {
                let mut others = all_shut.clone();
                if let Some(slot) = others.get_mut(index) {
                    *slot = (*id, pose_values(other, &values));
                }
                from_rest.push(SweepStart {
                    scene_index: 0,
                    held: closed.clone(),
                    others,
                });
            }
        }
        for start in from_rest {
            if !starts
                .iter()
                .any(|earlier| earlier.scene_index == 0 && *earlier == start)
            {
                starts.push(start);
            }
        }
        starts
    }

    /// Every body's placement, and what moves it, with the swept assembly
    /// held at `start` (its swept drive at its `from`), the assemblies it
    /// is inside or carries where `start` holds them, and everything else
    /// where the start's scene puts it.
    fn start_moves(
        &self,
        assembly: AssemblyId,
        mechanism: &Mechanism,
        start: &SweepStart,
    ) -> (Vec<crate::Moves>, Vec<Placement>) {
        let own = pose_values(mechanism, &start.held);
        let scene = self
            .design
            .scenes
            .get(start.scene_index)
            .filter(|_| start.scene_index > 0);
        self.moves_with(&|id: AssemblyId| {
            if id == assembly {
                return Some(own.clone());
            }
            if let Some((_, values)) = start.others.iter().find(|(other, _)| *other == id) {
                return Some(values.clone());
            }
            let drives = self.path(id)?.mechanism().drives().len();
            scene.map(|scene| scene.poses_of(id, drives).0)
        })
    }

    /// Every drive's values with one drive at one of its set values
    /// (other than its `from`) and every other drive closed, drive by
    /// drive and value by value (grammar, "Mechanisms", set positions):
    /// where a loose member is checked, and, leaving out `except`'s own,
    /// where the sweeps of `except` start.
    pub(crate) fn set_holds(
        mechanism: &Mechanism,
        except: Option<DriveIndex>,
    ) -> Vec<Vec<DriveValue>> {
        let closed = mechanism.closed();
        let mut holds = Vec::new();
        for (index, named) in mechanism.drives().iter().enumerate() {
            if except.is_some_and(|drive| drive.index() == index) {
                continue;
            }
            for value in named.set_values() {
                if value == named.drive.from() {
                    continue;
                }
                let mut held = closed.clone();
                if let Some(slot) = held.get_mut(index) {
                    *slot = value;
                }
                holds.push(held);
            }
        }
        holds
    }

    /// Loose members where they are set (grammar, "Shapes": a loose
    /// member is checked wherever it is set and never swept): at each set
    /// value of each drive, every other drive closed and everything else
    /// at rest, against everything that moves differently. Rest and the
    /// scenes check them where those put them. A hit is `error[motion]`
    /// at the loose member's hardware, once for each two items.
    pub(crate) fn set_positions(&mut self) {
        if !self.bodies.iter().any(|body| body.loose) {
            return;
        }
        let mut reported: BTreeSet<(Item, Item)> = BTreeSet::new();
        let mut found = Vec::new();
        for assembly in self.design.assemblies() {
            let (Some(path), Some(mechanism)) = (self.path(assembly.id), &assembly.mechanism)
            else {
                continue;
            };
            for held in Self::set_holds(mechanism, None) {
                let poses: Vec<PoseValue> = mechanism
                    .drives()
                    .iter()
                    .zip(&held)
                    .map(|(named, value)| match named.drive.amount_of(*value) {
                        Some(DriveValue::Turn(angle)) => PoseValue::Angle(angle),
                        Some(DriveValue::Travel(length)) => PoseValue::Travel(length),
                        None => PoseValue::Closed,
                    })
                    .collect();
                // Each body where the held values put it: those the
                // assembly moves with it (a link as its linkage places
                // it), everything else at rest.
                let moved = |index: usize| -> Option<(bool, Option<MemberIndex>)> {
                    let body = self.bodies.get(index)?;
                    let moves = body.chain.contains(&assembly.id);
                    let link = body
                        .link
                        .filter(|(own, _)| moves && *own == assembly.id)
                        .map(|(_, link)| link);
                    Some((moves, link))
                };
                let placement = |index: usize| -> Placement {
                    match moved(index) {
                        Some((true, link)) => {
                            Placement::of_poses(&[(path.mechanism(), poses.as_slice())], link)
                        }
                        Some((false, _)) | None => Placement::REST,
                    }
                };
                for (i, loose) in self.bodies.iter().enumerate() {
                    if !loose.loose || self.failed.contains(&loose.item) {
                        continue;
                    }
                    let at = placement(i);
                    let reach = placed_bounds(&loose.bounds, &at);
                    for (j, other) in self.bodies.iter().enumerate() {
                        if i == j
                            || moved(i) == moved(j)
                            || self.failed.contains(&other.item)
                            || self.exempt(loose.item, other.item)
                            || reported.contains(&crate::ordered_items(loose.item, other.item))
                        {
                            continue;
                        }
                        let other_at = placement(j);
                        if !boxes_overlap(&reach, &placed_bounds(&other.bounds, &other_at)) {
                            continue;
                        }
                        let overlaps = overlaps_placed(&loose.solid, &at, &other.solid, &other_at);
                        if overlaps.is_empty() {
                            continue;
                        }
                        reported.insert(crate::ordered_items(loose.item, other.item));
                        found.push((assembly.id, held.clone(), loose.item, other.item, overlaps));
                    }
                }
            }
        }
        for (assembly, held, loose, other, overlaps) in found {
            self.report_set_position(assembly, &held, loose, other, &overlaps);
        }
    }

    /// A loose member that hits something where a drive sets it.
    fn report_set_position(
        &mut self,
        assembly: AssemblyId,
        held: &[DriveValue],
        loose: Item,
        other: Item,
        overlaps: &[stave_geom::collide::Overlap],
    ) {
        let Some(mechanism) = self
            .design
            .assembly(assembly)
            .and_then(|found| found.mechanism.as_ref())
        else {
            return;
        };
        let name = self.assembly_name(assembly);
        let closed = mechanism.closed();
        // The one drive away from closed, as a message names it.
        let Some((named, value)) = mechanism
            .drives()
            .iter()
            .zip(held.iter().zip(&closed))
            .find(|(_, (value, from))| value != from)
            .map(|(named, (value, _))| (named, *value))
        else {
            return;
        };
        let drive = match &named.name {
            Some(drive) if mechanism.drives().len() > 1 => format!("{name}.{drive}"),
            Some(_) | None => name,
        };
        let amount = match named.drive.amount_of(value) {
            Some(DriveValue::Turn(angle)) => angle.to_string(),
            Some(DriveValue::Travel(length)) => length_text(length),
            None => String::new(),
        };
        let (loose_name, other_name) = (self.item_name(loose), self.item_name(other));
        let mut notes = Vec::new();
        if let Some(deepest) = crate::deepest(overlaps) {
            notes.push(format!(
                "{loose_name} goes {} into {other_name} there",
                length_text(deepest.depth)
            ));
        }
        notes.push(format!(
            "{loose_name} is loose: it is checked where it is set, not swept"
        ));
        let place = loose
            .hardware()
            .and_then(|hardware| self.hardware_place(hardware))
            .or_else(|| {
                self.instance_place(self.mirror_root(assembly))
                    .map(|p| (p.file, p.span))
            });
        self.next_contact = Some(crate::Contact::of(self.design, &[loose, other], overlaps));
        self.report(
            Severity::Error,
            Code::Motion,
            format!("{loose_name} hits {other_name} when {drive} is set at {amount}"),
            place,
            notes,
            Some(format!(
                "make room for {loose_name} at every value {drive} is set at, or leave {amount} out of its set_at:"
            )),
        );
    }

    /// Stops early, leaving the result partial, once `stopped` says so; it
    /// is read before each motion and each scene of it.
    pub(crate) fn sweeps(&mut self, stopped: &dyn Fn() -> bool) {
        let mut found = Vec::new();
        for assembly in self.design.assemblies() {
            if stopped() {
                return;
            }
            let Some(path) = self.path(assembly.id) else {
                continue;
            };
            let Some(mechanism) = assembly.mechanism.as_ref() else {
                continue;
            };
            // A loose member is never swept: it is out while things move.
            let moving: Vec<usize> = (0..self.bodies.len())
                .filter(|&i| {
                    self.bodies
                        .get(i)
                        .is_some_and(|b| b.chain.contains(&assembly.id) && !b.loose)
                })
                .collect();
            let blocked = moving.iter().any(|&i| {
                self.bodies.get(i).is_some_and(|b| {
                    self.failed.contains(&b.item) || self.posed_failed.contains(&b.item)
                })
            });
            if moving.is_empty() || blocked {
                continue;
            }
            for drive in (0..mechanism.drives().len()).filter_map(DriveIndex::new) {
                for start in self.sweep_starts(assembly.id, mechanism, drive) {
                    if stopped() {
                        return;
                    }
                    if start.scene_index > 0 {
                        let (moves, _) = self.scene_moves(start.scene_index);
                        if moves.iter().all(crate::Moves::is_empty) {
                            continue;
                        }
                    }
                    let (moves, placements) = self.start_moves(assembly.id, mechanism, &start);
                    // The first drive from rest is the assembly's own path;
                    // any other start is run on its own.
                    let own = drive == DriveIndex::FIRST && start.held == mechanism.closed();
                    let swept = if own {
                        path.clone()
                    } else {
                        match Path::sweeping(mechanism, drive, &start.held) {
                            Ok(swept) => swept,
                            Err(_) => continue,
                        }
                    };
                    // The moving assemblies it is inside, held: the sweep
                    // runs in their place, everything else seen from there.
                    let base = self.held_outside(assembly.id, &moves);
                    let seen: Vec<Placement> = placements
                        .iter()
                        .map(|placement| placement.seen_from(&base))
                        .collect();
                    // What the assemblies inside it hold where: moved with
                    // it from there.
                    let carried: Vec<Moving<'_>> = moving
                        .iter()
                        .filter_map(|&index| {
                            let body = self.bodies.get(index)?;
                            let held = moves
                                .get(index)
                                .map(|moved| self.held_inside(assembly.id, index, moved))
                                .unwrap_or(Placement::REST);
                            Some(match held {
                                Placement::Exact(by)
                                    if by == stave_model::solved::Point::default() =>
                                {
                                    Moving {
                                        index,
                                        solid: Cow::Borrowed(&body.solid),
                                        held: None,
                                    }
                                }
                                Placement::Exact(by) => Moving {
                                    index,
                                    solid: Cow::Owned(body.solid.translated(by)),
                                    held: None,
                                },
                                Placement::Turned(_) => Moving {
                                    index,
                                    solid: Cow::Borrowed(&body.solid),
                                    held: Some(held),
                                },
                            })
                        })
                        .collect();
                    if let Some(hit) = self.first_hit(assembly.id, &swept, &carried, &seen) {
                        found.push((assembly.id, start, swept, hit));
                        break;
                    }
                }
            }
        }
        for (assembly, start, path, (reach, a, b)) in found {
            self.report_sweep(assembly, &start, &path, reach, a, b);
        }
    }

    /// Where the moving assemblies an assembly is inside hold it, for one
    /// start: their placement, outermost first, at the poses `moves` gives
    /// them (those of a body of the assembly).
    fn held_outside(&self, assembly: AssemblyId, moves: &[crate::Moves]) -> Placement {
        let Some((body, moved)) = self
            .bodies
            .iter()
            .zip(moves)
            .find(|(body, _)| body.chain.contains(&assembly))
        else {
            return Placement::REST;
        };
        let outside = body
            .chain
            .iter()
            .take_while(|id| **id != assembly)
            .copied()
            .collect::<Vec<_>>();
        let chain: Vec<_> = moved
            .steps
            .iter()
            .filter(|(id, _)| outside.contains(id))
            .filter_map(|(id, values)| Some((self.path(*id)?.mechanism(), values.as_slice())))
            .collect();
        Placement::of_poses(&chain, None)
    }

    /// Where the moving assemblies inside an assembly hold one of its
    /// bodies, for one start: their placement, from the assembly's rest,
    /// at the poses `moved` gives them, with the link it is when the
    /// innermost of them turns it on its linkage.
    fn held_inside(&self, assembly: AssemblyId, index: usize, moved: &crate::Moves) -> Placement {
        let Some(body) = self.bodies.get(index) else {
            return Placement::REST;
        };
        let inside: Vec<AssemblyId> = body
            .chain
            .iter()
            .skip_while(|id| **id != assembly)
            .skip(1)
            .copied()
            .collect();
        let inner: Vec<_> = moved
            .steps
            .iter()
            .filter(|(id, _)| inside.contains(id))
            .filter_map(|(id, values)| Some((self.path(*id)?.mechanism(), values.as_slice())))
            .collect();
        if inner.is_empty() {
            return Placement::REST;
        }
        // `moved.link` is set only when the innermost posed assembly is
        // the one whose linkage places the body; that is one inside here.
        Placement::of_poses(&inner, moved.link)
    }

    /// How a body moves when `assembly` moves: a member its own
    /// assembly's mechanism places, or one piece with the assembly.
    fn mover<'p>(&self, index: usize, assembly: AssemblyId, path: &'p Path) -> Mover<'p> {
        match self.bodies.get(index).and_then(|body| body.link) {
            Some((own, link)) if own == assembly => Mover::Member(path, link),
            _ => Mover::Rigid(path),
        }
    }

    /// How a moving body moves when `assembly` moves, from where the
    /// assemblies inside it hold it.
    fn moving_mover<'p>(
        &self,
        body: &Moving<'_>,
        assembly: AssemblyId,
        path: &'p Path,
    ) -> Mover<'p> {
        match (self.mover(body.index, assembly, path), body.held) {
            (Mover::Rigid(_), Some(held)) => Mover::Carried(path, held),
            (mover, _) => mover,
        }
    }

    /// The first contact of an assembly's motion with everything else at
    /// `placements` (seen from where the moving assemblies it is inside
    /// hold it), and between the bodies it moves differently (a link and
    /// its door): how far, and the moving and the other item.
    fn first_hit(
        &self,
        assembly: AssemblyId,
        path: &Path,
        moving: &[Moving<'_>],
        placements: &[Placement],
    ) -> Option<(Reach, Item, Item)> {
        let reach = moving
            .iter()
            .filter_map(|body| path.reach(&body.solid, &self.moving_mover(body, assembly, path)))
            .reduce(|a, b| union(&a, &b))?;
        let mut best: Option<(Reach, Item, Item)> = None;
        let mut consider = |hit: Option<Reach>, a: Item, b: Item| {
            if let Some(hit) = hit
                && best.is_none_or(|(r, x, y)| reach_key(&hit, a, b) < reach_key(&r, x, y))
            {
                best = Some((hit, a, b));
            }
        };
        for (j, fixed) in self.bodies.iter().enumerate() {
            if fixed.chain.contains(&assembly)
                || fixed.loose
                || self.failed.contains(&fixed.item)
                || self.posed_failed.contains(&fixed.item)
            {
                continue;
            }
            let Some(placement) = placements.get(j).copied() else {
                continue;
            };
            if !boxes_overlap(&placed_bounds(&fixed.bounds, &placement), &reach) {
                continue;
            }
            for moved in moving {
                let Some(body) = self.bodies.get(moved.index) else {
                    continue;
                };
                if self.exempt(body.item, fixed.item) {
                    continue;
                }
                let hit = first_contact(
                    &moved.solid,
                    self.moving_mover(moved, assembly, path),
                    &fixed.solid,
                    Mover::Fixed(placement),
                );
                consider(hit, body.item, fixed.item);
            }
        }
        // Bodies the motion moves differently: the member a mechanism
        // turns comes first.
        for (position, first) in moving.iter().enumerate() {
            for second in moving.iter().skip(position + 1) {
                let (mi, mj) = (
                    self.moving_mover(first, assembly, path),
                    self.moving_mover(second, assembly, path),
                );
                let (Some(x), Some(y)) =
                    (self.bodies.get(first.index), self.bodies.get(second.index))
                else {
                    continue;
                };
                if mi == mj || self.exempt(x.item, y.item) {
                    continue;
                }
                let ((a, sa, ma), (b, sb, mb)) = if matches!(mj, Mover::Member(..)) {
                    ((y, &second.solid, mj), (x, &first.solid, mi))
                } else {
                    ((x, &first.solid, mi), (y, &second.solid, mj))
                };
                consider(first_contact(sa, ma, sb, mb), a.item, b.item);
            }
        }
        best
    }

    /// The help for a hinge that hits something as soon as it opens: the
    /// gap that clears it, found by moving the other item away along the
    /// door's free side in steps of 0.5mm, said in the instance's own
    /// terms when it has a mount.
    fn clearance_help(
        &self,
        assembly: AssemblyId,
        scene_index: usize,
        path: &Path,
        a: Item,
        b: Item,
    ) -> (String, Option<OverlayChange>) {
        let name = self.motion_name(assembly, path);
        let b_name = self.item_name(b);
        self.clearance(assembly, scene_index, path, a, b).unwrap_or_else(|| {
            (
                format!(
                    "{name} turns into {b_name} as soon as it opens; leave a gap between them, or move {b_name} out of its way"
                ),
                None,
            )
        })
    }

    /// The gap that clears a hinged assembly's first hit, found by moving
    /// the other item away from it across the door (toward the free side,
    /// or, for something beside the hinge edge, away from it) in steps of
    /// 0.5mm; said in the instance's own terms when it has a mount. `None`
    /// when 20mm doesn't clear it. With the help, the change to the
    /// front's overlay that clears it, when that is the edge between them.
    fn clearance(
        &self,
        assembly: AssemblyId,
        scene_index: usize,
        path: &Path,
        a: Item,
        b: Item,
    ) -> Option<(String, Option<OverlayChange>)> {
        let name = self.assembly_name(assembly);
        let b_name = self.item_name(b);
        let (free, _) = path.mechanism().swing()?;
        let (_, placements) = self.scene_moves(scene_index);
        let body = |item: Item| self.bodies.iter().position(|x| x.item == item);
        let (ia, ib) = (body(a)?, body(b)?);
        // Only something the motion doesn't move can be moved out of its
        // way.
        if self
            .bodies
            .get(ib)
            .is_some_and(|fixed| fixed.chain.contains(&assembly))
        {
            return None;
        }
        let mover = self.mover(ia, assembly, path);
        let (Some(moving), Some(fixed), Some(Placement::Exact(offset))) = (
            self.bodies.get(ia),
            self.bodies.get(ib),
            placements.get(ib).copied(),
        ) else {
            return None;
        };
        let axis = free.axis();
        let placed = placed_bounds(&fixed.bounds, &Placement::Exact(offset));
        // Is the other item on the free side of the moving one, or beside
        // its hinge edge?
        let middle = |b: &WorldBox| b.min().get(axis).nm() / 2 + b.max().get(axis).nm() / 2;
        let beyond =
            (middle(&placed) - middle(&moving.bounds)) * if free.is_positive() { 1 } else { -1 };
        let at_hinge_edge = beyond < 0;
        let free = if at_hinge_edge { free.opposite() } else { free };
        let gap = if free.is_positive() {
            placed.min().get(axis).nm() - moving.bounds.max().get(axis).nm()
        } else {
            moving.bounds.min().get(axis).nm() - placed.max().get(axis).nm()
        };
        let half_mm = 500_000;
        let needed = (1..=40).map(|k| k * half_mm).find(|&total| {
            let shift = total - gap;
            if shift <= 0 {
                return false;
            }
            let shift = if free.is_positive() { shift } else { -shift };
            let moved = offset.with(
                axis,
                Length::from_nm(offset.get(axis).nm().saturating_add(shift)),
            );
            first_contact(
                &moving.solid,
                mover,
                &fixed.solid,
                Mover::Fixed(Placement::Exact(moved)),
            )
            .is_none()
        });
        let total = needed?;
        let total_text = length_text(Length::from_nm(total));
        let more = length_text(Length::from_nm(total - gap));
        let root = self.mirror_root(assembly);
        let mount = self.instance_place(root).is_some_and(|p| p.mount);
        let other = self.outer(b);
        let edge_of = |direction: Direction| match direction {
            Direction::Right => Some("right"),
            Direction::Left => Some("left"),
            Direction::Up => Some("top"),
            Direction::Down => Some("bottom"),
            Direction::Front | Direction::Back => None,
        };
        let field = edge_of(free);
        // The overlay is written at the source of a mirrored front, where
        // the edge that hits may be the other one: a mirror across the
        // cabinet swaps left and right.
        let source_field = edge_of(self.at_mirror_root(assembly, free));
        let smaller = |by: i64| {
            source_field.map(|edge| OverlayChange::Smaller {
                edge,
                by: Length::from_nm(by),
            })
        };
        Some(match other {
            // Two mirrored doors that meet at their free edges.
            Some(other) if mount && self.mirror_root(other) == root && !at_hinge_edge => (
                format!(
                    "leave at least {total_text} between {name} and {}: use meeting: -{} (each door)",
                    self.assembly_name(other),
                    length_text(Length::from_nm(total / 2))
                ),
                Some(OverlayChange::Meeting(Length::from_nm(-(total / 2)))),
            ),
            // Two mirrored doors hinged back to back: each gives half.
            Some(other) if mount && self.mirror_root(other) == root && field.is_some() => (
                format!(
                    "leave at least {total_text} between {name} and {}: make its {} overlay {} smaller (the mirrored door follows)",
                    self.assembly_name(other),
                    field.unwrap_or_default(),
                    length_text(Length::from_nm((total - gap) / 2))
                ),
                smaller((total - gap) / 2),
            ),
            _ if mount => match (field, source_field) {
                // A mirrored front's overlay is its source's, and the
                // edge may be the other one there.
                (Some(_), Some(source_field)) if root != assembly => (
                    format!(
                        "leave at least {total_text} between {name} and {b_name}: make {}'s {source_field} overlay {more} smaller ({name} follows)",
                        self.assembly_name(root)
                    ),
                    smaller(total - gap),
                ),
                (Some(field), _) => (
                    format!(
                        "leave at least {total_text} between {name} and {b_name}: make its {field} overlay {more} smaller"
                    ),
                    smaller(total - gap),
                ),
                (None, _) => (
                    format!("leave at least {total_text} between {name} and {b_name}"),
                    None,
                ),
            },
            // Without a mount the overlay may still be written in the
            // component; the fix finds it there, or there is none.
            _ => (
                format!("leave at least {total_text} between {name} and {b_name}"),
                smaller(total - gap),
            ),
        })
    }

    /// The contact of a sweep's first hit: the motion stopped a little
    /// past it, and the wood shared there.
    fn sweep_contact(
        &self,
        assembly: AssemblyId,
        scene_index: usize,
        path: &Path,
        reach: Reach,
        a: Item,
        b: Item,
    ) -> Contact {
        let (at, shown) = match (reach, path.mechanism().extent()) {
            (Reach::Angle(angle), DriveValue::Turn(limit)) => (
                PoseValue::Angle(angle),
                PoseValue::Angle(Angle::from_microdeg(
                    angle
                        .microdeg()
                        .saturating_add(2_000_000)
                        .min(limit.microdeg()),
                )),
            ),
            (Reach::Distance(hit), DriveValue::Travel(travel)) => (
                PoseValue::Travel(hit.distance),
                PoseValue::Travel(Length::from_nm(
                    hit.distance.nm().saturating_add(2_000_000).min(travel.nm()),
                )),
            ),
            // A reach always matches its motion.
            (Reach::Angle(_), DriveValue::Travel(_))
            | (Reach::Distance(_), DriveValue::Turn(_)) => {
                return Contact::default();
            }
        };
        let (_, placements) = self.scene_moves(scene_index);
        let body = |item: Item| self.bodies.iter().position(|x| x.item == item);
        // Where the motion puts a body it moves, at `shown`.
        let shown_at = |index: usize| {
            let link = self
                .bodies
                .get(index)
                .and_then(|body| body.link)
                .filter(|(own, _)| *own == assembly)
                .map(|(_, link)| link);
            Placement::of_mechanisms(&[(path.mechanism(), shown)], link)
        };
        let moves = |index: usize| {
            self.bodies
                .get(index)
                .is_some_and(|body| body.chain.contains(&assembly))
        };
        let found = match (body(a), body(b)) {
            (Some(ia), Some(ib)) => {
                let pb = if moves(ib) {
                    Some(shown_at(ib))
                } else {
                    placements.get(ib).copied()
                };
                match (self.bodies.get(ia), self.bodies.get(ib), pb) {
                    (Some(x), Some(y), Some(pb)) => {
                        overlaps_placed(&x.solid, &shown_at(ia), &y.solid, &pb)
                    }
                    _ => Vec::new(),
                }
            }
            _ => Vec::new(),
        };
        let mut contact = Contact::of(self.design, &[a, b], &found);
        contact.scene = (scene_index > 0)
            .then(|| self.design.scenes.get(scene_index).map(|s| s.id))
            .flatten();
        contact.motion = Some(ContactMotion {
            assembly,
            drive: path.mechanism().swept(),
            at,
            shown,
        });
        contact
    }

    /// The motion a path sweeps, as a message names it: the assembly, or
    /// the assembly and the drive's name for one with several drives
    /// (`frame.tilt`).
    fn motion_name(&self, assembly: AssemblyId, path: &Path) -> String {
        let name = self.assembly_name(assembly);
        let mechanism = path.mechanism();
        match mechanism
            .drives()
            .get(mechanism.swept().index())
            .and_then(|named| named.name.as_ref())
        {
            Some(drive) if mechanism.drives().len() > 1 => format!("{name}.{drive}"),
            Some(_) | None => name,
        }
    }

    /// Where the other drives are held while a path sweeps one, for a note
    /// (`with frame.height held at 150mm`); `None` when they are all
    /// closed.
    fn held_note(&self, assembly: AssemblyId, path: &Path) -> Option<String> {
        let mechanism = path.mechanism();
        if mechanism.held_closed() {
            return None;
        }
        let name = self.assembly_name(assembly);
        let held: Vec<String> = mechanism
            .drives()
            .iter()
            .zip(mechanism.held())
            .enumerate()
            .filter(|(index, (named, value))| {
                *index != mechanism.swept().index() && named.drive.from() != **value
            })
            .map(|(_, (named, value))| {
                let amount = match (named.drive.from(), *value) {
                    (DriveValue::Turn(from), DriveValue::Turn(at)) => Angle::from_microdeg(
                        at.microdeg()
                            .saturating_sub(from.microdeg())
                            .saturating_abs(),
                    )
                    .to_string(),
                    (DriveValue::Travel(from), DriveValue::Travel(at)) => length_text(
                        Length::from_nm(at.nm().saturating_sub(from.nm()).saturating_abs()),
                    ),
                    (DriveValue::Turn(_), DriveValue::Travel(_))
                    | (DriveValue::Travel(_), DriveValue::Turn(_)) => String::new(),
                };
                match &named.name {
                    Some(drive) => format!("{name}.{drive} held at {amount}"),
                    None => format!("{name} held at {amount}"),
                }
            })
            .collect();
        (!held.is_empty()).then(|| format!("with {}", crate::names::and_list(&held)))
    }

    fn report_sweep(
        &mut self,
        assembly: AssemblyId,
        start: &SweepStart,
        path: &Path,
        reach: Reach,
        a: Item,
        b: Item,
    ) {
        let scene_index = start.scene_index;
        let name = self.motion_name(assembly, path);
        let (a_name, b_name) = (self.item_name(a), self.item_name(b));
        let extent = path.mechanism().extent();
        // A door that doesn't turn about one fixed line (a cup hinge's)
        // swings its edges out as it starts to open: an early hit is
        // cleared by a gap.
        let swings_out = !matches!(path.mechanism().shape(), MotionShape::Turn { .. });
        let mut fixes = Vec::new();
        let (message, mut notes, help) = match (reach, extent) {
            (Reach::Angle(angle), DriveValue::Turn(limit)) => {
                let at = angle_text(angle);
                let (help, overlay) = if angle.microdeg() < 1_000_000 {
                    self.clearance_help(assembly, scene_index, path, a, b)
                } else {
                    let early = swings_out && angle < Angle::from_deg(30);
                    early
                        .then(|| self.clearance(assembly, scene_index, path, a, b))
                        .flatten()
                        .unwrap_or_else(|| (format!(
                            "open {name} no further than {at} (give its hardware's kind a smaller range), or move {b_name} out of its way"
                        ), None))
                };
                // The overlay that clears it first, as the help says,
                // then the narrower opening the hardware's kind allows.
                let root = self.mirror_root(assembly);
                fixes.extend(overlay.and_then(|change| self.overlay_fix(root, change)));
                fixes.extend(self.opening_fix(assembly, angle));
                (
                    format!("{a_name} hits {b_name} when {name} is opened past {at}"),
                    vec![format!("the hinges open to {limit}")],
                    help,
                )
            }
            (Reach::Distance(hit), DriveValue::Travel(travel)) => {
                let at = length_text(hit.distance);
                (
                    format!("{a_name} hits {b_name} when {name} is pulled out past {at}"),
                    vec![format!("the slides travel {}", length_text(travel))],
                    match hit.shrink {
                        Some((amount, axis)) => format!(
                            "{a_name} reaches {} into the way of {b_name}, measured {}; make it that much smaller there, or move {b_name} out of its way",
                            length_text(amount),
                            axis.word()
                        ),
                        None => format!("move {b_name} out of the way of {name}"),
                    },
                )
            }
            // A reach always matches its motion.
            (Reach::Angle(_), DriveValue::Travel(_))
            | (Reach::Distance(_), DriveValue::Turn(_)) => {
                return;
            }
        };
        if let Some(held) = self.held_note(assembly, path) {
            notes.push(held);
        }
        // The moving assemblies it is inside or carries, where they are
        // held away from closed.
        let elsewhere: Vec<String> = start
            .others
            .iter()
            .filter(|(_, values)| values.iter().any(|v| !matches!(v, PoseValue::Closed)))
            .flat_map(|(other, values)| self.pose_conditions(*other, values))
            .collect();
        if !elsewhere.is_empty() {
            notes.push(format!("while {}", crate::names::and_list(&elsewhere)));
        }
        if scene_index > 0
            && let Some(scene) = self.design.scenes.get(scene_index)
        {
            notes.push(format!(
                "with everything else as scene '{}' poses it",
                scene.name
            ));
        }
        // The instance's line in the user's file (its mount, when it has
        // one), where the mistake almost always is.
        let place = self
            .instance_place(self.mirror_root(assembly))
            .map(|p| (p.file, p.span))
            .or_else(|| {
                self.motion_hardware(assembly)
                    .and_then(|hardware| self.hardware_place(hardware))
            });
        self.next_contact = Some(self.sweep_contact(assembly, scene_index, path, reach, a, b));
        self.report(
            Severity::Error,
            Code::Motion,
            message,
            place,
            notes,
            Some(help),
        );
        if let Some((diagnostic, _)) = self.diagnostics.last_mut() {
            diagnostic.fixes = fixes;
        }
    }
}

/// Each assembly's mechanism run through its range, by assembly (`None`
/// for one nothing moves, or whose mechanism can't move), and the
/// mechanisms that can't.
pub(crate) fn mechanism_paths(design: &SolvedDesign) -> crate::Mechanisms {
    let mut paths = Vec::with_capacity(design.assemblies().len());
    let mut broken = Vec::new();
    for assembly in design.assemblies() {
        paths.push(match &assembly.mechanism {
            Some(mechanism) => {
                // Each drive is run through its range with the others at
                // their `from` (grammar, "How a mechanism is solved", dead
                // points); the first drive's is the assembly's path.
                let closed = mechanism.closed();
                let others = (1..mechanism.drives().len())
                    .filter_map(DriveIndex::new)
                    .map(|drive| Path::sweeping(mechanism, drive, &closed).map(|_| ()));
                match Path::new(mechanism).and_then(|path| {
                    others.collect::<Result<Vec<()>, _>>()?;
                    Ok(path)
                }) {
                    Ok(path) => Some(path),
                    Err(error) => {
                        broken.push((assembly.id, error));
                        None
                    }
                }
            }
            None => None,
        });
    }
    crate::Mechanisms { paths, broken }
}
