//! The cuts a joint inside a mirrored assembly makes in parts outside it.
//!
//! A mirrored assembly is its source's results reflected (grammar,
//! "Mirroring"). The cuts its joints make in its own parts come with the
//! mirrored parts; the holes its members make in outside parts are made
//! again from the members' world shapes. This module makes the rest: the
//! cuts a joint kind makes in an outside part that its source joint was
//! bound to (the mortise a rail's tenon goes into, in a leg the assembly
//! was handed). Each is made again in the part that occupies the reflected
//! place, reflected into that part's own frame.

use std::collections::HashMap;

use stave_lang::Code;
use stave_lang::placement::{Axis, PartAxis, PerPartAxis};
use stave_lang::units::{Angle, ExactLength, Length};
use stave_model::ids::{AssemblyId, JointId, PartId};
use stave_model::solved::{
    Cut, CutKind, CutPlace, CutSource, End, LocalBox, LocalFace, LocalPoint, Orientation,
};

use crate::joints::{GeomTable, round_length};
use crate::{Fault, Solved, Solver};

/// One mirrored joint: the joint, its source, and the plane its assembly
/// is mirrored about.
pub(crate) struct MirroredJoint<'a> {
    /// The joint inside the mirrored assembly.
    pub joint: JointId,
    /// The joint it is the mirror image of.
    pub source: JointId,
    /// The mirrored assembly.
    pub assembly: AssemblyId,
    /// The world axis the mirror plane is square to.
    pub axis: Axis,
    /// Where the plane is along that axis.
    pub plane: ExactLength,
    /// Each part of the source assembly, with its mirror image.
    pub part_map: &'a HashMap<PartId, PartId>,
}

/// For each local axis (length, width, thickness): whether a coordinate
/// along it runs the other way in the part in the reflected place. `None`
/// when that part is turned so that a local axis lies along another world
/// axis than the reflected source's.
fn flips(source: &Orientation, target: &Orientation, mirror: Axis) -> Option<PerPartAxis<bool>> {
    let mut out = PerPartAxis::<bool>::default();
    for axis in PartAxis::ALL {
        let from = source.direction(axis);
        let reflected = if from.axis() == mirror {
            from.opposite()
        } else {
            from
        };
        let to = target.direction(axis);
        if to.axis() != reflected.axis() {
            return None;
        }
        out[axis] = to != reflected;
    }
    Some(out)
}

/// A cut of one part, re-expressed in the frame of the part in its
/// reflected place.
struct Reframe {
    /// Per local axis: whether coordinates along it run the other way.
    flipped: PerPartAxis<bool>,
    /// The target part's cut size per local axis.
    size: PerPartAxis<Length>,
}

impl Reframe {
    fn flipped(&self, axis: PartAxis) -> bool {
        self.flipped[axis]
    }

    fn end(&self, axis: PartAxis, end: End) -> End {
        match (self.flipped(axis), end) {
            (false, end) => end,
            (true, End::Start) => End::End,
            (true, End::End) => End::Start,
        }
    }

    fn face(&self, face: LocalFace) -> LocalFace {
        LocalFace {
            axis: face.axis,
            end: self.end(face.axis, face.end),
        }
    }

    /// `(low, high)` along `axis`, measured from the other end when the
    /// axis is flipped.
    fn interval(&self, axis: PartAxis, low: Length, high: Length) -> Option<(Length, Length)> {
        if !self.flipped(axis) {
            return Some((low, high));
        }
        let size = self.size[axis];
        Some((size.checked_sub(high)?, size.checked_sub(low)?))
    }

    fn local_box(&self, bounds: &LocalBox) -> Option<LocalBox> {
        let (length_min, length_max) =
            self.interval(PartAxis::Length, bounds.min.length, bounds.max.length)?;
        let (width_min, width_max) =
            self.interval(PartAxis::Width, bounds.min.width, bounds.max.width)?;
        let (thickness_min, thickness_max) = self.interval(
            PartAxis::Thickness,
            bounds.min.thickness,
            bounds.max.thickness,
        )?;
        Some(LocalBox {
            min: LocalPoint {
                length: length_min,
                width: width_min,
                thickness: thickness_min,
            },
            max: LocalPoint {
                length: length_max,
                width: width_max,
                thickness: thickness_max,
            },
        })
    }

    /// How many of the two axes in the plane of `face` are flipped, odd:
    /// the cut's outline is seen in a looking glass.
    fn mirrored_in(&self, face: LocalFace) -> bool {
        PartAxis::ALL
            .into_iter()
            .filter(|axis| *axis != face.axis && self.flipped(*axis))
            .count()
            % 2
            == 1
    }

    /// The cut's operation, unchanged unless it is drawn at an angle or as
    /// an outline in a face that is seen in a looking glass, which is not
    /// reflected yet: `None`.
    fn kind(&self, face: LocalFace, kind: &CutKind) -> Option<CutKind> {
        let reflected_in_face = self.mirrored_in(face);
        let reflected_around_length = PartAxis::ALL
            .into_iter()
            .filter(|axis| *axis != PartAxis::Length && self.flipped(*axis))
            .count()
            % 2
            == 1;
        match kind {
            CutKind::Notch { .. }
            | CutKind::Pocket { .. }
            | CutKind::Tenon { .. }
            | CutKind::Spigot { .. }
            | CutKind::Drill { .. }
            | CutKind::DovetailSocket { .. }
            | CutKind::DovetailTail { .. } => Some(kind.clone()),
            CutKind::Slot { angle, .. } => {
                (*angle == Angle::ZERO || !reflected_in_face).then(|| kind.clone())
            }
            CutKind::Round { flats, .. } => {
                (flats.is_none() || !reflected_around_length).then(|| kind.clone())
            }
            CutKind::Outline { .. } => (!reflected_in_face).then(|| kind.clone()),
        }
    }
}

impl Solver<'_> {
    /// Makes again, in the part in the reflected place, each cut the source
    /// joint's kind made in a part outside the source assembly, and records
    /// the mirrored joint on that part. Member cuts are left out: they are
    /// made from the members' world shapes.
    pub(crate) fn mirror_outside_cuts(
        &mut self,
        mirrored: &MirroredJoint<'_>,
        joint_name: &str,
        geoms: &GeomTable,
        cuts: &mut [Vec<Cut>],
        memberships: &mut [Vec<JointId>],
    ) {
        let mut outside: Vec<(PartId, Cut)> = Vec::new();
        for (index, list) in cuts.iter().enumerate() {
            let part = PartId::from_usize(index);
            if mirrored.part_map.contains_key(&part) {
                continue;
            }
            for cut in list {
                let from_source = matches!(
                    cut.source,
                    CutSource::Joint { joint, .. } if joint == mirrored.source
                );
                if from_source && !matches!(cut.place, CutPlace::Member { .. }) {
                    outside.push((part, cut.clone()));
                }
            }
        }
        for (part, cut) in outside {
            let target = self.rebind_outside_part(part, mirrored);
            let (file, span) = self.joint_place(mirrored.joint);
            let Some(target) = self.settled(target, file, span) else {
                continue;
            };
            let Ok(made) = self.reflect_cut(part, target, &cut, mirrored, geoms) else {
                let (file, span) = self.assembly_place(mirrored.assembly);
                let part_name = self.part_name(part);
                let target_name = self.part_name(target);
                self.error(
                    Code::Mirror,
                    format!(
                        "{joint_name} can't make the mirror image of its cut in {part_name} in {target_name}"
                    ),
                    file,
                    span,
                    format!(
                        "{target_name} is turned differently from {part_name}, or the cut is drawn at an angle; write the assembly out instead of mirroring it"
                    ),
                );
                continue;
            };
            if let Some(list) = cuts.get_mut(target.index()) {
                list.push(made);
            }
            if let Some(list) = memberships.get_mut(target.index())
                && !list.contains(&mirrored.joint)
            {
                list.push(mirrored.joint);
            }
        }
    }

    /// The part in the reflected place of a part outside the assembly, or
    /// the mirror image of a part inside it.
    fn rebind_outside_part(
        &mut self,
        part: PartId,
        mirrored: &MirroredJoint<'_>,
    ) -> Solved<PartId> {
        if let Some(image) = mirrored.part_map.get(&part) {
            return Ok(*image);
        }
        self.rebind_part(part, mirrored.axis, mirrored.plane, mirrored.assembly)
    }

    fn reflect_cut(
        &mut self,
        part: PartId,
        target: PartId,
        cut: &Cut,
        mirrored: &MirroredJoint<'_>,
        geoms: &GeomTable,
    ) -> Solved<Cut> {
        let (Some(source_geom), Some(target_geom)) = (geoms.get(part), geoms.get(target)) else {
            return Err(Fault::Earlier.into());
        };
        let flipped = flips(
            &source_geom.orientation,
            &target_geom.orientation,
            mirrored.axis,
        )
        .ok_or(Fault::Internal("how a cut is mirrored"))?;
        let size = PerPartAxis::try_from_fn(|axis| round_length(target_geom.cut_size(axis)?))?;
        let reframe = Reframe { flipped, size };
        let place = match &cut.place {
            CutPlace::End { toward } => CutPlace::End {
                toward: self.rebind_outside_part(*toward, mirrored)?,
            },
            CutPlace::Side {
                from,
                along_from,
                across_axis,
                across_from,
            } => CutPlace::Side {
                from: self.rebind_outside_part(*from, mirrored)?,
                along_from: reframe.end(PartAxis::Length, *along_from),
                across_axis: *across_axis,
                across_from: reframe.end(*across_axis, *across_from),
            },
            CutPlace::Crossing { other, footprint } => CutPlace::Crossing {
                other: self.rebind_outside_part(*other, mirrored)?,
                footprint: reframe.local_box(footprint).ok_or(Fault::Overflow)?,
            },
            CutPlace::Member { .. } => return Err(Fault::Internal("a mirrored joint's cut").into()),
        };
        let kind = reframe
            .kind(cut.face, &cut.kind)
            .ok_or(Fault::Internal("how a cut is mirrored"))?;
        let CutSource::Joint { pair, .. } = cut.source else {
            return Err(Fault::Internal("the joint a mirrored cut comes from").into());
        };
        Ok(Cut {
            source: CutSource::Joint {
                joint: mirrored.joint,
                pair,
            },
            name: cut.name.clone(),
            face: reframe.face(cut.face),
            place,
            kind,
        })
    }
}

#[cfg(test)]
mod tests {
    use stave_lang::placement::Direction;

    use super::*;

    fn frame(length: Direction, width: Direction) -> Orientation {
        Orientation::from_length_and_width(length, width).unwrap()
    }

    #[test]
    fn a_counterpart_turned_the_same_way_flips_the_axis_along_the_mirror() {
        // A leg standing up, mirrored front to back onto a leg turned the
        // same way: only its thickness (which runs back) runs the other way.
        let leg = frame(Direction::Up, Direction::Right);
        assert_eq!(
            flips(&leg, &leg, Axis::Deep),
            Some(PerPartAxis::new(false, false, true))
        );
        assert_eq!(
            flips(&leg, &leg, Axis::Across),
            Some(PerPartAxis::new(false, true, false))
        );
    }

    #[test]
    fn a_reflected_counterpart_flips_nothing() {
        let left = frame(Direction::Up, Direction::Right);
        let right = frame(Direction::Up, Direction::Left);
        assert_eq!(
            flips(&left, &right, Axis::Across),
            Some(PerPartAxis::new(false, false, false))
        );
    }

    #[test]
    fn a_counterpart_turned_another_way_cannot_take_the_cut() {
        let standing = frame(Direction::Up, Direction::Right);
        let lying = frame(Direction::Right, Direction::Up);
        assert_eq!(flips(&standing, &lying, Axis::Deep), None);
    }
}
