//! `contact(a, b)`: the patch where two parts touch (grammar, "Built-in
//! functions"), and what a failed rule that reads it says.
//!
//! The patch is the intersection of the two parts' drawn boxes: the same
//! box arithmetic as a `crossing` cut's footprint, taken before joints
//! lengthen the parts, so a tenon reaching into its mortise doesn't count
//! as touching. A turned part that touches with an end touches only over
//! the square holding that end's circle.

use stave_lang::Code;
use stave_lang::ast::{BinaryOp, Span};
use stave_lang::placement::{Axis, PartAxis};
use stave_lang::units::Rational;
use stave_model::defs::{BuiltinCall, Member, RExpr, RExprKind, Symbol};
use stave_model::ids::PartId;
use stave_model::value::{ExactBox, Patch, Value};
use stave_schema::functions::{Builtin, PatchSize};

use crate::eval::{Env, midpoint};
use crate::{Fault, Solved, Solver};

/// Two touching parts' boxes, each narrowed to where it can touch, and the
/// world axis square to where they meet.
struct Touching {
    a: ExactBox,
    b: ExactBox,
    normal: Axis,
}

/// The word for a part's surface square to one of its axes.
fn surface_word(axis: PartAxis) -> &'static str {
    match axis {
        PartAxis::Length => "end",
        PartAxis::Width => "edge",
        PartAxis::Thickness => "face",
    }
}

/// How a comparison's needed side is written after "needs".
fn needs_phrase(op: BinaryOp) -> Option<&'static str> {
    match op {
        BinaryOp::Eq => Some(""),
        BinaryOp::Ge => Some("at least "),
        BinaryOp::Gt => Some("more than "),
        BinaryOp::Le => Some("at most "),
        BinaryOp::Lt => Some("less than "),
        BinaryOp::Ne
        | BinaryOp::And
        | BinaryOp::Or
        | BinaryOp::Add
        | BinaryOp::Sub
        | BinaryOp::Mul
        | BinaryOp::Div
        | BinaryOp::In => None,
    }
}

/// The first `contact(a, b).length` or `.width` in an expression, as the
/// two part expressions and the size read.
fn patch_size_read(expr: &RExpr) -> Option<(&RExpr, &RExpr, PatchSize)> {
    if let RExprKind::Member {
        base,
        member: Member::PatchSize(size),
    } = &expr.kind
        && let RExprKind::Builtin(call) = &base.kind
        && let BuiltinCall::Contact { a, b } = &**call
    {
        return Some((a, b, *size));
    }
    match &expr.kind {
        RExprKind::Member { base, .. } | RExprKind::Index { base, .. } => patch_size_read(base),
        RExprKind::Unary { operand, .. } => patch_size_read(operand),
        RExprKind::Binary { lhs, rhs, .. } => patch_size_read(lhs).or_else(|| patch_size_read(rhs)),
        RExprKind::Builtin(call) => call.arguments().find_map(patch_size_read),
        RExprKind::KindValue { settings, .. } => settings
            .iter()
            .find_map(|setting| patch_size_read(&setting.value)),
        RExprKind::List(items) => items.iter().find_map(patch_size_read),
        RExprKind::Length(_)
        | RExprKind::Angle(_)
        | RExprKind::Number(_)
        | RExprKind::Bool(_)
        | RExprKind::Text(_)
        | RExprKind::Fit
        | RExprKind::Word(_)
        | RExprKind::Symbol(_)
        | RExprKind::SelfElement(_)
        | RExprKind::Wall(_)
        | RExprKind::Error => None,
    }
}

/// The kind params an expression reads, by index, in the order they are
/// first written.
fn params_read(expr: &RExpr, out: &mut Vec<usize>) {
    match &expr.kind {
        RExprKind::Symbol(Symbol::KindParam(_, index)) => {
            if !out.contains(index) {
                out.push(*index);
            }
        }
        RExprKind::Member { base, .. } | RExprKind::Index { base, .. } => params_read(base, out),
        RExprKind::Unary { operand, .. } => params_read(operand, out),
        RExprKind::Binary { lhs, rhs, .. } => {
            params_read(lhs, out);
            params_read(rhs, out);
        }
        RExprKind::Builtin(call) => {
            for argument in call.arguments() {
                params_read(argument, out);
            }
        }
        RExprKind::KindValue { settings, .. } => {
            for setting in settings {
                params_read(&setting.value, out);
            }
        }
        RExprKind::List(items) => {
            for item in items {
                params_read(item, out);
            }
        }
        RExprKind::Symbol(
            Symbol::Def(_) | Symbol::Role(..) | Symbol::MadePart(..) | Symbol::Mount(_),
        )
        | RExprKind::Length(_)
        | RExprKind::Angle(_)
        | RExprKind::Number(_)
        | RExprKind::Bool(_)
        | RExprKind::Text(_)
        | RExprKind::Fit
        | RExprKind::Word(_)
        | RExprKind::SelfElement(_)
        | RExprKind::Wall(_)
        | RExprKind::Error => {}
    }
}

impl Solver<'_> {
    /// One part, for an argument of `contact` or `edge_to_edge`: a
    /// `Parts` role is one part in each pair.
    pub(crate) fn one_part(
        &mut self,
        function: Builtin,
        expr: &RExpr,
        env: &Env<'_>,
    ) -> Solved<PartId> {
        let value = self.eval(expr, env)?;
        match value.part_list().as_deref() {
            Some([one]) => Ok(*one),
            _ => {
                let word = function.word();
                Err(self
                    .error(
                        Code::TypeMismatch,
                        format!("'{word}' needs one part, found {}", value.description()),
                        env.file,
                        expr.span,
                        format!("name one part on each side: {word}(piece, into)"),
                    )
                    .into())
            }
        }
    }

    /// `contact(a, b)`: the patch where they touch, or `error[no-contact]`.
    pub(crate) fn contact(
        &mut self,
        a: PartId,
        b: PartId,
        env: &Env<'_>,
        span: Span,
    ) -> Solved<Patch> {
        if let Some(patch) = self.contact_patch(a, b)? {
            return Ok(patch);
        }
        let (a_name, b_name) = (self.part_name(a), self.part_name(b));
        let (message, help) = if a == b {
            (
                format!("'contact' needs two different parts, found {a_name} twice"),
                "name the two parts that touch: contact(piece, into)".to_owned(),
            )
        } else {
            (
                format!("{a_name} and {b_name} don't touch"),
                format!(
                    "contact measures where two parts meet; place {a_name} so it lies against {b_name}"
                ),
            )
        };
        Err(self
            .error(Code::NoContact, message, env.file, span, help)
            .into())
    }

    /// The patch where two parts touch, or `None` when they don't touch
    /// over an area.
    pub(crate) fn contact_patch(&mut self, a: PartId, b: PartId) -> Solved<Option<Patch>> {
        Ok(match self.touching(a, b)? {
            Some(touching) => {
                // The second part lies on the side of the first its middle
                // is on; parts crossing with one middle face `a`'s way.
                let normal = touching.normal;
                let middle = |part: &ExactBox| midpoint(part.min(normal), part.max(normal));
                let positive = middle(&touching.b) >= middle(&touching.a);
                Patch::new(touching.a.intersection(&touching.b), normal)
                    .map(|patch| patch.facing(positive))
            }
            None => None,
        })
    }

    /// The two parts' boxes where they can touch, and the axis square to
    /// the patch: where they only touch, the axis they touch along; where
    /// they cross (a half lap), the axis of `a`'s thickness.
    fn touching(&mut self, a: PartId, b: PartId) -> Solved<Option<Touching>> {
        if a == b {
            return Ok(None);
        }
        let (box_a, box_b) = (self.drawn_box(a)?, self.drawn_box(b)?);
        if !box_a.meets(&box_b) {
            return Ok(None);
        }
        let normal = match box_a.touching_axis(&box_b) {
            Some(axis) => axis,
            None => self
                .part_orientation(a)?
                .direction(PartAxis::Thickness)
                .axis(),
        };
        let narrowed_a = self.turned_end(a, box_a, &box_b, normal)?;
        let narrowed_b = self.turned_end(b, box_b, &box_a, normal)?;
        Ok(Some(Touching {
            a: narrowed_a,
            b: narrowed_b,
            normal,
        }))
    }

    /// A turned part that touches `other` with an end touches only over
    /// the square holding that end's circle; any other part, or a turned
    /// part touching along its side, keeps its whole box.
    fn turned_end(
        &mut self,
        part: PartId,
        own: ExactBox,
        other: &ExactBox,
        normal: Axis,
    ) -> Solved<ExactBox> {
        let Some(profile) = self.part_profile(part)? else {
            return Ok(own);
        };
        let length = self.part_orientation(part)?.direction(PartAxis::Length);
        if length.axis() != normal {
            return Ok(own);
        }
        let toward_positive = if own.max(normal) == other.min(normal) {
            true
        } else if own.min(normal) == other.max(normal) {
            false
        } else {
            return Ok(own);
        };
        // Station 0 is at the part's start; its last station at its end.
        let stations = self.profile_stations(profile)?;
        let station = if length.is_positive() == toward_positive {
            stations.last()
        } else {
            stations.first()
        };
        let Some((_, diameter)) = station else {
            return Ok(own);
        };
        let half = diameter
            .checked_div(Rational::from(2i64))
            .ok_or(Fault::Overflow)?;
        let mut narrowed = own;
        for axis in Axis::ALL.into_iter().filter(|axis| *axis != normal) {
            let centre = midpoint(own.min(axis), own.max(axis)).ok_or(Fault::Overflow)?;
            let low = centre.checked_sub(half).ok_or(Fault::Overflow)?;
            let high = centre.checked_add(half).ok_or(Fault::Overflow)?;
            narrowed = narrowed.with(axis, low, high);
        }
        Ok(narrowed)
    }

    /// The comparison of a failed condition that fails: the condition
    /// itself, or the first failing one of conditions joined by `&&`.
    fn failing_comparison<'c>(&mut self, condition: &'c RExpr, env: &Env<'_>) -> Option<&'c RExpr> {
        match &condition.kind {
            RExprKind::Binary {
                op: BinaryOp::And,
                lhs,
                rhs,
            } => {
                if self.eval_bool(lhs, env).ok()? {
                    self.failing_comparison(rhs, env)
                } else {
                    self.failing_comparison(lhs, env)
                }
            }
            RExprKind::Binary { op, .. } => needs_phrase(*op).map(|_| condition),
            RExprKind::Length(_)
            | RExprKind::Angle(_)
            | RExprKind::Number(_)
            | RExprKind::Bool(_)
            | RExprKind::Text(_)
            | RExprKind::Fit
            | RExprKind::Word(_)
            | RExprKind::Symbol(_)
            | RExprKind::SelfElement(_)
            | RExprKind::Member { .. }
            | RExprKind::Index { .. }
            | RExprKind::Builtin(_)
            | RExprKind::KindValue { .. }
            | RExprKind::Unary { .. }
            | RExprKind::List(_)
            | RExprKind::Wall(_)
            | RExprKind::Error => None,
        }
    }

    /// For a failed rule that compares two values: what each side came
    /// to, `the rule measures 5.5mm and needs at least 16mm`. The left side
    /// is what is measured, the right what the rule needs; an equality
    /// needs its value exactly.
    pub(crate) fn measures_note(&mut self, condition: &RExpr, env: &Env<'_>) -> Option<String> {
        let comparison = self.failing_comparison(condition, env)?;
        let RExprKind::Binary { op, lhs, rhs } = &comparison.kind else {
            return None;
        };
        let phrase = needs_phrase(*op)?;
        let measured = self.eval(lhs, env).ok()?;
        let needed = self.eval(rhs, env).ok()?;
        Some(format!(
            "the rule measures {} and needs {phrase}{}",
            self.describe(&measured),
            self.describe(&needed)
        ))
    }

    /// For a failed rule that reads `contact(a, b).length` or `.width`
    /// where one part covers only some of the other's end, edge or face
    /// along that side: says how much, and what to change.
    pub(crate) fn contact_help(&mut self, condition: &RExpr, env: &Env<'_>) -> Option<String> {
        let comparison = self.failing_comparison(condition, env)?;
        let (a, b, size) = patch_size_read(comparison)?;
        let a = self.eval(a, env).ok()?.parts()?;
        let b = self.eval(b, env).ok()?.parts()?;
        let (&[a], &[b]) = (a.as_slice(), b.as_slice()) else {
            return None;
        };
        let touching = self.touching(a, b).ok()??;
        let patch = Patch::new(touching.a.intersection(&touching.b), touching.normal)?;
        let axis = patch.size_axis(size);
        let covered = patch.size(size);
        let (a_size, b_size) = (touching.a.size(axis)?, touching.b.size(axis)?);
        // The part whose surface is only partly covered is the one that
        // is narrower along this side.
        let (short, short_size, other) = if a_size <= b_size {
            (a, a_size, b)
        } else {
            (b, b_size, a)
        };
        if covered >= short_size {
            return None;
        }
        let local = self.part_orientation(short).ok()?.part_axis(patch.normal());
        let surface = surface_word(local);
        let (short_name, other_name) = (self.part_name(short), self.part_name(other));
        Some(format!(
            "{other_name} covers only {} of {short_name}'s {} {surface}; move or resize {other_name} so it covers the whole {surface}",
            self.describe(&Value::Length(covered)),
            self.describe(&Value::Length(short_size)),
        ))
    }

    /// The help for a kind's failed `require`: for a rule that asks how
    /// the parts meet, to choose another kind; else the list search's, then
    /// what a contact patch says, then the general help, naming the kind's
    /// params the failing comparison reads (`change the joint's count,
    /// diameter or wall, or the parts' sizes, so the rule holds`). `owner`
    /// is `joint` or `hardware`.
    pub(crate) fn require_help(&mut self, condition: &RExpr, env: &Env<'_>, owner: &str) -> String {
        if crate::meeting::asks_how_parts_meet(condition) {
            return format!(
                "choose another {owner} kind, one made for parts that meet the way these do, or place the parts as the rule says"
            );
        }
        if let Some(kind) = env.kind {
            let search = self.help_search(condition, kind);
            if let Some(help) = self.search_help(&search) {
                return help;
            }
        }
        if let Some(help) = self.contact_help(condition, env) {
            return help;
        }
        let general =
            format!("change the {owner}'s settings or the parts' sizes so the rule holds");
        let Some(kind) = env.kind else {
            return general;
        };
        let comparison = self.failing_comparison(condition, env).unwrap_or(condition);
        let mut indexes = Vec::new();
        params_read(comparison, &mut indexes);
        let names: Vec<&str> = match self.model.defs.sound_kind(kind.kind).ok() {
            Some(def) => indexes
                .iter()
                .filter_map(|index| def.params.get(*index))
                .map(|param| param.decl.name.as_str())
                .collect(),
            None => Vec::new(),
        };
        match names.as_slice() {
            [] => general,
            [one] => format!("change the {owner}'s {one}, or the parts' sizes, so the rule holds"),
            [rest @ .., last] => format!(
                "change the {owner}'s {} or {last}, or the parts' sizes, so the rule holds",
                rest.join(", ")
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use stave_lang::units::{ExactLength, Length};

    fn mm(value: i64) -> ExactLength {
        Length::from_mm(value).to_exact()
    }

    #[test]
    fn a_patch_has_its_longer_side_as_its_length() {
        let area = ExactBox::new((mm(0), mm(19)), (mm(100), mm(100)), (mm(0), mm(560)));
        let patch = Patch::new(area, Axis::Up).unwrap();
        assert_eq!(patch.size(PatchSize::Length), mm(560));
        assert_eq!(patch.size(PatchSize::Width), mm(19));
        assert_eq!(patch.size_axis(PatchSize::Length), Axis::Deep);
        assert_eq!(patch.axis(PartAxis::Thickness), Axis::Up);
    }

    #[test]
    fn touching_only_along_an_edge_is_no_patch() {
        let area = ExactBox::new((mm(0), mm(0)), (mm(100), mm(100)), (mm(0), mm(560)));
        assert!(Patch::new(area, Axis::Up).is_none());
    }

    #[test]
    fn every_comparison_has_a_phrase_and_arithmetic_has_none() {
        assert_eq!(needs_phrase(BinaryOp::Ge), Some("at least "));
        assert_eq!(needs_phrase(BinaryOp::Eq), Some(""));
        assert_eq!(needs_phrase(BinaryOp::Add), None);
    }
}
