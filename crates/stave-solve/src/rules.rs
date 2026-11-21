//! A hardware kind's rules where a piece of hardware is placed: its
//! `adjust` entries and its `require`s (grammar, "Adjustment"). Nothing
//! here knows a base, a member or a param by name: a cup hinge's arm on
//! its plate and a slide's gap beside the drawer are the kinds' own text.

use std::borrow::Cow;

use stave_lang::ast::Span;
use stave_lang::units::ExactLength;
use stave_lang::{Code, Diagnostic, FileId, Label};
use stave_model::defs::{AdjustDef, BodyId, DefId, FrameAxis, KindDef};
use stave_model::ids::HardwareId;
use stave_model::solved::{CutSource, Rules};
use stave_model::value::{AnchorValue, ExactBox, ParamSettings, Value};

use crate::eval::{Env, KindEnv};
use crate::joints::Binding;
use crate::{Failure, Fault, Solved, Solver};

/// One member's offset from its nominal place on one axis its own mount
/// sets (grammar, "Hardware placement").
pub(crate) type Offset = (BodyId, FrameAxis, ExactLength);

/// The setting of one `adjust`: how far its member sits from its nominal
/// place on the other, signed along the axis.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct AdjustSetting {
    /// The member that is set.
    pub body: BodyId,
    /// The axis of the kind's frame.
    pub axis: FrameAxis,
    /// The setting.
    pub offset: ExactLength,
}

/// What the adjustments of a piece of hardware come to at rest.
#[derive(Debug, Clone, PartialEq)]
struct Adjusted {
    /// One per `adjust`, in the kind's order.
    settings: Vec<AdjustSetting>,
    /// Offsets on an axis no `adjust` covers that are not `0mm`.
    unadjusted: Vec<Offset>,
}

/// The offset of a member on an axis: `0mm` when its own mount doesn't
/// set that axis.
fn offset_of(offsets: &[Offset], body: BodyId, axis: FrameAxis) -> ExactLength {
    offsets
        .iter()
        .find(|(b, a, _)| *b == body && *a == axis)
        .map_or(ExactLength::ZERO, |(_, _, offset)| *offset)
}

/// Every adjust's setting, and the offsets no adjust covers.
fn adjusted(adjusts: &[AdjustDef], offsets: &[Offset]) -> Solved<Adjusted> {
    let mut settings = Vec::with_capacity(adjusts.len());
    for adjust in adjusts {
        let offset = offset_of(offsets, adjust.body, adjust.axis)
            .checked_sub(offset_of(offsets, adjust.on, adjust.axis))
            .ok_or(Fault::Overflow)?;
        settings.push(AdjustSetting {
            body: adjust.body,
            axis: adjust.axis,
            offset,
        });
    }
    let covered = |body: BodyId, axis: FrameAxis| {
        adjusts
            .iter()
            .any(|adjust| adjust.axis == axis && (adjust.body == body || adjust.on == body))
    };
    let unadjusted = offsets
        .iter()
        .filter(|(body, axis, offset)| *offset != ExactLength::ZERO && !covered(*body, *axis))
        .copied()
        .collect();
    Ok(Adjusted {
        settings,
        unadjusted,
    })
}

/// The absolute value of a length.
fn magnitude(value: ExactLength) -> Solved<ExactLength> {
    if value < ExactLength::ZERO {
        value.checked_neg().ok_or(Failure::Fault(Fault::Overflow))
    } else {
        Ok(value)
    }
}

/// How an axis is written after "off its place".
fn axis_phrase(axis: FrameAxis) -> &'static str {
    match axis {
        FrameAxis::Along => "along",
        FrameAxis::Across => "across",
        FrameAxis::Depth => "in depth",
    }
}

/// Where a piece of hardware sits at rest, as its kind's `require`s read
/// it: its mount face (`mount`) and the box around each member with a body
/// (`contact(member, b)`).
#[derive(Debug, Clone, Copy)]
pub(crate) struct AtRest<'a> {
    /// The face of its piece it is mounted on, as an anchor.
    pub mount: AnchorValue,
    /// Each member with a body and the world box around its shapes.
    pub members: &'a [(BodyId, ExactBox)],
}

/// A piece of hardware's call, where its rules are reported.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Call<'a> {
    /// The piece of hardware.
    pub hardware: HardwareId,
    /// Its name as written.
    pub name: &'a str,
    /// Its kind call.
    pub file: FileId,
    /// Its kind call.
    pub span: Span,
}

impl Solver<'_> {
    /// Checks a piece of hardware's adjustments and its kind's `require`s
    /// where it is placed, reports what fails (`error[adjust]`,
    /// `error[require]`, with the help search), and returns the settings
    /// at rest and whether every rule holds.
    pub(crate) fn hardware_rules(
        &mut self,
        call: Call<'_>,
        binding: &Binding,
        kind: &KindDef,
        adjusts: &[AdjustDef],
        offsets: &[Offset],
        at_rest: AtRest<'_>,
    ) -> Solved<(Vec<AdjustSetting>, Rules)> {
        let adjusted = adjusted(adjusts, offsets)?;
        let mut rules = Rules::Hold;
        if !self.adjustments_hold(binding, kind, adjusts, &adjusted)? {
            rules = Rules::Broken;
            self.report_adjustment(call, binding, kind, adjusts, &adjusted)?;
        }
        if !self.requires_hold(call, binding, kind, at_rest)? {
            rules = Rules::Broken;
        }
        Ok((adjusted.settings, rules))
    }

    /// `true` when every setting is within its adjustment and no member
    /// is off its place on an axis no adjust covers.
    fn adjustments_hold(
        &mut self,
        binding: &Binding,
        kind: &KindDef,
        adjusts: &[AdjustDef],
        adjusted: &Adjusted,
    ) -> Solved<bool> {
        if !adjusted.unadjusted.is_empty() {
            return Ok(false);
        }
        let ranges = self.adjust_ranges(binding, kind, adjusts)?;
        for (setting, range) in adjusted.settings.iter().zip(ranges) {
            if magnitude(setting.offset)? > range {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// Each adjust's `within`, for this piece of hardware.
    fn adjust_ranges(
        &mut self,
        binding: &Binding,
        kind: &KindDef,
        adjusts: &[AdjustDef],
    ) -> Solved<Vec<ExactLength>> {
        self.in_kind(binding, kind, |solver, kind_env| {
            adjusts
                .iter()
                .map(|adjust| solver.kind_length(&adjust.within, kind_env))
                .collect()
        })
    }

    /// Reports the first adjustment that fails, with the help search: each
    /// listed value places the hardware again.
    fn report_adjustment(
        &mut self,
        call: Call<'_>,
        binding: &Binding,
        kind: &KindDef,
        adjusts: &[AdjustDef],
        adjusted: &Adjusted,
    ) -> Solved<()> {
        let body_name = |body: BodyId| {
            kind.bodies
                .get(body.index())
                .map(|b| b.name.as_str().to_owned())
                .unwrap_or_default()
        };
        let ranges = self.adjust_ranges(binding, kind, adjusts)?;
        let mut failing = None;
        for ((adjust, setting), range) in adjusts.iter().zip(&adjusted.settings).zip(&ranges) {
            if magnitude(setting.offset)? > *range {
                failing = Some((adjust, *setting, *range));
                break;
            }
        }
        let (message, note) = match (failing, adjusted.unadjusted.first()) {
            (Some((adjust, setting, range)), _) => {
                let (member, on) = (body_name(adjust.body), body_name(adjust.on));
                (
                    format!(
                        "{}: the {member} would sit {} off its place on the {on}; it adjusts {}",
                        call.name,
                        self.length_text(magnitude(setting.offset)?),
                        self.length_text(range),
                    ),
                    format!(
                        "the {member}'s setting {} is {}; adjust {member} on {on}: {} within {}",
                        adjust.axis.word(),
                        self.length_text(setting.offset),
                        adjust.axis.word(),
                        self.length_text(range),
                    ),
                )
            }
            (None, Some((body, axis, offset))) => {
                let member = body_name(*body);
                (
                    format!(
                        "{}: the {member} would sit {} off its place {}; it doesn't adjust that way",
                        call.name,
                        self.length_text(magnitude(*offset)?),
                        axis_phrase(*axis),
                    ),
                    format!(
                        "the kind has no adjust for the {member} {}, so its mount must put it exactly where the kind draws it",
                        axis_phrase(*axis),
                    ),
                )
            }
            (None, None) => return Ok(()),
        };
        if self.noted_with_earlier(Code::Adjust, call) {
            return Ok(());
        }
        let given = KindEnv::of_binding(binding);
        let search = self.search_lists(&given, call.span, &mut |solver, trial| {
            solver.adjustments_fit(call.hardware, binding, kind, adjusts, &trial.settings)
        });
        let help = self.search_help(&search).unwrap_or_else(|| {
            "move the parts so the hardware sits where its kind can be set, or use another kind"
                .to_owned()
        });
        self.deferred.push(
            Diagnostic::error(Code::Adjust, message, Label::new(call.file, call.span))
                .with_note(note)
                .with_help(help),
        );
        Ok(())
    }

    /// `true` when the hardware, placed again with `settings`, sits within
    /// every adjustment: one trial of the help search.
    fn adjustments_fit(
        &mut self,
        hardware: HardwareId,
        binding: &Binding,
        kind: &KindDef,
        adjusts: &[AdjustDef],
        settings: &ParamSettings,
    ) -> bool {
        // Its param values follow from the new settings, so its cache
        // starts empty.
        let trial = Binding {
            kind: binding.kind,
            name: binding.name.clone(),
            roles: binding.roles.clone(),
            settings: settings.clone(),
            glue: binding.glue,
            methods: binding.methods.clone(),
            face: binding.face,
            params: crate::eval::ParamCache::default(),
        };
        let Ok(rest) = self.rest(hardware, &trial) else {
            return false;
        };
        let Ok(adjusted) = adjusted(adjusts, &rest.offsets) else {
            return false;
        };
        self.adjustments_hold(&trial, kind, adjusts, &adjusted)
            .unwrap_or(false)
    }

    /// Checks the kind's `require`s for one piece of hardware, with `mount`
    /// bound to its mount face and its members placed where they are at
    /// rest; reports each that fails. `true` when all hold.
    fn requires_hold(
        &mut self,
        call: Call<'_>,
        binding: &Binding,
        kind: &KindDef,
        at_rest: AtRest<'_>,
    ) -> Solved<bool> {
        if kind.requires.is_empty() {
            return Ok(true);
        }
        // Its own param values: `mount` is bound here and nowhere else.
        let mut kind_env = KindEnv::new(
            binding.kind,
            &binding.roles,
            Cow::Borrowed(&binding.settings),
        );
        kind_env.mount = Some(at_rest.mount);
        kind_env.members = Some(at_rest.members);
        let Some(ctx) = self.module_ctx(DefId::Kind(binding.kind)) else {
            return Err(Fault::Internal("the file an item is declared in").into());
        };
        let env = Env {
            ctx,
            file: kind.decl.file,
            kind: Some(&kind_env),
            array: None,
        };
        let mut all = true;
        let source = CutSource::Hardware {
            hardware: call.hardware,
        };
        for (index, require) in kind.indexed_requires() {
            // A rule that can't be measured here (`contact(member, b)` for
            // a member off `b`) is reported in the kind; it names this
            // piece of hardware too, so the design shows where.
            let reported = self.diagnostics.len();
            let holds = match self.defer_wall_rule(source, binding.kind, index, require, &env) {
                Ok(true) => Ok(true),
                Ok(false) => self.eval_bool(&require.condition, &env),
                Err(failure) => Err(failure),
            };
            let holds = match holds {
                Ok(holds) => holds,
                Err(failure) => {
                    for diagnostic in self.diagnostics.iter_mut().skip(reported) {
                        diagnostic
                            .labels
                            .insert(0, Label::new(call.file, call.span));
                        diagnostic.notes.push(format!("for {}", call.name));
                    }
                    return Err(failure);
                }
            };
            if holds {
                continue;
            }
            all = false;
            let message = self.format_text(&require.message, &env)?;
            let message = format!("'{}' fails: {message}", require.decl.name);
            if self.noted_with_earlier_message(Code::Require, call, Some(&message)) {
                continue;
            }
            let mut diagnostic =
                Diagnostic::error(Code::Require, message, Label::new(call.file, call.span))
                    .with_label(Label::with_message(
                        require.decl.file,
                        require.decl.span,
                        "the kind's rule",
                    ))
                    .with_note(format!("for {}", call.name));
            if let Some(note) = self.measures_note(&require.condition, &env) {
                diagnostic = diagnostic.with_note(note);
            }
            let help = self.require_help(&require.condition, &env, "hardware");
            self.deferred.push(diagnostic.with_help(help));
        }
        Ok(all)
    }

    /// One mistake, one error: when an earlier piece of hardware of the
    /// same call (another element of its array) already has an error with
    /// this code, it gets a note `the same for <name>` and `true` comes
    /// back, so nothing more is reported for this one.
    fn noted_with_earlier(&mut self, code: Code, call: Call<'_>) -> bool {
        self.noted_with_earlier_message(code, call, None)
    }

    /// [`Solver::noted_with_earlier`], matching the message too when one
    /// is given.
    fn noted_with_earlier_message(
        &mut self,
        code: Code,
        call: Call<'_>,
        message: Option<&str>,
    ) -> bool {
        let earlier = self.deferred.iter_mut().find(|diagnostic| {
            diagnostic.code == code
                && diagnostic
                    .primary()
                    .is_some_and(|label| label.file == call.file && label.span == call.span)
                && message.is_none_or(|message| diagnostic.message == message)
        });
        let Some(earlier) = earlier else {
            return false;
        };
        let note = format!("the same for {}", call.name);
        if !earlier.notes.contains(&note) {
            earlier.notes.push(note);
        }
        true
    }

    /// A length as messages write it.
    fn length_text(&self, length: ExactLength) -> String {
        self.describe(&Value::Length(length))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(index: usize) -> BodyId {
        BodyId::new(index).unwrap()
    }

    fn mm(value: i64) -> ExactLength {
        stave_lang::units::Length::from_mm(value).to_exact()
    }

    fn adjust(member: usize, on: usize, axis: FrameAxis) -> AdjustDef {
        AdjustDef {
            body: body(member),
            on: body(on),
            axis,
            within: stave_model::defs::RExpr::error(Span::default()),
            span: Span::default(),
        }
    }

    #[test]
    fn a_setting_is_the_members_offset_less_the_others() {
        // The plate (1) is mounted 3mm across from its nominal place; the
        // arm (0) is not: the arm sits 3mm the other way on the plate.
        let offsets = [(body(1), FrameAxis::Across, mm(3))];
        let found = adjusted(&[adjust(0, 1, FrameAxis::Across)], &offsets).unwrap();
        assert_eq!(found.settings.len(), 1);
        assert_eq!(found.settings[0].offset, mm(-3));
        assert!(found.unadjusted.is_empty());
    }

    #[test]
    fn an_offset_on_an_axis_without_adjust_is_left_over() {
        let offsets = [
            (body(1), FrameAxis::Across, mm(1)),
            (body(1), FrameAxis::Depth, mm(2)),
            (body(2), FrameAxis::Depth, ExactLength::ZERO),
        ];
        let found = adjusted(&[adjust(0, 1, FrameAxis::Across)], &offsets).unwrap();
        assert_eq!(found.unadjusted, vec![(body(1), FrameAxis::Depth, mm(2))]);
    }
}
