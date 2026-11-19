//! Assembly sequences: each joint pair's `insert` steps run backwards from
//! the finished joint ([`stave_geom::sequence`]). A step that is blocked
//! both ways is `error[order]`, naming the step and the part in its way.
//!
//! The parts of a pair are checked by themselves (the rest of the design
//! is assembled around them). A pair with a part already reported (for an
//! overlap, or for a cut whose shape is a mistake) is left out.

use stave_geom::sequence::{Member, Step, disassemble, pair_members, pair_steps};
use stave_lang::placement::PartAxis;
use stave_lang::{Code, Severity};
use stave_model::solved::{AssemblyStep, JointPair, PartId, StepMotion};

use crate::{Checker, Item};

impl Checker<'_> {
    pub(crate) fn sequences(&mut self) {
        let mut found = Vec::new();
        for joint in self.design.joints() {
            for pair in &joint.pairs {
                if pair.insert.is_empty() {
                    continue;
                }
                let members = pair_members(pair);
                let left_out = members
                    .iter()
                    .any(|a| self.failed.contains(&Item::Part(*a)));
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
                    let alternative = match step.motion {
                        StepMotion::Push { angle: None, .. } | StepMotion::Drive => PartAxis::ALL
                            .into_iter()
                            .filter(|axis| *axis != step.axis)
                            .find(|axis| {
                                self.steps_of(pair, &members, Some((blocked.step, *axis)))
                                    .is_some_and(|steps| disassemble(&parts, &steps).is_ok())
                            }),
                        StepMotion::Push { angle: Some(_), .. } | StepMotion::Rotate { .. } => None,
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
        pair_steps(self.design, pair, members, replace)
    }

    /// A step as the source writes it: `push(rail, along: width)`.
    fn step_text(&self, step: &AssemblyStep, axis: PartAxis) -> String {
        let part = self.part_name(step.part);
        step.source_text(&part, axis)
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
        let help = match (alternative, step.motion) {
            (Some(axis), _) => format!(
                "{part} comes free along its {}: write {}",
                axis.word(),
                self.step_text(step, axis)
            ),
            (None, StepMotion::Rotate { .. }) => format!(
                "check that {part} is cut to turn where it does, or turn it about another axis"
            ),
            (None, StepMotion::Push { angle: Some(_), .. }) => format!(
                "{part} can't slide out of {other_name} on this diagonal either way; check the cuts that make its way out (slots, flats, rounds) and the insert steps of '{joint_name}'"
            ),
            (None, StepMotion::Push { angle: None, .. } | StepMotion::Drive) => format!(
                "{part} can't come out of {other_name} along any one of its axes; check the cuts and insert steps of '{joint_name}'"
            ),
        };
        self.next_contact = Some(crate::Contact::of(
            self.design,
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
