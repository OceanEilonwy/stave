//! Motion sweeps (layer 5): every assembly with a motion is moved through
//! its whole travel, from rest and from each scene's poses, and the first
//! thing it hits is reported with the angle or distance where it hits.
//!
//! The moving assembly starts closed; everything else stands where the
//! scene puts it. Every body the motion moves (its parts, the hardware on
//! them, and the links its hardware's mechanism turns) is swept against
//! every body that moves differently, the same way for every kind
//! (grammar, "One collision model"). Each assembly is reported once: the
//! first start (rest, then the scenes in order) at which it hits
//! something, with its first contact. An assembly inside another that
//! moves, or with an item already reported at rest or at a scene's poses,
//! is left out.

use stave_geom::collide::{Placement, boxes_overlap, overlaps_placed, placed_bounds, union};
use stave_model::mechanism::MotionShape;
use std::collections::BTreeSet;

use stave_geom::planar::{MechanismError, MechanismErrorKind};
use stave_geom::sweep::{Mover, Path, Reach, first_contact};
use stave_lang::placement::Direction;
use stave_lang::units::{Angle, Length};
use stave_lang::{Code, Severity};
use stave_model::mechanism::DriveValue;
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
        Some((kind.decl.file, kind.mechanism.as_ref()?.span))
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
            // Inside another assembly that moves: its path isn't a sweep
            // in one fixed plane.
            let mut parent = assembly.parent;
            let mut nested = false;
            while let Some(id) = parent {
                let Some(outer) = self.design.assembly(id) else {
                    break;
                };
                nested |= self.path(id).is_some();
                parent = outer.parent;
            }
            if nested {
                continue;
            }
            let moving: Vec<usize> = (0..self.bodies.len())
                .filter(|&i| {
                    self.bodies
                        .get(i)
                        .is_some_and(|b| b.chain.contains(&assembly.id))
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
            for scene_index in 0..self.design.scenes.len() {
                if stopped() {
                    return;
                }
                let (moves, placements) = self.scene_moves(scene_index);
                if scene_index > 0 && moves.iter().all(crate::Moves::is_empty) {
                    continue;
                }
                if let Some(hit) = self.first_hit(assembly.id, path, &moving, &placements) {
                    found.push((assembly.id, scene_index, hit));
                    break;
                }
            }
        }
        for (assembly, scene_index, (reach, a, b)) in found {
            self.report_sweep(assembly, scene_index, reach, a, b);
        }
    }

    /// How a body moves when `assembly` moves: a member its own
    /// assembly's mechanism places, or one piece with the assembly.
    fn mover<'p>(&self, index: usize, assembly: AssemblyId, path: &'p Path) -> Mover<'p> {
        match self.bodies.get(index).and_then(|body| body.link) {
            Some((own, link)) if own == assembly => Mover::Member(path, link),
            _ => Mover::Rigid(path),
        }
    }

    /// The first contact of an assembly's motion with everything else at
    /// `placements`, and between the bodies it moves differently (a link
    /// and its door): how far, and the moving and the other item.
    fn first_hit(
        &self,
        assembly: AssemblyId,
        path: &Path,
        moving: &[usize],
        placements: &[Placement],
    ) -> Option<(Reach, Item, Item)> {
        let reach = moving
            .iter()
            .filter_map(|&i| {
                let body = self.bodies.get(i)?;
                path.reach(&body.solid, &self.mover(i, assembly, path))
            })
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
            for &i in moving {
                let Some(body) = self.bodies.get(i) else {
                    continue;
                };
                if self.exempt(body.item, fixed.item) {
                    continue;
                }
                let hit = first_contact(
                    &body.solid,
                    self.mover(i, assembly, path),
                    &fixed.solid,
                    Mover::Fixed(placement),
                );
                consider(hit, body.item, fixed.item);
            }
        }
        // Bodies the motion moves differently: the member a mechanism
        // turns comes first.
        for (position, &i) in moving.iter().enumerate() {
            for &j in moving.iter().skip(position + 1) {
                let (mi, mj) = (self.mover(i, assembly, path), self.mover(j, assembly, path));
                let (Some(x), Some(y)) = (self.bodies.get(i), self.bodies.get(j)) else {
                    continue;
                };
                if mi == mj || self.exempt(x.item, y.item) {
                    continue;
                }
                let ((a, ma), (b, mb)) = if matches!(mj, Mover::Member(..)) {
                    ((y, mj), (x, mi))
                } else {
                    ((x, mi), (y, mj))
                };
                consider(first_contact(&a.solid, ma, &b.solid, mb), a.item, b.item);
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
        a: Item,
        b: Item,
    ) -> (String, Option<OverlayChange>) {
        let name = self.assembly_name(assembly);
        let b_name = self.item_name(b);
        self.clearance(assembly, scene_index, a, b).unwrap_or_else(|| {
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
        a: Item,
        b: Item,
    ) -> Option<(String, Option<OverlayChange>)> {
        let name = self.assembly_name(assembly);
        let b_name = self.item_name(b);
        let path = self.path(assembly)?;
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
        reach: Reach,
        a: Item,
        b: Item,
    ) -> Contact {
        let Some(path) = self.path(assembly) else {
            return Contact::default();
        };
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
            at,
            shown,
        });
        contact
    }

    fn report_sweep(
        &mut self,
        assembly: AssemblyId,
        scene_index: usize,
        reach: Reach,
        a: Item,
        b: Item,
    ) {
        let name = self.assembly_name(assembly);
        let (a_name, b_name) = (self.item_name(a), self.item_name(b));
        let Some(path) = self.path(assembly) else {
            return;
        };
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
                    self.clearance_help(assembly, scene_index, a, b)
                } else {
                    let early = swings_out && angle < Angle::from_deg(30);
                    early
                        .then(|| self.clearance(assembly, scene_index, a, b))
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
        self.next_contact = Some(self.sweep_contact(assembly, scene_index, reach, a, b));
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
            Some(mechanism) => match Path::new(mechanism) {
                Ok(path) => Some(path),
                Err(error) => {
                    broken.push((assembly.id, error));
                    None
                }
            },
            None => None,
        });
    }
    crate::Mechanisms { paths, broken }
}
