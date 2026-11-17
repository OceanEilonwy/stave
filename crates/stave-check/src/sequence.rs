//! Assembly sequences: each joint pair's `insert` steps run backwards from
//! the finished joint ([`stave_geom::sequence`]). A step that is blocked
//! both ways is `error[order]`, naming the step and the part in its way.
//!
//! The parts of a pair are checked by themselves (the rest of the design
//! is assembled around them). A pair with a cut the geometry can't make,
//! or with a part already reported for an overlap, is left out.

use stave_geom::sequence::{Member, Step, Undo, disassemble};
use stave_lang::placement::PartAxis;
use stave_lang::{Code, Severity};
use stave_model::solved::{AssemblyStep, JointPair, PartId, StepAmount, StepKind};

use crate::{Checker, Item, ordered};

/// The axis a push's `angle:` turns toward: length → width → thickness →
/// length.
fn next_axis(axis: PartAxis) -> PartAxis {
    match axis {
        PartAxis::Length => PartAxis::Width,
        PartAxis::Width => PartAxis::Thickness,
        PartAxis::Thickness => PartAxis::Length,
    }
}

impl Checker<'_> {
    pub(crate) fn sequences(&mut self) {
        let mut found = Vec::new();
        for joint in &self.design.joints {
            for pair in &joint.pairs {
                if pair.insert.is_empty() {
                    continue;
                }
                let members: Vec<PartId> = {
                    let mut all: Vec<PartId> = pair.roles.iter().map(|r| r.part).collect();
                    for made in &pair.made_parts {
                        if !all.contains(made) {
                            all.push(*made);
                        }
                    }
                    all.dedup();
                    all
                };
                let left_out = members.iter().enumerate().any(|(i, a)| {
                    self.failed.contains(&Item::Part(*a))
                        || members
                            .iter()
                            .skip(i + 1)
                            .any(|b| self.uncut.contains(&ordered(*a, *b)))
                });
                if left_out {
                    continue;
                }
                let Some(steps) = self.steps_of(pair, &members, None) else {
                    continue;
                };
                let parts: Vec<Member<'_>> = members
                    .iter()
                    .filter_map(|id| {
                        Some(Member {
                            part: self.design.part(*id)?,
                            solid: self.solids.get(id.index())?,
                        })
                    })
                    .collect();
                if parts.len() != members.len() {
                    continue;
                }
                if let Err(blocked) = disassemble(&parts, &steps) {
                    let (Some(step), Some(other)) =
                        (pair.insert.get(blocked.step), members.get(blocked.by))
                    else {
                        continue;
                    };
                    // A push along another of the part's axes that does
                    // come apart, for the help line.
                    let alternative = match step.kind {
                        StepKind::Push | StepKind::Drive if step.angle.is_none() => PartAxis::ALL
                            .into_iter()
                            .filter(|axis| *axis != step.axis)
                            .find(|axis| {
                                self.steps_of(pair, &members, Some((blocked.step, *axis)))
                                    .is_some_and(|steps| disassemble(&parts, &steps).is_ok())
                            }),
                        StepKind::Push | StepKind::Drive | StepKind::Rotate => None,
                    };
                    found.push((joint.id, step.clone(), *other, alternative));
                }
            }
        }
        for (joint, step, other, alternative) in found {
            self.report_order(joint, &step, other, alternative);
        }
    }

    /// The steps of a pair for the sequence engine, with one step's axis
    /// replaced when `replace` says.
    fn steps_of(
        &self,
        pair: &JointPair,
        members: &[PartId],
        replace: Option<(usize, PartAxis)>,
    ) -> Option<Vec<Step>> {
        pair.insert
            .iter()
            .enumerate()
            .map(|(index, step)| {
                let member = members.iter().position(|p| *p == step.part)?;
                let orientation = self.design.part(step.part)?.pose.orientation;
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

    /// A step as the source writes it: `push(rail, along: width)`.
    fn step_text(&self, step: &AssemblyStep, axis: PartAxis) -> String {
        let part = self.part_name(step.part);
        let by = match step.by {
            Some(StepAmount::Length(length)) => format!(", by: {length}"),
            Some(StepAmount::Angle(angle)) => format!(", by: {angle}"),
            None => String::new(),
        };
        match step.kind {
            StepKind::Push | StepKind::Drive => {
                let word = if step.kind == StepKind::Push {
                    "push"
                } else {
                    "drive"
                };
                let angle = step
                    .angle
                    .map(|a| format!(", angle: {a}"))
                    .unwrap_or_default();
                format!("{word}({part}, along: {}{by}{angle})", axis.word())
            }
            StepKind::Rotate => format!("rotate({part}{by}, about: {})", axis.word()),
        }
    }

    fn report_order(
        &mut self,
        joint: stave_model::solved::JointId,
        step: &AssemblyStep,
        other: PartId,
        alternative: Option<PartAxis>,
    ) {
        let text = self.step_text(step, step.axis);
        let (joint_name, part, other_name) = (
            self.joint_name(joint),
            self.part_name(step.part),
            self.part_name(other),
        );
        let help = match (alternative, step.kind) {
            (Some(axis), _) => format!(
                "{part} comes free along its {}: write {}",
                axis.word(),
                self.step_text(step, axis)
            ),
            (None, StepKind::Rotate) => format!(
                "check that {part} is cut to turn where it does, or turn it about another axis"
            ),
            (None, StepKind::Push | StepKind::Drive) if step.angle.is_some() => format!(
                "{part} can't slide out of {other_name} on this diagonal either way; check the cuts that make its way out (slots, flats, rounds) and the insert steps of '{joint_name}'"
            ),
            (None, StepKind::Push | StepKind::Drive) => format!(
                "{part} can't come out of {other_name} along any one of its axes; check the cuts and insert steps of '{joint_name}'"
            ),
        };
        self.next_contact = Some(crate::Contact::of(
            &[Item::Part(step.part), Item::Part(other)],
            &[],
        ));
        self.report(
            Severity::Error,
            Code::Order,
            format!("{joint_name}: {text} is blocked by {other_name}"),
            self.joint_place(joint),
            vec![format!(
                "the steps are undone from the last, from the finished joint: undoing {text} moves {part} into {other_name} either way"
            )],
            Some(help),
        );
    }
}
