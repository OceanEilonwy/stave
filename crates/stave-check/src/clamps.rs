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
//! the contact tolerance. Everything is exact: a clamp is checked at rest,
//! where every part is axis-aligned.

use stave_geom::collide::CONTACT_TOLERANCE;
use stave_geom::line::{Line, Stretch};
use stave_lang::units::Length;
use stave_lang::{Code, Severity};
use stave_model::solved::{
    BodyCarrier, BodyOwner, CutPlace, CutSource, Hardware, PartId, Size3, WorldBox,
};

use crate::names::length_text;
use crate::{Checker, Contact, ContactItem, Item};

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

impl Checker<'_> {
    /// Every piece of hardware whose kind has a `clamp`: its stack must be
    /// solid, at rest.
    pub(crate) fn clamps(&mut self) {
        let design = self.design;
        for hardware in design.hardware() {
            if self.misfit.contains(&hardware.id) {
                continue;
            }
            let Some(gap) = self.clamp_gap(hardware) else {
                continue;
            };
            self.report_gap(hardware, gap);
        }
    }

    /// The first gap in a piece of hardware's stack, if it has a clamp and
    /// a gap.
    fn clamp_gap(&self, hardware: &Hardware) -> Option<Gap> {
        let clamp = hardware.clamp?;
        let member = |body| hardware.members.iter().find(|m| m.id.body == body);
        let bearing = member(clamp.bearing)?;
        let drawn = member(clamp.drawn)?;
        let BodyCarrier::Part(far) = drawn.carrier else {
            return None;
        };
        let far_box = self.design.part(far)?.drawn_box();
        let axis = clamp.axis;
        let line = Line::through_middle(axis, &drawn.bounds());
        let bearing_box = bearing.bounds();
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
            (false, false) => return None,
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
            let mut stretches = line.in_solid(&body.solid);
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
        first_gap(start, end, &filled)
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

    /// Reports a gap at the hardware's call; another piece of hardware of
    /// the same call with a gap is a note on the first.
    fn report_gap(&mut self, hardware: &Hardware, gap: Gap) {
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
        self.next_contact = Some(contact);
        self.report(
            Severity::Error,
            Code::NoContact,
            format!(
                "{} draws {far} toward {near}, but they are {width} apart with nothing between",
                hardware.name
            ),
            place,
            vec![format!(
                "tightening the {} would pull {pulled} {width} toward {near}",
                member_name(clamp.bearing)
            )],
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
fn first_gap(start: Length, end: Length, filled: &[Filled]) -> Option<Gap> {
    let tolerance = CONTACT_TOLERANCE.nm();
    let mut reached = start;
    let mut near = Side::Bearing;
    loop {
        if end.nm().saturating_sub(reached.nm()) <= tolerance {
            return None;
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
                return Some(Gap {
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
