//! The values an editor shows next to their source (grammar, "Tooling
//! and editors": hover and inlay hints): what each param and `let` works
//! out to, every part size as drawn and as cut, and every
//! `between(a, b)`, each at the span it is written at.
//!
//! The design is solved once and every value read from the same solver,
//! so a value here is the value the solved design used. A value that
//! can't be worked out (its item has an error) is left out; nothing here
//! reports a diagnostic.

use stave_lang::FileId;
use stave_lang::ast::Span;
use stave_lang::placement::PartAxis;
use stave_model::Model;
use stave_model::defs::{BuiltinCall, PartKind, RExpr, RExprKind, Size};
use stave_model::instance::Ctx;
use stave_model::trace::Amount;
use stave_schema::words::OutputUnit;

use crate::Solver;
use crate::eval::Env;

/// What a worked-out value is the value of.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ValueSite {
    /// A param's value (its default, or what an assembly gives it).
    Param,
    /// A `let`'s value.
    Let,
    /// A part's size along one of its axes.
    Size(PartAxis),
    /// A `between(a, b)`.
    Between,
}

/// One value at the span it is written at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkedValue {
    /// The file it is written in.
    pub file: FileId,
    /// What the value is shown next to: the expression (a param's
    /// default, a `let`'s value, a size's `fit` or expression, the whole
    /// `between(...)` call), or a param's name when no default is
    /// written.
    pub span: Span,
    /// The declared name, for a param or `let`: hovering it shows the
    /// value too.
    pub name: Option<Span>,
    /// What it is the value of.
    pub site: ValueSite,
    /// `true` when the expression is a plain literal (`18mm`), whose value
    /// is what is written.
    pub literal: bool,
    /// The value, exact. For a size, as drawn.
    pub value: Amount,
    /// For a size, the cut size (drawn plus what joints add).
    pub cut: Option<Amount>,
}

/// Every value an editor can show of one design.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkedOut {
    /// The project's `units:`, which values are shown in.
    pub units: OutputUnit,
    /// The values, sorted by file, span and site. An item declared inside
    /// a component or an array has a value per instance, all at the same
    /// span, in instance order.
    pub values: Vec<WorkedValue>,
}

/// Works out every value an editor shows next to its source. `None` for a
/// library, which has no design.
pub fn worked_out(model: &Model) -> Option<WorkedOut> {
    let project = model.defs.project(model.root)?;
    let mut solver = Solver::new(model, project.file, project.span);
    let design = solver.run();
    let mut values = Vec::new();
    params(&mut solver, &mut values);
    lets(&mut solver, &mut values);
    if let Some(design) = &design {
        sizes(&mut solver, design, &mut values);
    }
    requires(&mut solver, &mut values);
    values.sort_by_key(|value| (value.file, value.span.start(), value.span.end(), value.site));
    values.dedup();
    Some(WorkedOut {
        units: project.units,
        values,
    })
}

/// `true` for an expression whose value is what is written.
fn is_literal(expr: &RExpr) -> bool {
    match &expr.kind {
        RExprKind::Length(_) | RExprKind::Angle(_) | RExprKind::Number(_) => true,
        RExprKind::Unary { operand, .. } => is_literal(operand),
        RExprKind::Bool(_)
        | RExprKind::Text(_)
        | RExprKind::Fit
        | RExprKind::Word(_)
        | RExprKind::Symbol(_)
        | RExprKind::SelfElement(_)
        | RExprKind::Member { .. }
        | RExprKind::Index { .. }
        | RExprKind::Builtin(_)
        | RExprKind::KindValue { .. }
        | RExprKind::Binary { .. }
        | RExprKind::List(_)
        | RExprKind::Error => false,
    }
}

fn params(solver: &mut Solver<'_>, values: &mut Vec<WorkedValue>) {
    let model = solver.model;
    for inst in &model.params {
        let Ok(def) = model.defs.sound_param(inst.def) else {
            continue;
        };
        let Some(value) = solver
            .param_value(inst.id)
            .ok()
            .and_then(|value| Amount::from_value(&value))
        else {
            continue;
        };
        let (span, literal) = match &def.default {
            Some(default) => (default.span, is_literal(default)),
            None => (def.decl.name_span, false),
        };
        values.push(WorkedValue {
            file: def.decl.file,
            span,
            name: Some(def.decl.name_span),
            site: ValueSite::Param,
            literal,
            value,
            cut: None,
        });
        if let Some(default) = &def.default {
            let env = Env {
                ctx: Ctx {
                    instance: inst.instance,
                    element: None,
                },
                file: def.decl.file,
                kind: None,
                array: None,
            };
            betweens(solver, default, &env, values);
        }
    }
}

fn lets(solver: &mut Solver<'_>, values: &mut Vec<WorkedValue>) {
    let model = solver.model;
    for inst in &model.lets {
        let Ok(def) = model.defs.sound_let_def(inst.def) else {
            continue;
        };
        if let Some(value) = solver
            .let_value(inst.id)
            .ok()
            .and_then(|value| Amount::from_value(&value))
        {
            values.push(WorkedValue {
                file: def.decl.file,
                span: def.value.span,
                name: Some(def.decl.name_span),
                site: ValueSite::Let,
                literal: is_literal(&def.value),
                value,
                cut: None,
            });
        }
        let env = Env {
            ctx: inst.ctx,
            file: def.decl.file,
            kind: None,
            array: None,
        };
        betweens(solver, &def.value, &env, values);
    }
}

fn sizes(
    solver: &mut Solver<'_>,
    design: &stave_model::solved::SolvedDesign,
    values: &mut Vec<WorkedValue>,
) {
    let model = solver.model;
    for inst in &model.parts {
        if inst.mirror_of.is_some() {
            continue;
        }
        let Ok(def) = model.defs.sound_part(inst.def) else {
            continue;
        };
        let PartKind::FromStock(body) = &def.kind else {
            continue;
        };
        let Some(part) = design.part(inst.id) else {
            continue;
        };
        let size = |axis: PartAxis| {
            (
                Amount::Length(part.drawn.get(axis).to_exact()),
                Amount::Length(part.cut.get(axis).to_exact()),
            )
        };
        let mut written: Vec<(PartAxis, Span, bool, Option<&RExpr>)> = Vec::new();
        written.push(size_field(PartAxis::Length, &body.length));
        if let Some(width) = &body.width {
            written.push(size_field(PartAxis::Width, width));
        }
        if let Some(thickness) = &body.thickness {
            written.push((
                PartAxis::Thickness,
                thickness.span,
                is_literal(thickness),
                Some(thickness),
            ));
        }
        let env = solver.part_env(inst.id);
        for (axis, span, literal, expr) in written {
            let (drawn, cut) = size(axis);
            values.push(WorkedValue {
                file: def.decl.file,
                span,
                name: None,
                site: ValueSite::Size(axis),
                literal,
                value: drawn,
                cut: Some(cut),
            });
            if let (Some(expr), Some(env)) = (expr, &env) {
                betweens(solver, expr, env, values);
            }
        }
        if let Some(env) = &env {
            for term in &body.place {
                for expr in term_lengths(&term.kind) {
                    betweens(solver, expr, env, values);
                }
            }
        }
    }
}

/// Where a size field is written, whether it is a literal, and its
/// expression (none for `fit`).
fn size_field(axis: PartAxis, field: &Size) -> (PartAxis, Span, bool, Option<&RExpr>) {
    match field {
        Size::Fit(span) => (axis, *span, false, None),
        Size::Expr(expr) => (axis, expr.span, is_literal(expr), Some(expr)),
    }
}

/// The lengths a placement term is offset by: a gap or an inset.
fn term_lengths(term: &stave_model::defs::PlaceTermKind) -> Vec<&RExpr> {
    use stave_model::defs::PlaceTermKind;
    #[expect(
        clippy::wildcard_enum_match_arm,
        reason = "picks out the terms with a gap or an inset; the others offset by nothing"
    )]
    match term {
        PlaceTermKind::Against { gap: Some(gap), .. } => vec![gap],
        PlaceTermKind::Flush {
            inset: Some(inset), ..
        } => vec![inset],
        _ => Vec::new(),
    }
}

fn requires(solver: &mut Solver<'_>, values: &mut Vec<WorkedValue>) {
    let model = solver.model;
    for inst in &model.requires {
        let Ok(def) = model.defs.sound_require(inst.def) else {
            continue;
        };
        let env = Env {
            ctx: inst.ctx,
            file: def.decl.file,
            kind: None,
            array: None,
        };
        betweens(solver, &def.condition, &env, values);
    }
}

/// Every `between(a, b)` in an expression, with its value where `env`
/// evaluates it.
fn betweens(solver: &mut Solver<'_>, expr: &RExpr, env: &Env<'_>, values: &mut Vec<WorkedValue>) {
    match &expr.kind {
        RExprKind::Builtin(call) => {
            if let BuiltinCall::Between { .. } = call.as_ref()
                && let Ok(value) = solver.eval(expr, env)
                && let Some(value) = Amount::from_value(&value)
            {
                values.push(WorkedValue {
                    file: env.file,
                    span: expr.span,
                    name: None,
                    site: ValueSite::Between,
                    literal: false,
                    value,
                    cut: None,
                });
            }
        }
        RExprKind::Unary { operand, .. } => betweens(solver, operand, env, values),
        RExprKind::Binary { lhs, rhs, .. } => {
            betweens(solver, lhs, env, values);
            betweens(solver, rhs, env, values);
        }
        RExprKind::List(items) => {
            for item in items {
                betweens(solver, item, env, values);
            }
        }
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
        | RExprKind::KindValue { .. }
        | RExprKind::Error => {}
    }
}
