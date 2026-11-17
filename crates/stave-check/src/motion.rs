//! Motion sweeps (layer 5): every assembly with a motion is moved through
//! its whole travel, from rest and from each scene's poses, and the first
//! thing it hits is reported with the angle or distance where it hits.
//!
//! The moving assembly starts closed; everything else stands where the
//! scene puts it. Each assembly is reported once: the first start (rest,
//! then the scenes in order) at which it hits something, with its first
//! contact. An assembly inside another that moves, or with an item already
//! reported at rest or at a scene's poses, is left out.

use stave_geom::collide::{Placement, boxes_overlap, overlaps_placed, placed_bounds, union};
use stave_geom::sweep::{
    LinkageSweep, SlideHit, linkage_contact, linkage_reach, slide_contact, slide_reach,
    turn_contact, turn_reach,
};
use stave_lang::placement::Direction;
use stave_lang::units::{Angle, Length};
use stave_lang::{Code, Severity};
use stave_model::solved::{AssemblyId, HardwareBase, Motion, PoseValue, WorldBox};

use crate::names::length_text;
use crate::{Checker, Contact, ContactMotion, Item};

/// Where a motion first hits something.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Reach {
    /// A slide, this far out.
    Distance(SlideHit),
    /// A hinge, opened this far.
    Angle(Angle),
}

impl Reach {
    /// What decides which hit comes first: the distance or angle, then
    /// parts before hardware (a drawer side rather than the slide on it).
    fn key(&self, a: Item, b: Item) -> (i64, bool, bool, Item, Item) {
        let amount = match self {
            Reach::Distance(hit) => hit.distance.nm(),
            Reach::Angle(angle) => angle.microdeg(),
        };
        let hardware = |item: Item| !matches!(item, Item::Part(_));
        (amount, hardware(a), hardware(b), a, b)
    }
}

/// A motion ready to sweep: a cup hinge's linkage solved once.
enum Sweeper {
    Turn {
        pivot: stave_model::solved::Pivot,
        opening: Angle,
    },
    Linkage(Box<LinkageSweep>),
    Travel {
        toward: Direction,
        travel: Length,
    },
}

impl Sweeper {
    fn of(motion: &Motion) -> Sweeper {
        match *motion {
            Motion::Turn {
                pivot,
                opening_angle,
            } => Sweeper::Turn {
                pivot,
                opening: opening_angle,
            },
            Motion::Linkage { linkage } => Sweeper::Linkage(Box::new(LinkageSweep::new(&linkage))),
            Motion::Travel { toward, travel } => Sweeper::Travel { toward, travel },
        }
    }

    /// The box a solid can reach over the motion.
    fn reach(&self, solid: &stave_geom::collide::WorldSolid) -> Option<WorldBox> {
        match self {
            Sweeper::Turn { pivot, .. } => turn_reach(solid, pivot),
            Sweeper::Linkage(sweep) => linkage_reach(solid, sweep),
            Sweeper::Travel { toward, travel } => slide_reach(solid, *toward, *travel),
        }
    }

    /// The first contact of a moving solid with a fixed one.
    fn contact(
        &self,
        moving: &stave_geom::collide::WorldSolid,
        fixed: &stave_geom::collide::WorldSolid,
        placement: &Placement,
    ) -> Option<Reach> {
        match self {
            Sweeper::Turn { pivot, opening } => {
                turn_contact(moving, fixed, placement, pivot, *opening).map(Reach::Angle)
            }
            Sweeper::Linkage(sweep) => {
                linkage_contact(moving, fixed, placement, sweep).map(Reach::Angle)
            }
            Sweeper::Travel { toward, travel } => {
                slide_contact(moving, fixed, placement, *toward, *travel).map(Reach::Distance)
            }
        }
    }
}

/// An angle rounded down to a tenth of a degree: `87.3deg`.
fn angle_text(angle: Angle) -> String {
    let tenth = 100_000;
    Angle::from_microdeg(angle.microdeg().div_euclid(tenth) * tenth).to_string()
}

impl Checker<'_> {
    /// The hinge or slide of an assembly that gives it its motion.
    fn motion_hardware(&self, assembly: AssemblyId) -> Option<stave_model::solved::HardwareId> {
        let found = self.design.assemblies.get(assembly.index())?;
        found.hardware.iter().copied().find(|id| {
            self.design.hardware.get(id.index()).is_some_and(|h| {
                matches!(h.base, HardwareBase::Hinge | HardwareBase::Slide)
                    && h.assembly == Some(assembly)
            })
        })
    }

    pub(crate) fn sweeps(&mut self) {
        let mut found = Vec::new();
        for assembly in &self.design.assemblies {
            let Some(motion) = assembly.motion else {
                continue;
            };
            // Inside another assembly that moves: its path isn't a sweep
            // about one fixed line.
            let mut parent = assembly.parent;
            let mut nested = false;
            while let Some(id) = parent {
                let Some(outer) = self.design.assemblies.get(id.index()) else {
                    break;
                };
                nested |= outer.motion.is_some();
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
                let (moves, placements) = self.scene_moves(scene_index);
                if scene_index > 0 && moves.iter().all(Vec::is_empty) {
                    continue;
                }
                if let Some(hit) = self.first_hit(assembly.id, &motion, &moving, &placements) {
                    found.push((assembly.id, motion, scene_index, hit));
                    break;
                }
            }
        }
        for (assembly, motion, scene_index, (reach, a, b)) in found {
            self.report_sweep(assembly, &motion, scene_index, reach, a, b);
        }
    }

    /// The first contact of an assembly's motion with everything else at
    /// `placements`: how far, and the moving and the fixed item.
    fn first_hit(
        &self,
        assembly: AssemblyId,
        motion: &Motion,
        moving: &[usize],
        placements: &[Placement],
    ) -> Option<(Reach, Item, Item)> {
        let sweeper = Sweeper::of(motion);
        let reach_of = |index: usize| -> Option<WorldBox> {
            let body = self.bodies.get(index)?;
            sweeper.reach(&body.solid)
        };
        let reach = moving
            .iter()
            .filter_map(|&i| reach_of(i))
            .reduce(|a, b| union(&a, &b))?;
        let mut best: Option<(Reach, Item, Item)> = None;
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
                let hit = sweeper.contact(&body.solid, &fixed.solid, &placement);
                if let Some(hit) = hit
                    && best.is_none_or(|(b, x, y)| hit.key(body.item, fixed.item) < b.key(x, y))
                {
                    best = Some((hit, body.item, fixed.item));
                }
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
        motion: &Motion,
        scene_index: usize,
        a: Item,
        b: Item,
    ) -> String {
        let name = self.assembly_name(assembly);
        let b_name = self.item_name(b);
        self.clearance(assembly, motion, scene_index, a, b).unwrap_or_else(|| {
            format!(
                "{name} turns into {b_name} as soon as it opens; leave a gap between them, or move {b_name} out of its way"
            )
        })
    }

    /// The gap that clears a hinged assembly's first hit, found by moving
    /// the other item away from it across the door (toward the free side,
    /// or, for something beside the hinge edge, away from it) in steps of
    /// 0.5mm; said in the instance's own terms when it has a mount. `None`
    /// when 20mm doesn't clear it.
    fn clearance(
        &self,
        assembly: AssemblyId,
        motion: &Motion,
        scene_index: usize,
        a: Item,
        b: Item,
    ) -> Option<String> {
        let name = self.assembly_name(assembly);
        let b_name = self.item_name(b);
        let (free, _) = motion.swing()?;
        let sweeper = Sweeper::of(motion);
        let (_, placements) = self.scene_moves(scene_index);
        let body = |item: Item| self.bodies.iter().position(|x| x.item == item);
        let (ia, ib) = (body(a)?, body(b)?);
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
            sweeper
                .contact(&moving.solid, &fixed.solid, &Placement::Exact(moved))
                .is_none()
        });
        let total = needed?;
        let total_text = length_text(Length::from_nm(total));
        let more = length_text(Length::from_nm(total - gap));
        let root = self.mirror_root(assembly);
        let mount = self.instance_place(root).is_some_and(|p| p.mount);
        let other = self.outer(b);
        let field = match free {
            Direction::Right => Some("right"),
            Direction::Left => Some("left"),
            Direction::Up => Some("top"),
            Direction::Down => Some("bottom"),
            Direction::Front | Direction::Back => None,
        };
        Some(match other {
            // Two mirrored doors that meet at their free edges.
            Some(other) if mount && self.mirror_root(other) == root && !at_hinge_edge => format!(
                "leave at least {total_text} between {name} and {}: use meeting: -{} (each door)",
                self.assembly_name(other),
                length_text(Length::from_nm(total / 2))
            ),
            // Two mirrored doors hinged back to back: each gives half.
            Some(other) if mount && self.mirror_root(other) == root && field.is_some() => format!(
                "leave at least {total_text} between {name} and {}: make its {} overlay {} smaller (the mirrored door follows)",
                self.assembly_name(other),
                field.unwrap_or_default(),
                length_text(Length::from_nm((total - gap) / 2))
            ),
            _ if mount => {
                let field = match free {
                    Direction::Right => "right",
                    Direction::Left => "left",
                    Direction::Up => "top",
                    Direction::Down => "bottom",
                    Direction::Front | Direction::Back => {
                        return Some(format!(
                            "leave at least {total_text} between {name} and {b_name}"
                        ));
                    }
                };
                format!(
                    "leave at least {total_text} between {name} and {b_name}: make its {field} overlay {more} smaller"
                )
            }
            _ => format!("leave at least {total_text} between {name} and {b_name}"),
        })
    }

    /// The contact of a sweep's first hit: the motion stopped a little
    /// past it, and the wood shared there.
    fn sweep_contact(
        &self,
        assembly: AssemblyId,
        motion: &Motion,
        scene_index: usize,
        reach: Reach,
        a: Item,
        b: Item,
    ) -> Contact {
        let (at, shown) = match (reach, motion.opening_angle()) {
            (Reach::Angle(angle), Some(opening_angle)) => (
                PoseValue::Angle(angle),
                PoseValue::Angle(Angle::from_microdeg(
                    angle
                        .microdeg()
                        .saturating_add(2_000_000)
                        .min(opening_angle.microdeg()),
                )),
            ),
            (Reach::Distance(hit), None) => {
                let Motion::Travel { travel, .. } = *motion else {
                    return Contact::default();
                };
                (
                    PoseValue::Travel(hit.distance),
                    PoseValue::Travel(Length::from_nm(
                        hit.distance.nm().saturating_add(2_000_000).min(travel.nm()),
                    )),
                )
            }
            // A reach always matches its motion.
            (Reach::Angle(_), None) | (Reach::Distance(_), Some(_)) => {
                return Contact::default();
            }
        };
        let (_, placements) = self.scene_moves(scene_index);
        let body = |item: Item| self.bodies.iter().position(|x| x.item == item);
        let found = match (body(a), body(b)) {
            (Some(ia), Some(ib)) => {
                match (self.bodies.get(ia), self.bodies.get(ib), placements.get(ib)) {
                    (Some(x), Some(y), Some(pb)) => overlaps_placed(
                        &x.solid,
                        &Placement::of_motions(&[(*motion, shown)]),
                        &y.solid,
                        pb,
                    ),
                    _ => Vec::new(),
                }
            }
            _ => Vec::new(),
        };
        let mut contact = Contact::of(&[a, b], &found);
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
        motion: &Motion,
        scene_index: usize,
        reach: Reach,
        a: Item,
        b: Item,
    ) {
        let name = self.assembly_name(assembly);
        let (a_name, b_name) = (self.item_name(a), self.item_name(b));
        let (message, mut notes, help) = match (reach, *motion) {
            (Reach::Angle(angle), Motion::Turn { .. } | Motion::Linkage { .. }) => {
                let opening_angle = motion.opening_angle().unwrap_or_default();
                let at = angle_text(angle);
                (
                    format!("{a_name} hits {b_name} when {name} is opened past {at}"),
                    vec![format!("the hinges open to {opening_angle}")],
                    if angle.microdeg() < 1_000_000 {
                        self.clearance_help(assembly, motion, scene_index, a, b)
                    } else {
                        // A cup hinge swings the door's edges out as it
                        // starts to open: an early hit is cleared by a gap.
                        let early =
                            matches!(motion, Motion::Linkage { .. }) && angle < Angle::from_deg(30);
                        early
                            .then(|| self.clearance(assembly, motion, scene_index, a, b))
                            .flatten()
                            .unwrap_or_else(|| format!(
                                "open {name} no further than {at} (a smaller opening_angle), or move {b_name} out of its way"
                            ))
                    },
                )
            }
            (Reach::Distance(hit), Motion::Travel { travel, .. }) => {
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
            (Reach::Angle(_), Motion::Travel { .. })
            | (Reach::Distance(_), Motion::Turn { .. } | Motion::Linkage { .. }) => {
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
        self.next_contact = Some(self.sweep_contact(assembly, motion, scene_index, reach, a, b));
        self.report(
            Severity::Error,
            Code::Motion,
            message,
            place,
            notes,
            Some(help),
        );
    }
}
