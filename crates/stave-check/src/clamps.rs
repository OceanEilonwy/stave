//! Clamps (grammar, "Clamps"): hardware whose kind says `clamp a, b
//! along w` draws member `a` and member `b` toward each other when it is
//! tightened, so what lies between `a` and the part `b` moves with (the
//! far part) must be solid. A gap is `error[no-contact]` at the
//! hardware's call.
//!
//! The line runs along the clamp's world axis through the middle of `b`.
//! It is followed from the end of `a` nearer the far part to the far
//! part's face nearer `a`, through every part and every other member of
//! the same piece of hardware it meets ([`stave_geom::line`]): a part's
//! wood is its boxes less its square cuts, with the cuts the same piece of
//! hardware makes filled back in. Each body must touch the next within
//! the contact tolerance.
//!
//! A clamp is checked wherever the hardware is set: at rest, at every
//! scene's poses, and at each set value of each drive (every other drive
//! closed), never while a drive moves. At a pose the walk runs where the
//! drawn member is, every other body seen from there
//! ([`stave_geom::line::Line::seen_by`]): exact for a body the pose moved
//! straight, or turned about a line along the clamp's axis (a pivot bolt
//! clamped along its own pin), and a body the pose turned so that the line
//! crosses it slanted fills nothing, so a gap is never missed.

use stave_geom::collide::{CONTACT_TOLERANCE, Placement, placed_bounds};
use stave_geom::line::{Line, Stretch};
use stave_lang::units::Length;
use stave_lang::{Code, Severity};
use stave_model::mechanism::DriveValue;
use stave_model::solved::{
    BodyCarrier, BodyOwner, CutPlace, CutSource, Hardware, PartId, PoseValue, Size3, WorldBox,
};

use crate::names::length_text;
use crate::{Checker, Contact, ContactItem, Item};

/// A body in a clamp's solid stack at rest, with the two it lies between
/// (the bearing member or the far part at the ends).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct StackBody {
    /// The body.
    pub item: Item,
    /// The bodies either side of it in the stack.
    pub neighbours: [Item; 2],
}

/// One side of a gap in a clamp's stack.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Side {
    /// The member that bears on the stack.
    Bearing,
    /// A part, or another member of the same piece of hardware.
    Body(Item),
    /// The part the drawn member moves with.
    Far,
}

/// A stretch of the line that some body fills, by who fills it.
#[derive(Debug, Clone, Copy)]
struct Filled {
    stretch: Stretch,
    side: Side,
}

/// The first gap from the bearing member: what is on either side and how
/// wide it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Gap {
    near: Side,
    far: Side,
    width: Length,
}

/// Where a clamp is checked besides rest: a scene's poses, or a set value
/// of a drive.
#[derive(Debug, Clone, PartialEq)]
enum Pose {
    /// A scene, by its index (never 0, the rest).
    Scene(usize),
    /// A drive of an assembly at one of its set values, every other drive
    /// closed: the words for it (`frame.height is set at 100mm`).
    Set(String),
}

impl Checker<'_> {
    /// Every piece of hardware whose kind has a `clamp`: its stack must be
    /// solid wherever it is set, at rest, at every scene's poses and at
    /// each set value of each drive; the first place it isn't is reported.
    pub(crate) fn clamps(&mut self) {
        let design = self.design;
        if !design
            .hardware()
            .iter()
            .any(|hardware| hardware.clamp.is_some())
        {
            return;
        }
        let poses = self.clamp_poses();
        for hardware in design.hardware() {
            if self.misfit.contains(&hardware.id) || hardware.clamp.is_none() {
                continue;
            }
            if let Some(gap) = self.clamp_gap(hardware, None) {
                self.report_gap(hardware, gap, None);
                continue;
            }
            let posed = poses.iter().find_map(|(pose, placements)| {
                Some((pose, self.clamp_gap(hardware, Some(placements))?))
            });
            if let Some((pose, gap)) = posed {
                let pose = pose.clone();
                self.report_gap(hardware, gap, Some(&pose));
            }
        }
    }

    /// Every place a clamp is set besides rest, with every body's
    /// placement there: each scene that moves something, then each set
    /// value of each drive of each assembly, every other drive closed and
    /// everything else at rest.
    fn clamp_poses(&self) -> Vec<(Pose, Vec<Placement>)> {
        let mut poses = Vec::new();
        for scene_index in 1..self.design.scenes.len() {
            let (moves, placements) = self.scene_moves(scene_index);
            if moves.iter().any(|moved| !moved.is_empty()) {
                poses.push((Pose::Scene(scene_index), placements));
            }
        }
        for assembly in self.design.assemblies() {
            let (Some(path), Some(mechanism)) = (self.path(assembly.id), &assembly.mechanism)
            else {
                continue;
            };
            let name = self.assembly_name(assembly.id);
            for held in Self::set_holds(mechanism, None) {
                let values: Vec<PoseValue> = mechanism
                    .drives()
                    .iter()
                    .zip(&held)
                    .map(|(named, value)| match named.drive.amount_of(*value) {
                        Some(DriveValue::Turn(angle)) => PoseValue::Angle(angle),
                        Some(DriveValue::Travel(length)) => PoseValue::Travel(length),
                        None => PoseValue::Closed,
                    })
                    .collect();
                let placements = self
                    .bodies
                    .iter()
                    .map(|body| {
                        if !body.chain.contains(&assembly.id) {
                            return Placement::REST;
                        }
                        let link = body
                            .link
                            .filter(|(own, _)| *own == assembly.id)
                            .map(|(_, link)| link);
                        Placement::of_poses(&[(path.mechanism(), values.as_slice())], link)
                    })
                    .collect();
                // The one drive away from closed, as a message names it.
                let words = mechanism
                    .drives()
                    .iter()
                    .zip(held.iter().zip(mechanism.closed()))
                    .find(|(_, (value, from))| **value != *from)
                    .map(|(named, (value, _))| {
                        let drive = match &named.name {
                            Some(drive) if mechanism.drives().len() > 1 => {
                                format!("{name}.{drive}")
                            }
                            Some(_) | None => name.clone(),
                        };
                        let amount = match named.drive.amount_of(*value) {
                            Some(DriveValue::Turn(angle)) => angle.to_string(),
                            Some(DriveValue::Travel(length)) => length_text(length),
                            None => String::new(),
                        };
                        format!("{drive} is set at {amount}")
                    })
                    .unwrap_or_default();
                poses.push((Pose::Set(words), placements));
            }
        }
        poses
    }

    /// The first gap in a piece of hardware's stack, if it has a clamp and
    /// a gap: at rest, or with every body where `placements` puts it.
    fn clamp_gap(&self, hardware: &Hardware, placements: Option<&[Placement]>) -> Option<Gap> {
        self.clamp_walk(hardware, placements)?.err()
    }

    /// The bodies in a piece of hardware's stack at rest, from the one
    /// against the bearing member to the far part, each between its two
    /// neighbours (the bearing member and the far part at the ends): `None`
    /// when it has no clamp or its stack has a gap (grammar, "Running
    /// clearance", a spacer).
    pub(crate) fn clamp_stack(&self, hardware: &Hardware) -> Option<Vec<StackBody>> {
        let sides = self.clamp_walk(hardware, None)?.ok()?;
        let items: Vec<Option<Item>> = std::iter::once(None)
            .chain(sides.iter().map(|side| match side {
                Side::Body(item) => Some(*item),
                Side::Bearing | Side::Far => None,
            }))
            .chain(std::iter::once(None))
            .collect();
        let clamp = hardware.clamp?;
        let far = hardware
            .members
            .iter()
            .find(|m| m.id.body == clamp.drawn)
            .and_then(|m| match m.carrier {
                BodyCarrier::Part(part) => Some(Item::Part(part)),
                BodyCarrier::Region(_) | BodyCarrier::Link(_) => None,
            })?;
        let bearing = hardware
            .members
            .iter()
            .find(|m| m.id.body == clamp.bearing)
            .map(|m| Item::Member(m.id))?;
        let named = |item: Option<Item>, end: Item| item.unwrap_or(end);
        Some(
            items
                .windows(3)
                .filter_map(|window| {
                    let [before, Some(body), after] = window else {
                        return None;
                    };
                    Some(StackBody {
                        item: *body,
                        neighbours: [named(*before, bearing), named(*after, far)],
                    })
                })
                .collect(),
        )
    }

    /// The walk along a piece of hardware's stack: the bodies it passes,
    /// in order, when it is solid, or the first gap. `None` when it has no
    /// clamp, or nothing lies between its members.
    fn clamp_walk(
        &self,
        hardware: &Hardware,
        placements: Option<&[Placement]>,
    ) -> Option<Result<Vec<Side>, Gap>> {
        let clamp = hardware.clamp?;
        let member = |body| hardware.members.iter().find(|m| m.id.body == body);
        let bearing = member(clamp.bearing)?;
        let drawn = member(clamp.drawn)?;
        let BodyCarrier::Part(far) = drawn.carrier else {
            return None;
        };
        // Each body as the drawn member sees it: the walk runs where it is.
        let placed = |item: Item| -> Placement {
            placements
                .and_then(|placements| {
                    let index = self.bodies.iter().position(|body| body.item == item)?;
                    placements.get(index).copied()
                })
                .unwrap_or(Placement::REST)
        };
        let drawn_at = placed(Item::Member(drawn.id));
        let seen = |item: Item| placed(item).seen_from(&drawn_at);
        let far_box = placed_bounds(&self.design.part(far)?.drawn_box(), &seen(Item::Part(far)));
        let axis = clamp.axis;
        let line = Line::through_middle(axis, &drawn.bounds());
        let bearing_box = placed_bounds(&bearing.bounds(), &seen(Item::Member(bearing.id)));
        let (bearing_low, bearing_high) =
            (bearing_box.min().get(axis), bearing_box.max().get(axis));
        let (far_low, far_high) = (far_box.min().get(axis), far_box.max().get(axis));
        // Followed toward the far part; the far side is read reversed so
        // the walk always runs up the axis.
        let forward = far_low >= bearing_high;
        let backward = far_high <= bearing_low;
        let (start, end) = match (forward, backward) {
            (true, _) => (bearing_high, far_low),
            (false, true) => (negate(bearing_low)?, negate(far_high)?),
            // They overlap along the axis: nothing lies between them.
            (false, false) => return Some(Ok(Vec::new())),
        };
        let mut filled = Vec::new();
        for body in &self.bodies {
            let side = match body.item {
                Item::Part(part) if part == far => Side::Far,
                Item::Part(_) => Side::Body(body.item),
                Item::Member(member)
                    if member.owner
                        == (BodyOwner::Hardware {
                            hardware: hardware.id,
                        })
                        && member.body != clamp.bearing
                        && member.body != clamp.drawn =>
                {
                    Side::Body(body.item)
                }
                Item::Member(_) => continue,
            };
            // Where the line runs in the body as it sees it; a body the
            // pose slants across the line fills nothing.
            let Some((own_line, back)) = line.seen_by(&seen(body.item)) else {
                continue;
            };
            let mut stretches: Vec<Stretch> = own_line
                .in_solid(&body.solid)
                .into_iter()
                .filter_map(|stretch| back.stretch(stretch))
                .collect();
            // The room cut for the clamp itself goes where the clamp goes
            // (a bolt set lower in the slot cut for it): taken where it
            // is with the drawn member at rest.
            if let Item::Part(part) = body.item {
                stretches.extend(
                    self.own_cuts(part, hardware)
                        .iter()
                        .filter_map(|region| line.in_box(region)),
                );
            }
            for stretch in stretches {
                let stretch = if forward {
                    stretch
                } else {
                    Stretch {
                        low: negate(stretch.high)?,
                        high: negate(stretch.low)?,
                    }
                };
                filled.push(Filled { stretch, side });
            }
        }
        Some(walk(start, end, &filled))
    }

    /// The world boxes of the cuts a piece of hardware makes in a part:
    /// the room cut for it, which is never a gap in its stack.
    fn own_cuts(&self, part: PartId, hardware: &Hardware) -> Vec<WorldBox> {
        let Some(found) = self.design.part(part) else {
            return Vec::new();
        };
        found
            .cuts
            .iter()
            .filter(|cut| {
                cut.source
                    == CutSource::Hardware {
                        hardware: hardware.id,
                    }
            })
            .filter_map(|cut| match &cut.place {
                CutPlace::Member { footprint, .. } => {
                    let offset = Size3 {
                        length: footprint.min.length,
                        width: footprint.min.width,
                        thickness: footprint.min.thickness,
                    };
                    let size = Size3 {
                        length: footprint.max.length.checked_sub(footprint.min.length)?,
                        width: footprint.max.width.checked_sub(footprint.min.width)?,
                        thickness: footprint
                            .max
                            .thickness
                            .checked_sub(footprint.min.thickness)?,
                    };
                    Some(found.pose.world_box_at(offset, size))
                }
                CutPlace::End { .. }
                | CutPlace::Side { .. }
                | CutPlace::Crossing { .. }
                | CutPlace::Edge { .. } => None,
            })
            .collect()
    }

    /// Reports a gap at the hardware's call, at rest or at a pose; another
    /// piece of hardware of the same call with a gap is a note on the
    /// first.
    fn report_gap(&mut self, hardware: &Hardware, gap: Gap, pose: Option<&Pose>) {
        let place = self.hardware_place(hardware.id);
        if let Some((file, span)) = place
            && let Some((earlier, _)) = self.diagnostics.iter_mut().find(|(d, _)| {
                d.code == Code::NoContact
                    && d.primary()
                        .is_some_and(|label| label.file == file && label.span == span)
            })
        {
            let note = format!("the same for {}", hardware.name);
            if !earlier.notes.contains(&note) {
                earlier.notes.push(note);
            }
            return;
        }
        let Some(clamp) = hardware.clamp else {
            return;
        };
        let member_name = |body| {
            hardware
                .members
                .iter()
                .find(|m| m.id.body == body)
                .map(|m| m.name.clone())
                .unwrap_or_default()
        };
        let far_part = hardware
            .members
            .iter()
            .find(|m| m.id.body == clamp.drawn)
            .and_then(|m| match m.carrier {
                BodyCarrier::Part(part) => Some(part),
                BodyCarrier::Region(_) | BodyCarrier::Link(_) => None,
            });
        let side_name = |checker: &Checker<'_>, side: Side| match side {
            Side::Bearing => format!("{}'s {}", hardware.name, member_name(clamp.bearing)),
            Side::Body(item) => checker.item_name(item),
            Side::Far => far_part
                .map(|part| checker.part_name(part))
                .unwrap_or_default(),
        };
        let near = side_name(self, gap.near);
        let far = side_name(self, gap.far);
        let pulled = far_part
            .map(|part| self.part_name(part))
            .unwrap_or_default();
        let width = length_text(gap.width);
        let mut contact = Contact::default();
        contact.items.push(ContactItem::Hardware(hardware.id));
        for side in [gap.near, gap.far] {
            let item = match side {
                Side::Body(Item::Part(part)) => Some(ContactItem::Part(part)),
                Side::Far => far_part.map(ContactItem::Part),
                Side::Bearing | Side::Body(Item::Member(_)) => None,
            };
            if let Some(item) = item
                && !contact.items.contains(&item)
            {
                contact.items.push(item);
            }
        }
        if let Some(Pose::Scene(scene_index)) = pose {
            contact.scene = self.design.scenes.get(*scene_index).map(|scene| scene.id);
        }
        self.next_contact = Some(contact);
        let mut notes = Vec::new();
        match pose {
            Some(Pose::Scene(scene_index)) => {
                if let Some(scene) = self.design.scenes.get(*scene_index) {
                    notes.push(format!("as scene '{}' poses it", scene.name));
                }
            }
            Some(Pose::Set(words)) => notes.push(format!("when {words}")),
            None => {}
        }
        notes.push(format!(
            "tightening the {} would pull {pulled} {width} toward {near}",
            member_name(clamp.bearing)
        ));
        self.report(
            Severity::Error,
            Code::NoContact,
            format!(
                "{} draws {far} toward {near}, but they are {width} apart with nothing between",
                hardware.name
            ),
            place,
            notes,
            Some(format!(
                "put a washer or spacer in the gap, or place {far} against {near}"
            )),
        );
    }
}

/// A coordinate read the other way along its axis.
fn negate(length: Length) -> Option<Length> {
    length.nm().checked_neg().map(Length::from_nm)
}

/// The first gap wider than the contact tolerance on the way up from
/// `start` (the bearing member's end) to `end` (the far part's face),
/// through the stretches some body fills; `None` when the stack is solid.
#[cfg(test)]
fn first_gap(start: Length, end: Length, filled: &[Filled]) -> Option<Gap> {
    walk(start, end, filled).err()
}

/// The walk up from `start` (the bearing member's end) to `end` (the far
/// part's face), through the stretches some body fills: the bodies it
/// passes, in order, when the stack is solid, or the first gap wider than
/// the contact tolerance.
fn walk(start: Length, end: Length, filled: &[Filled]) -> Result<Vec<Side>, Gap> {
    let tolerance = CONTACT_TOLERANCE.nm();
    let mut reached = start;
    let mut near = Side::Bearing;
    let mut passed = Vec::new();
    loop {
        if end.nm().saturating_sub(reached.nm()) <= tolerance {
            return Ok(passed);
        }
        // The body that carries on furthest from what touches the stack
        // so far.
        let next = filled
            .iter()
            .filter(|f| f.stretch.low.nm().saturating_sub(reached.nm()) <= tolerance)
            .filter(|f| f.stretch.high > reached)
            .max_by_key(|f| f.stretch.high);
        match next {
            Some(found) => {
                reached = found.stretch.high;
                near = found.side;
                if !passed.contains(&found.side) && found.side != Side::Far {
                    passed.push(found.side);
                }
            }
            None => {
                // The nearest body past the gap, or the far part.
                let beyond = filled
                    .iter()
                    .filter(|f| f.stretch.low > reached && f.stretch.low < end)
                    .min_by_key(|f| f.stretch.low);
                let (far, at) = match beyond {
                    Some(found) => (found.side, found.stretch.low),
                    None => (Side::Far, end),
                };
                return Err(Gap {
                    near,
                    far,
                    width: Length::from_nm(at.nm().saturating_sub(reached.nm())),
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use stave_model::solved::PartId;

    fn mm(value: i64) -> Length {
        Length::from_mm(value)
    }

    fn filled(low: i64, high: i64, part: usize) -> Filled {
        Filled {
            stretch: Stretch {
                low: mm(low),
                high: mm(high),
            },
            side: Side::Body(Item::Part(PartId::from_usize(part))),
        }
    }

    #[test]
    fn a_solid_stack_has_no_gap() {
        let stack = [filled(0, 45, 0), filled(45, 48, 1)];
        assert_eq!(first_gap(mm(0), mm(48), &stack), None);
    }

    #[test]
    fn a_gap_names_the_bodies_either_side() {
        let stack = [filled(0, 45, 0)];
        assert_eq!(
            first_gap(mm(0), mm(48), &stack),
            Some(Gap {
                near: Side::Body(Item::Part(PartId::from_usize(0))),
                far: Side::Far,
                width: mm(3),
            })
        );
        let washer = [filled(0, 45, 0), filled(47, 48, 1)];
        assert_eq!(
            first_gap(mm(0), mm(48), &washer),
            Some(Gap {
                near: Side::Body(Item::Part(PartId::from_usize(0))),
                far: Side::Body(Item::Part(PartId::from_usize(1))),
                width: mm(2),
            })
        );
    }

    #[test]
    fn a_gap_within_the_contact_tolerance_is_touching() {
        let stack = [Filled {
            stretch: Stretch {
                low: Length::from_nm(40_000),
                high: mm(48),
            },
            side: Side::Body(Item::Part(PartId::from_usize(0))),
        }];
        assert_eq!(first_gap(mm(0), mm(48), &stack), None);
    }

    #[test]
    fn a_gap_at_the_bearing_member_names_it() {
        let stack = [filled(2, 48, 0)];
        assert_eq!(
            first_gap(mm(0), mm(48), &stack),
            Some(Gap {
                near: Side::Bearing,
                far: Side::Body(Item::Part(PartId::from_usize(0))),
                width: mm(2),
            })
        );
    }
}
