//! Cup hinges: the overlay the catalogue allows (layer 4), and the hinge's
//! own arm and links against the door and the side as the door opens
//! (layer 5).

use std::collections::BTreeMap;

use stave_geom::sweep::{HingePart, LinkageSweep, MechanismMember, mechanism_contact};
use stave_lang::units::{Angle, Length};
use stave_lang::{Code, Severity};
use stave_model::solved::{HardwareData, HardwareId, Motion, PoseValue};

use crate::names::length_text;
use crate::{Checker, Contact, ContactMotion, Item};

/// The farthest a cup is bored from the door's edge (7mm in the catalogues;
/// a little more still leaves wood outside a 35mm cup in a 60mm stile).
const MAX_INSET_NM: i64 = 8_000_000;

/// The tallest mounting plate in the catalogues (9mm).
const MAX_PLATE_NM: i64 = 9_000_000;

impl Checker<'_> {
    /// `error[hinge-overlay]`: a cup hinge whose door overlays the side by
    /// more or less than the hinge's catalogue allows at its boring
    /// distance and plate height, with the side screw's travel. One error
    /// per hinge call; the other hinges it places are a note.
    pub(crate) fn hinge_overlays(&mut self) {
        let mut found: BTreeMap<(stave_lang::FileId, u32, u32), Vec<HardwareId>> = BTreeMap::new();
        for hardware in &self.design.hardware {
            let HardwareData::Hinge {
                cup_inset,
                overlay,
                overlay_range: Some(range),
                ..
            } = hardware.data
            else {
                continue;
            };
            let nominal = range.nominal(cup_inset);
            let off = overlay.nm().saturating_sub(nominal.nm());
            if off.saturating_abs() <= range.adjustment.nm() {
                continue;
            }
            if let Some((file, span)) = self.hardware_place(hardware.id) {
                found
                    .entry((file, span.start(), span.end()))
                    .or_default()
                    .push(hardware.id);
            }
        }
        for list in found.into_values() {
            let Some(first) = list
                .first()
                .and_then(|id| self.design.hardware.get(id.index()))
            else {
                continue;
            };
            let HardwareData::Hinge {
                cup_inset,
                overlay,
                overlay_range: Some(range),
                on,
                ..
            } = first.data
            else {
                continue;
            };
            let nominal = range.nominal(cup_inset);
            let low = Length::from_nm(nominal.nm().saturating_sub(range.adjustment.nm()));
            let high = Length::from_nm(nominal.nm().saturating_add(range.adjustment.nm()));
            let name = first.name.clone();
            let piece = self.part_name(first.piece);
            let on_name = self.part_name(on);
            // A cup further from the edge gives more overlay; a taller plate
            // gives less.
            let needed_inset = Length::from_nm(
                overlay
                    .nm()
                    .saturating_sub(range.base.nm())
                    .saturating_add(range.plate_height.nm()),
            );
            let needed_plate = Length::from_nm(
                range
                    .base
                    .nm()
                    .saturating_add(cup_inset.nm())
                    .saturating_sub(overlay.nm()),
            );
            let range_text = format!("{} to {}", length_text(low), length_text(high));
            let help = if (0..=MAX_INSET_NM).contains(&needed_inset.nm()) {
                format!(
                    "bore the cup with cup_inset: {} (the overlay is base_overlay + cup_inset - plate_height, give or take {} on the side screw), or make the door overlay {on_name} by {range_text}",
                    length_text(needed_inset),
                    length_text(range.adjustment),
                )
            } else if (0..=MAX_PLATE_NM).contains(&needed_plate.nm()) {
                format!(
                    "use a mounting plate with plate_height: {} (a taller plate holds the door less far over {on_name}), or make the door overlay it by {range_text}",
                    length_text(needed_plate),
                )
            } else {
                format!(
                    "make the door overlay {on_name} by {range_text}, or use a hinge made for half-overlay or inset doors (Stave models full-overlay cup hinges only)"
                )
            };
            let mut notes = vec![format!(
                "base_overlay {} + cup_inset {} - plate_height {} = {}",
                length_text(range.base),
                length_text(cup_inset),
                length_text(range.plate_height),
                length_text(nominal)
            )];
            let others: Vec<String> = list
                .iter()
                .skip(1)
                .filter_map(|id| self.design.hardware.get(id.index()))
                .map(|h| h.name.clone())
                .collect();
            if !others.is_empty() {
                notes.push(format!("the same for {}", others.join(", ")));
            }
            for id in &list {
                self.failed.insert(Item::Hardware(*id));
                self.failed.insert(Item::Fixed(*id));
            }
            self.next_contact = Some(Contact::of(
                &[
                    Item::Hardware(first.id),
                    Item::Fixed(first.id),
                    Item::Part(first.piece),
                    Item::Part(on),
                ],
                &[],
            ));
            self.report(
                Severity::Error,
                Code::HingeOverlay,
                format!(
                    "{name}: {piece} overlays {on_name} by {}, but the hinge gives {} to {}",
                    length_text(overlay),
                    length_text(low),
                    length_text(high)
                ),
                self.hardware_place(first.id),
                notes,
                Some(help),
            );
        }
    }

    /// `error[motion]`: a cup hinge's own arm or links running into the
    /// wood of its door (outside the cup) or of the side as the door opens.
    /// One error per assembly (its first hinge that hits; the others are a
    /// note).
    pub(crate) fn mechanisms(&mut self) {
        let mut found = Vec::new();
        for assembly in &self.design.assemblies {
            let Some(Motion::Linkage { .. }) = assembly.motion else {
                continue;
            };
            let mut hits = Vec::new();
            for id in &assembly.hardware {
                let Some(hardware) = self.design.hardware.get(id.index()) else {
                    continue;
                };
                let (Some(mechanism), HardwareData::Hinge { on, .. }) =
                    (&hardware.mechanism, hardware.data)
                else {
                    continue;
                };
                if hardware.assembly != Some(assembly.id) {
                    continue;
                }
                let blocked = [
                    Item::Hardware(*id),
                    Item::Fixed(*id),
                    Item::Part(hardware.piece),
                    Item::Part(on),
                ]
                .iter()
                .any(|item| self.failed.contains(item) || self.posed_failed.contains(item));
                if blocked {
                    continue;
                }
                let solid_of = |part| {
                    self.bodies
                        .iter()
                        .find(|b| b.item == Item::Part(part))
                        .map(|b| &b.solid)
                };
                let (Some(piece), Some(on_solid)) = (solid_of(hardware.piece), solid_of(on)) else {
                    continue;
                };
                let sweep = LinkageSweep::new(&mechanism.linkage);
                if let Some(hit) = mechanism_contact(hardware, piece, on_solid, &sweep) {
                    let part = match hit.part {
                        HingePart::Piece => hardware.piece,
                        HingePart::On => on,
                    };
                    hits.push((
                        *id,
                        part,
                        hit.member,
                        hit.angle,
                        mechanism.linkage.opening_angle(),
                    ));
                }
            }
            if let Some(first) = hits.first().copied() {
                let others: Vec<HardwareId> = hits.iter().skip(1).map(|h| h.0).collect();
                found.push((assembly.id, first, others));
            }
        }
        for (assembly, (hardware, part, member, angle, opening), others) in found {
            let name = self.assembly_name(assembly);
            let hardware_name = self.item_name(Item::Hardware(hardware));
            let member_name = match member {
                MechanismMember::Arm => "the arm",
                MechanismMember::InnerLink => "the inner link",
                MechanismMember::OuterLink => "the outer link",
            };
            let part_name = self.part_name(part);
            let tenth = 100_000;
            let at = Angle::from_microdeg(angle.microdeg().div_euclid(tenth) * tenth);
            let mut notes = vec![format!("the hinges open to {opening}")];
            if !others.is_empty() {
                let names: Vec<String> = others
                    .iter()
                    .map(|h| self.item_name(Item::Hardware(*h)))
                    .collect();
                notes.push(format!("the same for {}", names.join(", ")));
            }
            let mut contact = Contact::of(&[Item::Hardware(hardware), Item::Part(part)], &[]);
            contact.motion = Some(ContactMotion {
                assembly,
                at: PoseValue::Angle(angle),
                shown: PoseValue::Angle(Angle::from_microdeg(
                    angle
                        .microdeg()
                        .saturating_add(2_000_000)
                        .min(opening.microdeg()),
                )),
            });
            self.next_contact = Some(contact);
            self.report(
                Severity::Error,
                Code::Motion,
                format!(
                    "{member_name} of {hardware_name} hits {part_name} when {name} is opened past {at}"
                ),
                self.hardware_place(hardware),
                notes,
                Some(format!(
                    "the hinge's arm and links need room behind the door and inside the cup: check the door's thickness and the hinge's cup_inset against its catalogue, or open {name} no further than {at}"
                )),
            );
        }
    }
}
