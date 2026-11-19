//! The dependency trace of a size (grammar, "Smart edits"): how a part's
//! size, a region's size, a placement offset, a param or a share depends on
//! the params, stock fields and kinds' sizes it is worked out from.
//!
//! The trace follows the same path the solver takes, term by term, but
//! carries each value as an exact linear form (`constant + Σ coefficient ×
//! value`) instead of a number. Where the solver makes a choice from where
//! things are (the side a part stands on, which way a spaced row runs, the
//! piece of a split a region is), the trace reads the choice the solver
//! made and holds it: that is what "with everything else held" means. So
//! the trace runs on a solved design, and it checks its answer against the
//! solver's value before returning it.
//!
//! It follows `let`s, params whose value is an expression (a default, or
//! what an assembly gives a component), region functions, faces and
//! placement terms. It stops at a param written as a number (a candidate),
//! a `locked param`, a stock's field and a kind's own size. Something with
//! no linear form (a product of two values that both change, a division by
//! one, `between` and `extent`, a `fit` that more than one part could end)
//! makes the answer [`Form::NotLinear`]. Linearity is then decided for each
//! value on its own, as rule 2 of "Smart edits" asks: with every other
//! value held, `width * factor + extra` is still linear in each of the
//! three, and only a value multiplied by itself, divided by, or deciding a
//! `fit` or a measure is named as not linear.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use stave_lang::ast::{BinaryOp, ParamKind, Span, UnaryOp};
use stave_lang::placement::{Axis, Face, PartAxis, PerAxis};
use stave_lang::units::{ExactLength, Rational};
use stave_model::Model;
use stave_model::defs::{
    AssemblyKind, DefId, Domain, Member, PartKind, RExpr, RExprKind, Setting, Size, StockField,
    Symbol, Word,
};
use stave_model::ids::{
    AssemblyId, InstanceId, LetId, ParamId, PartId, RegionId, RequireId, StockId,
};
use stave_model::instance::{Ctx, InstanceKind, ItemSlot, ParamSource};
use stave_model::trace::{
    Amount, Dependence, Followed, Form, Freedom, Linear, NotLinear, ParamDomain, Quantity, Reason,
    Source, Term, Trace,
};
use stave_model::value::{RegionOrigin, Spacing, Value};
use stave_schema::functions::Builtin;

use crate::Solver;
use crate::eval::Env;
use crate::place::{AxisTerm, AxisTermKind};

/// Why a size could not be traced.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TraceError {
    /// The model is a library: there is nothing placed to trace.
    #[error("a library has no design to trace")]
    NoProject,
    /// The id does not name an item of the model.
    #[error("the item is not in this design")]
    UnknownItem,
    /// The size could not be worked out: the design has an error that
    /// reaches it.
    #[error("the size can't be worked out; fix the errors it depends on first")]
    NotWorkedOut,
    /// The value is not a number, a length or an angle.
    #[error("the value is not a quantity")]
    NotAQuantity,
    /// The placement term fixing that axis has no gap or inset.
    #[error("the placement term on this axis has no gap or inset")]
    NoOffset,
    /// The region is not a piece of a split written with `shares:`.
    #[error("the region is not a piece of a split with shares")]
    NoShares,
    /// A require's condition is not comparisons joined by `&&`
    /// ([`trace_requires`]).
    #[error("the require is not comparisons joined by '&&'")]
    NotAComparison,
    /// The trace and the solver disagree; a bug in the trace.
    #[error("the trace gives {traced:?}, but the solver gives {solved:?}")]
    Disagrees {
        /// What the trace gives.
        traced: Amount,
        /// What the solver gives.
        solved: Amount,
    },
}

/// Traces one size of the design `model` describes. The design is solved
/// first; to trace several sizes of one design, use [`trace_all`], which
/// solves it once.
pub fn trace(model: &Model, quantity: Quantity) -> Result<Trace, TraceError> {
    trace_all(model, &[quantity])
        .into_iter()
        .next()
        .unwrap_or(Err(TraceError::NoProject))
}

/// Traces several sizes of one design, solving it once. The results are in
/// the order of `quantities`.
pub fn trace_all(model: &Model, quantities: &[Quantity]) -> Vec<Result<Trace, TraceError>> {
    let Some(project) = model.defs.project(model.root) else {
        return quantities
            .iter()
            .map(|_| Err(TraceError::NoProject))
            .collect();
    };
    let mut solver = Solver::new(model, project.file, project.span);
    // Every value the trace reads is worked out on demand and remembered;
    // running the solver first fills in the choices it holds.
    let _ = solver.run();
    quantities
        .iter()
        .map(|quantity| Tracer::new(&mut solver, *quantity).and_then(|t| t.run()))
        .collect()
}

/// How the two sides of one comparison in a `require` compare.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Comparison {
    /// `==`
    Equal,
    /// `<`
    Less,
    /// `<=`
    LessOrEqual,
    /// `>`
    Greater,
    /// `>=`
    GreaterOrEqual,
}

impl Comparison {
    fn of(op: BinaryOp) -> Option<Comparison> {
        match op {
            BinaryOp::Eq => Some(Comparison::Equal),
            BinaryOp::Lt => Some(Comparison::Less),
            BinaryOp::Le => Some(Comparison::LessOrEqual),
            BinaryOp::Gt => Some(Comparison::Greater),
            BinaryOp::Ge => Some(Comparison::GreaterOrEqual),
            BinaryOp::Or
            | BinaryOp::And
            | BinaryOp::Ne
            | BinaryOp::Add
            | BinaryOp::Sub
            | BinaryOp::Mul
            | BinaryOp::Div
            | BinaryOp::In => None,
        }
    }
}

/// One comparison of a require's condition, traced as `left − right`
/// compared with zero: `left >= right` is `left − right >= 0`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TracedComparison {
    /// How the sides compare.
    pub comparison: Comparison,
    /// `left − right` at the current values, exact.
    pub value: Amount,
    /// How `left − right` depends on the values it is worked out from.
    /// The [`Dependence::levels`] in it count from the require.
    pub form: Form,
}

/// Traces the condition of every `require` the project's design has (the
/// project's own and those inside its assemblies' components; a mirrored
/// assembly's copies are left out, as the solver leaves them out), solving
/// the design once. A condition made of comparisons joined by `&&` gives
/// one [`TracedComparison`] per comparison, in the order written;
/// anything else (`||`, `!=`, `!`) is [`TraceError::NotAComparison`]. The
/// results are in the order of the model's requires. The requires of
/// kinds (a joint's, a board's) are not here: they are checked where the
/// kind is used.
pub fn trace_requires(
    model: &Model,
) -> Vec<(RequireId, Result<Vec<TracedComparison>, TraceError>)> {
    let Some(project) = model.defs.project(model.root) else {
        return Vec::new();
    };
    let mut solver = Solver::new(model, project.file, project.span);
    let _ = solver.run();
    model
        .requires
        .iter()
        .filter(|inst| inst.mirror_of.is_none())
        .map(|inst| (inst.id, trace_require(&mut solver, inst.id)))
        .collect()
}

fn trace_require(
    solver: &mut Solver<'_>,
    require: RequireId,
) -> Result<Vec<TracedComparison>, TraceError> {
    let model = solver.model;
    let inst = model
        .requires
        .get(require.index())
        .ok_or(TraceError::UnknownItem)?;
    let def = model
        .defs
        .require(inst.def)
        .ok_or(TraceError::UnknownItem)?;
    let env = Env {
        ctx: inst.ctx,
        file: def.decl.file,
        kind: None,
        array: None,
    };
    let mut comparisons = Vec::new();
    let mut pending = vec![&def.condition];
    while let Some(expr) = pending.pop() {
        let RExprKind::Binary { op, lhs, rhs } = &expr.kind else {
            return Err(TraceError::NotAComparison);
        };
        if *op == BinaryOp::And {
            pending.push(rhs);
            pending.push(lhs);
            continue;
        }
        let comparison = Comparison::of(*op).ok_or(TraceError::NotAComparison)?;
        let mut tracer = Tracer::at(solver, inst.ctx.instance);
        let halt = |Halt| TraceError::NotWorkedOut;
        let left = tracer.value(lhs, &env).map_err(halt)?;
        let right = tracer.value(rhs, &env).map_err(halt)?;
        let (Some(left), Some(right)) = (Amount::from_value(&left), Amount::from_value(&right))
        else {
            return Err(TraceError::NotAQuantity);
        };
        if !left.same_dimension(right) {
            return Err(TraceError::NotAQuantity);
        }
        let raw = left
            .raw()
            .checked_sub(right.raw())
            .ok_or(TraceError::NotWorkedOut)?;
        let value = left.with_raw(raw);
        let left_form = tracer.affine(lhs, &env).map_err(halt)?;
        let right_form = tracer.affine(rhs, &env).map_err(halt)?;
        let form = left_form.minus(&right_form).map_err(halt)?.settled();
        comparisons.push(TracedComparison {
            comparison,
            value,
            form: tracer.form_of(value, form)?,
        });
    }
    Ok(comparisons)
}

// ---------------------------------------------------------------------------
// Linear forms
// ---------------------------------------------------------------------------

/// A value as a form over the values it is worked out from, in raw units
/// (see [`Amount::raw`]).
///
/// When `reasons` is empty the form is exact: the value is `constant + Σ
/// coefficient × value` whatever the values are. Otherwise it is not linear
/// as a whole, and the form says, for each value, what it is with every
/// other value held at its current value (grammar, "Smart edits", rule 2):
/// a value in `terms` is still `a × p + b`, with its coefficient `a` worked
/// out at the other values' current values; a value in `bent` is not (it is
/// multiplied by itself, divided by, or decides a `fit` or a measure).
/// `constant` then means nothing and is zero.
#[derive(Debug, Clone, PartialEq)]
struct Affine {
    constant: Rational,
    terms: BTreeMap<Source, Rational>,
    bent: BTreeSet<Source>,
    reasons: BTreeSet<Reason>,
    /// Part of the value is something the trace does not follow, which may
    /// depend on any value: no coefficient beside it can be trusted.
    opaque: bool,
}

/// The trace could not work a value out; the solver has said why.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Halt;

type Step<T> = Result<T, Halt>;

fn ratio(numerator: i128, denominator: i128) -> Step<Rational> {
    Rational::new(numerator, denominator).ok_or(Halt)
}

impl Affine {
    fn constant(value: Rational) -> Affine {
        Affine {
            constant: value,
            terms: BTreeMap::new(),
            bent: BTreeSet::new(),
            reasons: BTreeSet::new(),
            opaque: false,
        }
    }

    fn zero() -> Affine {
        Affine::constant(Rational::ZERO)
    }

    fn length(length: ExactLength) -> Affine {
        Affine::constant(length.value())
    }

    fn source(source: Source) -> Affine {
        Affine {
            terms: BTreeMap::from([(source, Rational::ONE)]),
            ..Affine::zero()
        }
    }

    /// Something the trace does not follow.
    fn other(what: &str) -> Affine {
        Affine {
            reasons: BTreeSet::from([Reason::Other(what.to_owned())]),
            opaque: true,
            ..Affine::zero()
        }
    }

    /// `true` when the form is exactly linear as a whole.
    fn is_exact(&self) -> bool {
        self.reasons.is_empty()
    }

    /// Every value it depends on.
    fn keys(&self) -> BTreeSet<Source> {
        let mut keys: BTreeSet<Source> = self.terms.keys().cloned().collect();
        keys.extend(self.bent.iter().cloned());
        keys
    }

    /// The value, when it depends on nothing.
    fn as_constant(&self) -> Option<Rational> {
        (self.is_exact() && self.terms.is_empty() && !self.opaque).then_some(self.constant)
    }

    /// Keeps the invariants: a value that is bent anywhere is bent, an
    /// opaque form trusts no coefficient, and a form that is not exact has
    /// no constant.
    fn settled(mut self) -> Affine {
        if self.opaque {
            let terms = std::mem::take(&mut self.terms);
            self.bent.extend(terms.into_keys());
        }
        let bent = &self.bent;
        self.terms.retain(|source, _| !bent.contains(source));
        if !self.is_exact() {
            self.constant = Rational::ZERO;
        }
        self
    }

    /// Not linear for `reason` in every value `parts` depend on: something
    /// with no linear form in any of them (a `fit` several parts could end,
    /// a measure, a share over the sum of shares).
    fn not_linear(reason: Reason, parts: &[&Affine]) -> Affine {
        let mut form = Affine::zero();
        form.reasons.insert(reason);
        for part in parts {
            form.reasons.extend(part.reasons.iter().cloned());
            form.bent.extend(part.keys());
            form.opaque |= part.opaque;
        }
        form
    }

    /// Not linear, for the reasons of the parts that are not, in every
    /// value the parts depend on.
    fn joined(parts: &[&Affine]) -> Affine {
        let mut form = Affine::zero();
        for part in parts {
            form.reasons.extend(part.reasons.iter().cloned());
            form.bent.extend(part.keys());
            form.opaque |= part.opaque;
        }
        if form.is_exact() && !form.bent.is_empty() {
            form.reasons
                .insert(Reason::Other("a value with no linear form".to_owned()));
        }
        form
    }

    fn plus(&self, other: &Affine) -> Step<Affine> {
        let mut terms = self.terms.clone();
        for (source, coefficient) in &other.terms {
            let sum = match terms.get(source) {
                Some(existing) => existing.checked_add(*coefficient).ok_or(Halt)?,
                None => *coefficient,
            };
            if sum == Rational::ZERO {
                terms.remove(source);
            } else {
                terms.insert(source.clone(), sum);
            }
        }
        let mut reasons = self.reasons.clone();
        reasons.extend(other.reasons.iter().cloned());
        let constant = if reasons.is_empty() {
            self.constant.checked_add(other.constant).ok_or(Halt)?
        } else {
            Rational::ZERO
        };
        let mut bent = self.bent.clone();
        bent.extend(other.bent.iter().cloned());
        Ok(Affine {
            constant,
            terms,
            bent,
            reasons,
            opaque: self.opaque || other.opaque,
        }
        .settled())
    }

    fn scaled(&self, factor: Rational) -> Step<Affine> {
        // Nothing times zero is nothing, whatever it depends on.
        if factor == Rational::ZERO {
            return Ok(Affine::zero());
        }
        let mut terms = BTreeMap::new();
        for (source, coefficient) in &self.terms {
            terms.insert(source.clone(), coefficient.checked_mul(factor).ok_or(Halt)?);
        }
        Ok(Affine {
            constant: self.constant.checked_mul(factor).ok_or(Halt)?,
            terms,
            ..self.clone()
        })
    }

    fn minus(&self, other: &Affine) -> Step<Affine> {
        self.plus(&other.scaled(Rational::from(-1_i64))?)
    }

    /// The product. `values` are the current values of the two factors;
    /// with one factor held, the other's values keep their form scaled by
    /// it. A value in both factors is multiplied by itself and is bent.
    fn times(&self, other: &Affine, values: (Option<Rational>, Option<Rational>)) -> Step<Affine> {
        if let Some(factor) = self.as_constant() {
            return other.scaled(factor);
        }
        if let Some(factor) = other.as_constant() {
            return self.scaled(factor);
        }
        let mut form = Affine::zero();
        form.reasons.insert(Reason::Product);
        for (part, held) in [(self, values.1), (other, values.0)] {
            form.reasons.extend(part.reasons.iter().cloned());
            form.bent.extend(part.bent.iter().cloned());
            form.opaque |= part.opaque;
            for (source, coefficient) in &part.terms {
                match held {
                    Some(held) => {
                        let scaled = coefficient.checked_mul(held).ok_or(Halt)?;
                        if scaled != Rational::ZERO {
                            form.terms.insert(source.clone(), scaled);
                        }
                    }
                    None => {
                        form.bent.insert(source.clone());
                    }
                }
            }
        }
        form.bent
            .extend(self.keys().intersection(&other.keys()).cloned());
        Ok(form.settled())
    }

    /// The quotient. `divisor` is the current value of `other`; a value
    /// the divisor depends on is bent, and the dividend's values keep their
    /// form divided by it.
    fn over(&self, other: &Affine, divisor: Option<Rational>) -> Step<Affine> {
        if let Some(constant) = other.as_constant() {
            if constant == Rational::ZERO {
                return Err(Halt);
            }
            return self.scaled(Rational::ONE.checked_div(constant).ok_or(Halt)?);
        }
        let mut form = Affine::zero();
        form.reasons.insert(Reason::Division);
        form.reasons.extend(self.reasons.iter().cloned());
        form.reasons.extend(other.reasons.iter().cloned());
        form.bent.extend(self.bent.iter().cloned());
        form.bent.extend(other.keys());
        form.opaque = self.opaque || other.opaque;
        let inverse = divisor
            .filter(|d| *d != Rational::ZERO)
            .and_then(|d| Rational::ONE.checked_div(d));
        for (source, coefficient) in &self.terms {
            match inverse {
                Some(inverse) => {
                    form.terms.insert(
                        source.clone(),
                        coefficient.checked_mul(inverse).ok_or(Halt)?,
                    );
                }
                None => {
                    form.bent.insert(source.clone());
                }
            }
        }
        Ok(form.settled())
    }

    fn half(&self) -> Step<Affine> {
        self.scaled(ratio(1, 2)?)
    }

    fn whole(count: usize) -> Step<Rational> {
        Ok(Rational::from(i64::try_from(count).map_err(|_| Halt)?))
    }
}

/// The low and high ends of a box along each axis, as linear forms.
#[derive(Debug, Clone, PartialEq)]
struct Bounds(PerAxis<(Affine, Affine)>);

impl Bounds {
    fn new(across: (Affine, Affine), up: (Affine, Affine), deep: (Affine, Affine)) -> Bounds {
        Bounds(PerAxis { across, up, deep })
    }

    fn other(what: &str) -> Bounds {
        Bounds(PerAxis::from_fn(|_| {
            (Affine::other(what), Affine::other(what))
        }))
    }

    fn get(&self, axis: Axis) -> Step<&(Affine, Affine)> {
        Ok(&self.0[axis])
    }

    fn low(&self, axis: Axis) -> Step<Affine> {
        Ok(self.get(axis)?.0.clone())
    }

    fn high(&self, axis: Axis) -> Step<Affine> {
        Ok(self.get(axis)?.1.clone())
    }

    fn face(&self, face: Face) -> Step<Affine> {
        if face.is_positive() {
            self.high(face.axis())
        } else {
            self.low(face.axis())
        }
    }

    fn size(&self, axis: Axis) -> Step<Affine> {
        let (low, high) = self.get(axis)?;
        high.minus(low)
    }

    fn with(&self, axis: Axis, low: Affine, high: Affine) -> Bounds {
        let mut bounds = self.clone();
        bounds.0[axis] = (low, high);
        bounds
    }
}

/// The overlays of an `overlay(...)` mount, as linear forms.
struct Overlays {
    left: Affine,
    right: Affine,
    top: Affine,
    bottom: Affine,
    meeting: Option<Affine>,
}

/// A mount as linear forms.
enum MountForm {
    Overlay(Box<Overlays>),
    Inset { reveal: Affine },
}

fn arg<'a>(args: &'a [Setting], name: &str) -> Option<&'a RExpr> {
    args.iter().find(|a| a.name == name).map(|a| &a.value)
}

// ---------------------------------------------------------------------------
// The tracer
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Busy {
    Param(ParamId),
    Let(LetId),
    Size(PartId, PartAxis),
    Min(PartId, Axis),
    Region(RegionId),
}

/// The longest chain of names `peel` follows, so a cycle the solver
/// reported can't loop.
const PEEL_LIMIT: usize = 64;

struct Tracer<'s, 'm> {
    solver: &'s mut Solver<'m>,
    /// What is traced; `None` for the sides of a require
    /// ([`trace_requires`]), which are traced as expressions.
    quantity: Option<Quantity>,
    /// Component levels the quantity is inside.
    depth: u32,
    dependences: BTreeMap<Source, Dependence>,
    followed: Vec<Followed>,
    params: HashMap<ParamId, Affine>,
    lets: HashMap<LetId, Affine>,
    sizes: HashMap<(PartId, PartAxis), Affine>,
    mins: HashMap<(PartId, Axis), Affine>,
    regions: HashMap<RegionId, Bounds>,
    busy: HashSet<Busy>,
}

impl<'s, 'm> Tracer<'s, 'm> {
    fn new(solver: &'s mut Solver<'m>, quantity: Quantity) -> Result<Self, TraceError> {
        let model = solver.model;
        let instance = match quantity {
            Quantity::PartSize { part, .. } | Quantity::PlaceOffset { part, .. } => {
                model.parts.get(part.index()).map(|p| p.ctx.instance)
            }
            Quantity::RegionSize { region, .. } | Quantity::Share { region } => {
                model.regions.get(region.index()).map(|r| r.ctx.instance)
            }
            Quantity::Param(param) => model.params.get(param.index()).map(|p| p.instance),
        }
        .ok_or(TraceError::UnknownItem)?;
        let mut tracer = Tracer::at(solver, instance);
        tracer.quantity = Some(quantity);
        Ok(tracer)
    }

    /// A tracer for expressions evaluated in `instance`, tracing no
    /// quantity of its own.
    fn at(solver: &'s mut Solver<'m>, instance: InstanceId) -> Self {
        let depth = instance_depth(solver.model, instance);
        Tracer {
            solver,
            quantity: None,
            depth,
            dependences: BTreeMap::new(),
            followed: Vec::new(),
            params: HashMap::new(),
            lets: HashMap::new(),
            sizes: HashMap::new(),
            mins: HashMap::new(),
            regions: HashMap::new(),
            busy: HashSet::new(),
        }
    }

    fn run(mut self) -> Result<Trace, TraceError> {
        let quantity = self.quantity.ok_or(TraceError::UnknownItem)?;
        let (value, form) = self.quantity()?;
        let form = self.form_of(value, form)?;
        Ok(Trace {
            quantity,
            value,
            form,
            followed: self.followed,
        })
    }

    /// The public form of a value worked out as `form`, checked against
    /// its value `value` as the solver gives it.
    fn form_of(&self, value: Amount, form: Affine) -> Result<Form, TraceError> {
        let mut terms = Vec::new();
        for (source, coefficient) in &form.terms {
            let dependence = self
                .dependences
                .get(source)
                .cloned()
                .ok_or(TraceError::NotWorkedOut)?;
            terms.push(Term {
                dependence,
                coefficient: *coefficient,
            });
        }
        let form = if form.is_exact() {
            let linear = Linear {
                constant: value.with_raw(form.constant),
                terms,
            };
            let traced = linear.evaluate().ok_or(TraceError::NotWorkedOut)?;
            if traced != value {
                return Err(TraceError::Disagrees {
                    traced,
                    solved: value,
                });
            }
            Form::Linear(linear)
        } else {
            Form::NotLinear(NotLinear {
                reasons: form.reasons.into_iter().collect(),
                held: terms,
                dependences: form
                    .bent
                    .iter()
                    .filter_map(|k| self.dependences.get(k).cloned())
                    .collect(),
            })
        };
        Ok(form)
    }

    /// The quantity's value as the solver gives it, and its form.
    fn quantity(&mut self) -> Result<(Amount, Affine), TraceError> {
        let halt = |Halt| TraceError::NotWorkedOut;
        let amount = |value: &Value| Amount::from_value(value).ok_or(TraceError::NotAQuantity);
        match self.quantity.ok_or(TraceError::UnknownItem)? {
            Quantity::PartSize { part, axis } => {
                let value = self
                    .solver
                    .drawn_size(part, axis)
                    .map_err(|_| TraceError::NotWorkedOut)?;
                Ok((
                    Amount::Length(value),
                    self.part_size(part, axis).map_err(halt)?,
                ))
            }
            Quantity::RegionSize { region, axis } => {
                let value = self
                    .solver
                    .region_value(region)
                    .map_err(|_| TraceError::NotWorkedOut)?;
                let size = value.bounds.size(axis).ok_or(TraceError::NotWorkedOut)?;
                let form = self
                    .region(region)
                    .and_then(|b| b.size(axis))
                    .map_err(halt)?;
                Ok((Amount::Length(size), form))
            }
            Quantity::PlaceOffset { part, axis } => {
                // A mirror image has no place block of its own.
                if self.solver.part_body(part).is_none() {
                    return Err(TraceError::NoOffset);
                }
                let terms = self
                    .solver
                    .part_terms(part)
                    .map_err(|_| TraceError::NotWorkedOut)?;
                let env = self.solver.part_env(part).ok_or(TraceError::NotWorkedOut)?;
                let offset = match terms.axis(axis).map(|t| &t.kind) {
                    Some(AxisTermKind::Against { gap: offset, .. })
                    | Some(AxisTermKind::Flush { inset: offset, .. })
                    | Some(AxisTermKind::Corner { inset: offset, .. }) => *offset,
                    Some(
                        AxisTermKind::CenterOn { .. }
                        | AxisTermKind::CenterBetween { .. }
                        | AxisTermKind::Spaced { .. },
                    )
                    | None => return Err(TraceError::NoOffset),
                };
                match offset {
                    Some(expr) => {
                        let value = self.value(expr, &env).map_err(halt)?;
                        Ok((amount(&value)?, self.affine(expr, &env).map_err(halt)?))
                    }
                    None => Ok((Amount::Length(ExactLength::ZERO), Affine::zero())),
                }
            }
            Quantity::Param(param) => {
                let value = self
                    .solver
                    .param_value(param)
                    .map_err(|_| TraceError::NotWorkedOut)?;
                let value = amount(&value)?;
                Ok((value, self.param(param).map_err(halt)?))
            }
            Quantity::Share { region } => {
                let (expr, env) = self.share(region).map_err(|Halt| TraceError::NoShares)?;
                let value = self.value(&expr, &env).map_err(halt)?;
                Ok((amount(&value)?, self.affine(&expr, &env).map_err(halt)?))
            }
        }
    }

    /// The share written for a piece of a split.
    fn share(&mut self, region: RegionId) -> Step<(RExpr, Env<'static>)> {
        let model = self.solver.model;
        let inst = model.regions.get(region.index()).ok_or(Halt)?;
        let def = model.defs.region(inst.def).ok_or(Halt)?;
        let RExprKind::Builtin {
            function: Builtin::Split,
            args,
        } = &def.value.kind
        else {
            return Err(Halt);
        };
        let shares = arg(args, "shares").ok_or(Halt)?;
        let env = Env {
            ctx: inst.ctx,
            file: def.decl.file,
            kind: None,
            array: None,
        };
        let (shares, env) = self.peel(shares, env);
        let RExprKind::List(items) = &shares.kind else {
            return Err(Halt);
        };
        let item = items
            .get(inst.ctx.element.unwrap_or(0))
            .cloned()
            .ok_or(Halt)?;
        Ok((item, env))
    }

    fn value(&mut self, expr: &RExpr, env: &Env<'_>) -> Step<Value> {
        self.solver.eval(expr, env).map_err(|_| Halt)
    }

    /// The current value of an expression as a raw number, when it is a
    /// quantity the solver can work out.
    fn raw_value(&mut self, expr: &RExpr, env: &Env<'_>) -> Option<Rational> {
        let value = self.solver.eval(expr, env).ok()?;
        Amount::from_value(&value).map(Amount::raw)
    }

    fn levels(&self, instance: InstanceId) -> u32 {
        self.depth
            .saturating_sub(instance_depth(self.solver.model, instance))
    }

    /// Records a value the size depends on.
    fn depend(&mut self, source: Source, freedom: Freedom, levels: u32, value: Amount) -> Affine {
        self.dependences
            .entry(source.clone())
            .or_insert(Dependence {
                source: source.clone(),
                freedom,
                levels,
                value,
                domain: None,
            });
        Affine::source(source)
    }

    // -- names ---------------------------------------------------------------

    /// The expression a param's value is written as, and where.
    fn param_source(&self, param: ParamId) -> Option<(RExpr, Env<'static>)> {
        let model = self.solver.model;
        let mut current = param;
        for _ in 0..PEEL_LIMIT {
            let inst = model.params.get(current.index())?;
            let def = model.defs.param(inst.def)?;
            match inst.source {
                ParamSource::SameAs(other) => current = other,
                ParamSource::Default => {
                    let env = Env {
                        ctx: Ctx {
                            instance: inst.instance,
                            element: None,
                        },
                        file: def.decl.file,
                        kind: None,
                        array: None,
                    };
                    return Some((def.default.clone()?, env));
                }
                ParamSource::Argument {
                    assembly,
                    index,
                    ctx,
                } => {
                    let assembly_def = model.defs.assembly(assembly)?;
                    let AssemblyKind::Instance { args, .. } = &assembly_def.kind else {
                        return None;
                    };
                    let (_, expr) = args.get(index)?;
                    let id = DefId::Assembly(assembly);
                    let array = model
                        .defs
                        .array(id)
                        .map(|_| (id, self.solver.array_len(id, ctx)));
                    let env = Env {
                        ctx,
                        file: assembly_def.decl.file,
                        kind: None,
                        array,
                    };
                    return Some((expr.clone(), env));
                }
            }
        }
        None
    }

    /// Follows names (params and lets) to the expression they stand for.
    fn peel(&self, expr: &RExpr, env: Env<'static>) -> (RExpr, Env<'static>) {
        let model = self.solver.model;
        let mut current = (expr.clone(), env);
        for _ in 0..PEEL_LIMIT {
            let RExprKind::Symbol(Symbol::Def(def)) = &current.0.kind else {
                break;
            };
            let next = match model.item(*def, current.1.ctx) {
                Some(ItemSlot::Param(param)) => self.param_source(*param),
                Some(ItemSlot::Let(binding)) => model.lets.get(binding.index()).and_then(|inst| {
                    let def = model.defs.let_def(inst.def)?;
                    Some((
                        def.value.clone(),
                        Env {
                            ctx: inst.ctx,
                            file: def.decl.file,
                            kind: None,
                            array: None,
                        },
                    ))
                }),
                _ => None,
            };
            match next {
                Some(next) => current = next,
                None => break,
            }
        }
        current
    }

    fn param(&mut self, param: ParamId) -> Step<Affine> {
        if let Some(form) = self.params.get(&param) {
            return Ok(form.clone());
        }
        if !self.busy.insert(Busy::Param(param)) {
            return Ok(Affine::other("a value worked out from itself"));
        }
        let result = self.compute_param(param);
        self.busy.remove(&Busy::Param(param));
        if let Ok(form) = &result {
            self.params.insert(param, form.clone());
        }
        result
    }

    fn compute_param(&mut self, param: ParamId) -> Step<Affine> {
        let model = self.solver.model;
        let inst = model.params.get(param.index()).ok_or(Halt)?;
        if let ParamSource::SameAs(other) = inst.source {
            return self.param(other);
        }
        let def = model.defs.param(inst.def).ok_or(Halt)?;
        let levels = self.levels(inst.instance);
        let freedom = freedom_of(def);
        if freedom == Freedom::Locked {
            return self.stop_at(param, freedom, levels);
        }
        let (expr, env) = self.param_source(param).ok_or(Halt)?;
        let form = self.affine(&expr, &env)?;
        if form.as_constant().is_some() {
            return self.stop_at(param, freedom, levels);
        }
        self.followed.push(Followed::Param {
            id: param,
            freedom,
            levels,
        });
        Ok(form)
    }

    /// A param that stops the chain: written as a number, or locked.
    fn stop_at(&mut self, param: ParamId, freedom: Freedom, levels: u32) -> Step<Affine> {
        let model = self.solver.model;
        let value = self.solver.param_value(param).map_err(|_| Halt)?;
        let Some(amount) = Amount::from_value(&value) else {
            return Ok(Affine::other("a value that is not a number"));
        };
        let inst = model.params.get(param.index()).ok_or(Halt)?;
        let def = model.defs.param(inst.def).ok_or(Halt)?;
        let env = Env {
            ctx: Ctx {
                instance: inst.instance,
                element: None,
            },
            file: def.decl.file,
            kind: None,
            array: None,
        };
        let domain = match &def.domain {
            None => None,
            Some(Domain::Range { low, high, .. }) => {
                let low = self.value(low, &env)?;
                let high = self.value(high, &env)?;
                match (Amount::from_value(&low), Amount::from_value(&high)) {
                    (Some(low), Some(high)) => Some(ParamDomain::Range { low, high }),
                    _ => None,
                }
            }
            Some(Domain::OneOf { values, .. }) => {
                let mut list = Vec::new();
                for value in values {
                    let value = self.value(value, &env)?;
                    if let Some(amount) = Amount::from_value(&value) {
                        list.push(amount);
                    }
                }
                Some(ParamDomain::OneOf(list))
            }
        };
        let source = Source::Param(param);
        let form = self.depend(source.clone(), freedom, levels, amount);
        if let Some(entry) = self.dependences.get_mut(&source) {
            entry.domain = domain;
        }
        Ok(form)
    }

    fn let_form(&mut self, binding: LetId) -> Step<Affine> {
        if let Some(form) = self.lets.get(&binding) {
            return Ok(form.clone());
        }
        if !self.busy.insert(Busy::Let(binding)) {
            return Ok(Affine::other("a value worked out from itself"));
        }
        let model = self.solver.model;
        let result = (|| {
            let inst = model.lets.get(binding.index()).ok_or(Halt)?;
            let def = model.defs.let_def(inst.def).ok_or(Halt)?;
            let env = Env {
                ctx: inst.ctx,
                file: def.decl.file,
                kind: None,
                array: None,
            };
            let levels = self.levels(inst.ctx.instance);
            let form = self.affine(&def.value, &env)?;
            self.followed.push(Followed::Let {
                id: binding,
                levels,
            });
            Ok(form)
        })();
        self.busy.remove(&Busy::Let(binding));
        if let Ok(form) = &result {
            self.lets.insert(binding, form.clone());
        }
        result
    }

    fn def_form(&mut self, def: DefId, ctx: Ctx) -> Step<Affine> {
        match self.solver.model.item(def, ctx) {
            Some(ItemSlot::Param(param)) => {
                let param = *param;
                self.param(param)
            }
            Some(ItemSlot::Let(binding)) => {
                let binding = *binding;
                self.let_form(binding)
            }
            _ => Ok(Affine::other("a value that is not a number")),
        }
    }

    fn stock_field(&mut self, stock: StockId, field: StockField) -> Step<Affine> {
        let value = self
            .solver
            .stock_field(stock, field, Span::default())
            .map_err(|_| Halt)?;
        let Some(amount) = Amount::from_value(&value) else {
            return Ok(Affine::other("a stock field that is not a length"));
        };
        let levels = self.depth;
        Ok(self.depend(
            Source::StockField { stock, field },
            Freedom::StockField,
            levels,
            amount,
        ))
    }

    /// The setting a kind value was given for `name`, where it is written.
    fn kind_setting(
        &self,
        expr: &RExpr,
        env: Env<'static>,
        name: &str,
    ) -> Option<(RExpr, Env<'static>)> {
        let mut current = self.peel(expr, env);
        for _ in 0..PEEL_LIMIT {
            let RExprKind::KindValue { kind, settings } = &current.0.kind else {
                return None;
            };
            if let Some(setting) = settings.iter().find(|s| s.name == name) {
                return Some((setting.value.clone(), current.1));
            }
            let inner = (**kind).clone();
            current = self.peel(&inner, current.1);
        }
        None
    }

    // -- expressions ---------------------------------------------------------

    fn affine(&mut self, expr: &RExpr, env: &Env<'static>) -> Step<Affine> {
        match &expr.kind {
            RExprKind::Length(length) => Ok(Affine::length(*length)),
            RExprKind::Angle(angle) => Ok(Affine::constant(angle.value())),
            RExprKind::Number(number) => Ok(Affine::constant(*number)),
            RExprKind::Symbol(Symbol::Def(def)) => self.def_form(*def, env.ctx),
            RExprKind::Member { base, member } => self.member(base, member, env),
            RExprKind::Unary {
                op: UnaryOp::Neg,
                operand,
            } => self.affine(operand, env)?.scaled(Rational::from(-1_i64)),
            RExprKind::Binary { op, lhs, rhs } => {
                let (left, right) = (self.affine(lhs, env)?, self.affine(rhs, env)?);
                // A product or a quotient of two values that both change
                // needs their current values, to hold one while the other
                // changes.
                let varies = left.as_constant().is_none() && right.as_constant().is_none();
                match op {
                    BinaryOp::Add => left.plus(&right),
                    BinaryOp::Sub => left.minus(&right),
                    BinaryOp::Mul if varies => {
                        left.times(&right, (self.raw_value(lhs, env), self.raw_value(rhs, env)))
                    }
                    BinaryOp::Mul => left.times(&right, (None, None)),
                    BinaryOp::Div if varies => left.over(&right, self.raw_value(rhs, env)),
                    BinaryOp::Div => left.over(&right, None),
                    BinaryOp::Or
                    | BinaryOp::And
                    | BinaryOp::Eq
                    | BinaryOp::Ne
                    | BinaryOp::Lt
                    | BinaryOp::Le
                    | BinaryOp::Gt
                    | BinaryOp::Ge
                    | BinaryOp::In => Ok(Affine::other("a comparison")),
                }
            }
            RExprKind::Builtin {
                function: Builtin::Between,
                args,
            } => self.between(args, env),
            RExprKind::Builtin {
                function: Builtin::Extent,
                args,
            } => self.extent(args, env),
            RExprKind::Unary {
                op: UnaryOp::Not, ..
            }
            | RExprKind::Bool(_)
            | RExprKind::Text(_)
            | RExprKind::Fit
            | RExprKind::Word(_)
            | RExprKind::Symbol(_)
            | RExprKind::SelfElement(_)
            | RExprKind::Index { .. }
            | RExprKind::Builtin { .. }
            | RExprKind::KindValue { .. }
            | RExprKind::List(_)
            | RExprKind::Wall(_)
            | RExprKind::Error => Ok(Affine::other("a value that is not a number")),
        }
    }

    fn member(&mut self, base: &RExpr, member: &Member, env: &Env<'static>) -> Step<Affine> {
        match member {
            Member::Size(axis) => match self.value(base, env)? {
                Value::Part(part) => self.part_size(part, *axis),
                Value::Parts(parts) => self.parts_measured(&parts),
                _ => Err(Halt),
            },
            Member::Face(face) => match self.value(base, env)? {
                Value::Part(part) => self.part_face(part, *face),
                Value::Region(region) => self.region(region)?.face(*face),
                _ => Err(Halt),
            },
            Member::RegionSize(axis) => match self.value(base, env)? {
                Value::Region(region) => self.region(region)?.size(*axis),
                _ => Err(Halt),
            },
            Member::StockField(field) => match self.value(base, env)? {
                Value::Stock(stock) => self.stock_field(stock, *field),
                _ => Err(Halt),
            },
            Member::AssemblyItem(def) => match self.value(base, env)? {
                Value::Assembly(assembly) => {
                    let inner = self
                        .solver
                        .model
                        .assemblies
                        .get(assembly.index())
                        .map(|a| a.inner)
                        .ok_or(Halt)?;
                    self.def_form(
                        *def,
                        Ctx {
                            instance: inner,
                            element: None,
                        },
                    )
                }
                _ => Err(Halt),
            },
            Member::KindEntry(name) => self.kind_entry(base, name, env),
            Member::Diameter => Ok(Affine::other("a round part's diameter")),
            Member::PartStock => Ok(Affine::other("a stock")),
            Member::ProfileLength => Ok(Affine::other("a profile's length")),
            Member::PatchSize(_) => Ok(Affine::other("where two parts touch")),
        }
    }

    /// `base.name` where the type of `base` is known only now: a kind's
    /// param or field, or a member of a value from a param.
    fn kind_entry(&mut self, base: &RExpr, name: &str, env: &Env<'static>) -> Step<Affine> {
        let value = self.value(base, env)?;
        match &value {
            Value::Part(part) => {
                if let Some(axis) = PartAxis::from_word(name) {
                    return self.part_size(*part, axis);
                }
                if let Some(face) = Face::from_word(name) {
                    return self.part_face(*part, face);
                }
                Ok(Affine::other("a field of a part"))
            }
            Value::Parts(parts) if PartAxis::from_word(name).is_some() => {
                self.parts_measured(parts)
            }
            Value::Region(region) => {
                if let Some(face) = Face::from_word(name) {
                    return self.region(*region)?.face(face);
                }
                match Axis::from_word(name) {
                    Some(axis) => self.region(*region)?.size(axis),
                    None => Ok(Affine::other("a field of a region")),
                }
            }
            Value::Stock(stock) => match StockField::from_name(name) {
                Some(field) => self.stock_field(*stock, field),
                None => Ok(Affine::other("a field of a stock")),
            },
            Value::Kind(kind_value) => {
                if let Some((expr, setting_env)) = self.kind_setting(base, *env, name) {
                    return self.affine(&expr, &setting_env);
                }
                let entry = self
                    .solver
                    .kind_entry(kind_value, name, env.file, base.span)
                    .map_err(|_| Halt)?;
                let Some(amount) = Amount::from_value(&entry) else {
                    return Ok(Affine::other("a kind's field that is not a number"));
                };
                let levels = self.depth;
                Ok(self.depend(
                    Source::KindSize {
                        kind: kind_value.kind,
                        name: name.to_owned(),
                    },
                    Freedom::KindSize,
                    levels,
                    amount,
                ))
            }
            _ => Ok(Affine::other("a field of a value that is not a number")),
        }
    }

    /// A face or mid-plane, as the position of its plane.
    fn anchor(&mut self, expr: &RExpr, env: &Env<'static>) -> Step<Affine> {
        let (expr, env) = self.peel(expr, *env);
        match &expr.kind {
            RExprKind::Member {
                base,
                member: Member::Face(face),
            } => self.member(base, &Member::Face(*face), &env),
            RExprKind::Member {
                base,
                member: Member::KindEntry(name),
            } => self.kind_entry(base, name, &env),
            RExprKind::Builtin {
                function: Builtin::Mid,
                args,
            } => {
                let (Some(item), Some(axis)) = (arg(args, "item"), arg(args, "axis")) else {
                    return Err(Halt);
                };
                let Value::Word(Word::Axis(axis)) = self.value(axis, &env)? else {
                    return Err(Halt);
                };
                let (low, high) = self.item_extent(item, axis, &env)?;
                low.plus(&high)?.half()
            }
            _ => Ok(Affine::other("a face")),
        }
    }

    /// The low and high ends of a part or a region along an axis.
    fn item_extent(
        &mut self,
        item: &RExpr,
        axis: Axis,
        env: &Env<'static>,
    ) -> Step<(Affine, Affine)> {
        match self.value(item, env)? {
            Value::Part(part) => {
                let low = self.part_min(part, axis)?;
                let high = low.plus(&self.size_along(part, axis)?)?;
                Ok((low, high))
            }
            Value::Region(region) => {
                let bounds = self.region(region)?;
                Ok((bounds.low(axis)?, bounds.high(axis)?))
            }
            _ => Err(Halt),
        }
    }

    fn part_box(&mut self, part: PartId) -> Step<Vec<Affine>> {
        let mut ends = Vec::new();
        for axis in Axis::ALL {
            let low = self.part_min(part, axis)?;
            let high = low.plus(&self.size_along(part, axis)?)?;
            ends.push(low);
            ends.push(high);
        }
        Ok(ends)
    }

    /// The box around several parts: its size is the farthest end less the
    /// nearest, which has no linear form.
    fn parts_measured(&mut self, parts: &[PartId]) -> Step<Affine> {
        let mut ends = Vec::new();
        for part in parts {
            ends.extend(self.part_box(*part)?);
        }
        let refs: Vec<&Affine> = ends.iter().collect();
        Ok(Affine::not_linear(Reason::Measured, &refs))
    }

    fn between(&mut self, args: &[Setting], env: &Env<'static>) -> Step<Affine> {
        let (Some(a), Some(b)) = (arg(args, "a"), arg(args, "b")) else {
            return Err(Halt);
        };
        let mut ends = Vec::new();
        for side in [a, b] {
            match self.value(side, env)? {
                Value::Part(part) => ends.extend(self.part_box(part)?),
                Value::Anchor(_) => ends.push(self.anchor(side, env)?),
                Value::Region(region) => {
                    let bounds = self.region(region)?;
                    for axis in Axis::ALL {
                        ends.push(bounds.low(axis)?);
                        ends.push(bounds.high(axis)?);
                    }
                }
                _ => return Err(Halt),
            }
        }
        let refs: Vec<&Affine> = ends.iter().collect();
        Ok(Affine::not_linear(Reason::Measured, &refs))
    }

    fn extent(&mut self, args: &[Setting], env: &Env<'static>) -> Step<Affine> {
        let Some(items) = arg(args, "items") else {
            return Err(Halt);
        };
        let parts = self.value(items, env)?.parts().ok_or(Halt)?;
        self.parts_measured(&parts)
    }

    // -- parts ---------------------------------------------------------------

    fn part_size(&mut self, part: PartId, axis: PartAxis) -> Step<Affine> {
        if let Some(form) = self.sizes.get(&(part, axis)) {
            return Ok(form.clone());
        }
        if !self.busy.insert(Busy::Size(part, axis)) {
            return Ok(Affine::other("a size worked out from itself"));
        }
        let result = self.compute_part_size(part, axis);
        self.busy.remove(&Busy::Size(part, axis));
        if let Ok(form) = &result {
            self.sizes.insert((part, axis), form.clone());
        }
        result
    }

    fn compute_part_size(&mut self, part: PartId, axis: PartAxis) -> Step<Affine> {
        if self.solver.boards.contains_key(&part) {
            return Ok(Affine::other("one board of a glued-up part"));
        }
        if let Some((source, _, _)) = self.solver.mirror_source(part).map_err(|_| Halt)? {
            return self.part_size(source, axis);
        }
        let body = self.solver.part_body(part).ok_or(Halt)?;
        let env = self.solver.part_env(part).ok_or(Halt)?;
        if self.solver.part_profile(part).map_err(|_| Halt)?.is_some() {
            return Ok(Affine::other("a turned part's profile"));
        }
        let stock_field = |this: &mut Self, field: StockField| -> Step<Affine> {
            let stock = this.solver.part_stock(part).map_err(|_| Halt)?;
            this.stock_field(stock, field)
        };
        match axis {
            PartAxis::Length => match &body.length {
                Size::Expr(expr) => self.affine(expr, &env),
                Size::Fit(_) => self.fit(part, axis),
            },
            PartAxis::Width => match &body.width {
                Some(Size::Expr(expr)) => self.affine(expr, &env),
                Some(Size::Fit(_)) => self.fit(part, axis),
                None => stock_field(self, StockField::Width),
            },
            PartAxis::Thickness => match &body.thickness {
                Some(expr) => self.affine(expr, &env),
                None => stock_field(self, StockField::Thickness),
            },
        }
    }

    fn size_along(&mut self, part: PartId, axis: Axis) -> Step<Affine> {
        let orientation = self.solver.part_orientation(part).map_err(|_| Halt)?;
        self.part_size(part, orientation.part_axis(axis))
    }

    fn part_face(&mut self, part: PartId, face: Face) -> Step<Affine> {
        let axis = face.axis();
        let low = self.part_min(part, axis)?;
        if face.is_positive() {
            low.plus(&self.size_along(part, axis)?)
        } else {
            Ok(low)
        }
    }

    fn part_min(&mut self, part: PartId, axis: Axis) -> Step<Affine> {
        if let Some(form) = self.mins.get(&(part, axis)) {
            return Ok(form.clone());
        }
        if !self.busy.insert(Busy::Min(part, axis)) {
            return Ok(Affine::other("a position worked out from itself"));
        }
        let result = self.compute_part_min(part, axis);
        self.busy.remove(&Busy::Min(part, axis));
        if let Ok(form) = &result {
            self.mins.insert((part, axis), form.clone());
        }
        result
    }

    fn compute_part_min(&mut self, part: PartId, axis: Axis) -> Step<Affine> {
        if self.solver.boards.contains_key(&part) {
            return Ok(Affine::other("one board of a glued-up part"));
        }
        if let Some((source, plane_axis, _)) = self.solver.mirror_source(part).map_err(|_| Halt)? {
            let low = self.part_min(source, axis)?;
            if axis != plane_axis {
                return Ok(low);
            }
            let high = low.plus(&self.size_along(source, axis)?)?;
            let plane = self.part_mirror_plane(part)?;
            // Reflected: the new low end is 2p - high.
            return plane.scaled(Rational::from(2_i64))?.minus(&high);
        }
        let terms = self.solver.part_terms(part).map_err(|_| Halt)?;
        if terms.ground {
            return Ok(Affine::zero());
        }
        let env = self.solver.part_env(part).ok_or(Halt)?;
        let term = terms.axis(axis).copied().ok_or(Halt)?;
        let size = self.size_along(part, axis)?;
        self.term_low(&term, axis, &size, &env, part)
    }

    /// The plane a mirrored part is reflected through.
    fn part_mirror_plane(&mut self, part: PartId) -> Step<Affine> {
        let model = self.solver.model;
        let inst = model.parts.get(part.index()).ok_or(Halt)?;
        if inst.mirror_of.is_some() {
            return self.assembly_mirror_plane(inst.assembly.ok_or(Halt)?);
        }
        let def = model.defs.part(inst.def).ok_or(Halt)?;
        let PartKind::Mirror(mirror) = &def.kind else {
            return Err(Halt);
        };
        let env = self.solver.part_env(part).ok_or(Halt)?;
        self.anchor(&mirror.about, &env)
    }

    /// The plane a mirrored assembly is reflected through, walking up to
    /// the assembly declared with `mirror(...)`.
    fn assembly_mirror_plane(&mut self, assembly: AssemblyId) -> Step<Affine> {
        let model = self.solver.model;
        let mut current = assembly;
        for _ in 0..=model.assemblies.len() {
            let inst = model.assemblies.get(current.index()).ok_or(Halt)?;
            let def = model.defs.assembly(inst.def).ok_or(Halt)?;
            if let (Some(_), AssemblyKind::Mirror(mirror)) = (inst.mirror_of, &def.kind) {
                let env = Env {
                    ctx: inst.ctx,
                    file: def.decl.file,
                    kind: None,
                    array: None,
                };
                return self.anchor(&mirror.about, &env);
            }
            current = inst.parent.ok_or(Halt)?;
        }
        Err(Halt)
    }

    /// The face a term names, with its position and side.
    fn face_term(&mut self, face: &RExpr, env: &Env<'static>) -> Step<(Affine, Face)> {
        let anchor = self.solver.eval_anchor(face, env).map_err(|_| Halt)?;
        let side = anchor.face.ok_or(Halt)?;
        Ok((self.anchor(face, env)?, side))
    }

    fn optional(&mut self, expr: Option<&RExpr>, env: &Env<'static>) -> Step<Affine> {
        match expr {
            Some(expr) => self.affine(expr, env),
            None => Ok(Affine::zero()),
        }
    }

    /// For `against`, `flush` and corners: the end of the part the term
    /// puts in place, and whether it is the low end.
    fn known_end(&mut self, term: &AxisTerm, env: &Env<'static>) -> Step<(Affine, bool)> {
        match &term.kind {
            AxisTermKind::Against { face, gap } => {
                let (position, side) = self.face_term(face, env)?;
                let gap = self.optional(*gap, env)?;
                if side.is_positive() {
                    Ok((position.plus(&gap)?, true))
                } else {
                    Ok((position.minus(&gap)?, false))
                }
            }
            AxisTermKind::Flush { face, inset } => {
                let (position, side) = self.face_term(face, env)?;
                let inset = self.optional(*inset, env)?;
                if side.is_positive() {
                    Ok((position.minus(&inset)?, false))
                } else {
                    Ok((position.plus(&inset)?, true))
                }
            }
            AxisTermKind::Corner { part, inset, face } => {
                let other = self.solver.eval_part(part, env).map_err(|_| Halt)?;
                let position = self.part_face(other, *face)?;
                let inset = self.optional(*inset, env)?;
                if face.is_positive() {
                    Ok((position.minus(&inset)?, false))
                } else {
                    Ok((position.plus(&inset)?, true))
                }
            }
            AxisTermKind::CenterOn { .. }
            | AxisTermKind::CenterBetween { .. }
            | AxisTermKind::Spaced { .. } => Err(Halt),
        }
    }

    /// The centre a `center` term puts a part at.
    fn centre(&mut self, term: &AxisTerm, axis: Axis, env: &Env<'static>) -> Step<Affine> {
        match &term.kind {
            AxisTermKind::CenterOn { item } => {
                let (low, high) = self.item_extent(item, axis, env)?;
                low.plus(&high)?.half()
            }
            AxisTermKind::CenterBetween { from, to } => {
                let from = self.anchor(from, env)?;
                let to = self.anchor(to, env)?;
                from.plus(&to)?.half()
            }
            AxisTermKind::Against { .. }
            | AxisTermKind::Flush { .. }
            | AxisTermKind::Corner { .. }
            | AxisTermKind::Spaced { .. } => Err(Halt),
        }
    }

    fn term_low(
        &mut self,
        term: &AxisTerm,
        axis: Axis,
        size: &Affine,
        env: &Env<'static>,
        part: PartId,
    ) -> Step<Affine> {
        match &term.kind {
            AxisTermKind::Against { .. }
            | AxisTermKind::Flush { .. }
            | AxisTermKind::Corner { .. } => {
                let (known, is_low) = self.known_end(term, env)?;
                if is_low { Ok(known) } else { known.minus(size) }
            }
            AxisTermKind::CenterOn { .. } | AxisTermKind::CenterBetween { .. } => {
                self.centre(term, axis, env)?.minus(&size.half()?)
            }
            AxisTermKind::Spaced { spacing, from, to } => {
                self.spaced(part, axis, size, spacing, from, to, env)
            }
        }
    }

    /// The distance a spacing is written with, as a form.
    fn spacing(&mut self, spacing: &RExpr, env: &Env<'static>) -> Step<(Spacing, Affine)> {
        let value = match self.value(spacing, env)? {
            Value::Spacing(spacing) => spacing,
            Value::Word(Word::Evenly) => Spacing::Evenly,
            Value::Word(Word::Ends) => Spacing::Ends(ExactLength::ZERO),
            _ => return Err(Halt),
        };
        let (written, written_env) = self.peel(spacing, *env);
        let name = match value {
            Spacing::Evenly => return Ok((value, Affine::zero())),
            Spacing::Ends(_) => "inset",
            Spacing::Pitch(_) | Spacing::Gap(_) => "distance",
        };
        let distance = match &written.kind {
            RExprKind::Builtin { args, .. } => match arg(args, name) {
                Some(expr) => self.affine(expr, &written_env)?,
                None => Affine::zero(),
            },
            RExprKind::Word(_) => Affine::zero(),
            _ => Affine::other("a spacing"),
        };
        Ok((value, distance))
    }

    #[allow(clippy::too_many_arguments)]
    fn spaced(
        &mut self,
        part: PartId,
        axis: Axis,
        size: &Affine,
        spacing: &RExpr,
        from: &RExpr,
        to: &RExpr,
        env: &Env<'static>,
    ) -> Step<Affine> {
        let (kind, distance) = self.spacing(spacing, env)?;
        let a_value = self
            .solver
            .eval_anchor(from, env)
            .map_err(|_| Halt)?
            .position;
        let b_value = self.solver.eval_anchor(to, env).map_err(|_| Halt)?.position;
        let forward = b_value >= a_value;
        let a = self.anchor(from, env)?;
        let b = self.anchor(to, env)?;
        let span = if forward { b.minus(&a)? } else { a.minus(&b)? };
        let model = self.solver.model;
        let inst = model.parts.get(part.index()).ok_or(Halt)?;
        let index = inst.ctx.element.unwrap_or(0);
        let siblings = match model.item(DefId::Part(inst.def), inst.ctx) {
            Some(ItemSlot::Part(slot)) => slot.all(),
            _ => vec![part],
        };
        let mut sizes = Vec::new();
        for sibling in siblings {
            sizes.push(self.size_along(sibling, axis)?);
        }
        let n = sizes.len();
        let mut total = Affine::zero();
        let mut before = Affine::zero();
        for (i, sibling) in sizes.iter().enumerate() {
            total = total.plus(sibling)?;
            if i < index {
                before = before.plus(sibling)?;
            }
        }
        let own = sizes.get(index).cloned().unwrap_or_else(|| size.clone());
        let offset = match kind {
            Spacing::Evenly => {
                let gap = span.minus(&total)?.scaled(
                    Rational::ONE
                        .checked_div(Affine::whole(n + 1)?)
                        .ok_or(Halt)?,
                )?;
                gap.scaled(Affine::whole(index + 1)?)?.plus(&before)?
            }
            Spacing::Ends(_) => {
                if n <= 1 {
                    distance
                } else {
                    let free = span
                        .minus(&distance.scaled(Rational::from(2_i64))?)?
                        .minus(&total)?;
                    let gap = free.scaled(
                        Rational::ONE
                            .checked_div(Affine::whole(n - 1)?)
                            .ok_or(Halt)?,
                    )?;
                    distance
                        .plus(&gap.scaled(Affine::whole(index)?)?)?
                        .plus(&before)?
                }
            }
            Spacing::Pitch(_) => {
                let first = sizes.first().cloned().unwrap_or_else(|| size.clone());
                let centre = first
                    .half()?
                    .plus(&distance.scaled(Affine::whole(index)?)?)?;
                centre.minus(&own.half()?)?
            }
            Spacing::Gap(_) => distance.scaled(Affine::whole(index)?)?.plus(&before)?,
        };
        if forward {
            a.plus(&offset)
        } else {
            a.minus(&offset)?.minus(&own)
        }
    }

    /// The parts a part is joined into, as the first role of a joint.
    fn joined_into(&mut self, part: PartId) -> Vec<PartId> {
        let mut partners = Vec::new();
        let model = self.solver.model;
        for (index, joint) in model.joints.iter().enumerate() {
            if joint.mirror_of.is_some() {
                continue;
            }
            let Ok(binding) = self
                .solver
                .joint_binding(stave_model::ids::JointId::from_usize(index))
            else {
                continue;
            };
            let Some(first) = binding.roles.first() else {
                continue;
            };
            if first.parts().is_some_and(|parts| parts.contains(&part)) {
                for other in binding.roles.iter().skip(1) {
                    for candidate in other.parts().unwrap_or_default() {
                        if candidate != part && !partners.contains(&candidate) {
                            partners.push(candidate);
                        }
                    }
                }
            }
        }
        partners
    }

    /// A `fit` size, decided by the parts it is joined into (grammar,
    /// "Sizes"). It is linear when exactly one part could end it on each
    /// side it is measured to; when several could, which one is nearest
    /// decides it, and that has no linear form.
    fn fit(&mut self, part: PartId, axis: PartAxis) -> Step<Affine> {
        let env = self.solver.part_env(part).ok_or(Halt)?;
        let orientation = self.solver.part_orientation(part).map_err(|_| Halt)?;
        let world = orientation.direction(axis).axis();
        let terms = self.solver.part_terms(part).map_err(|_| Halt)?;
        let term = terms.axis(world).copied().ok_or(Halt)?;
        // Each partner's ends: the solver's values, to hold its choices,
        // and the forms.
        let mut partners = Vec::new();
        for partner in self.joined_into(part) {
            let low_value = self.solver.part_min(partner, world).map_err(|_| Halt)?;
            let size_value = self.solver.size_along(partner, world).map_err(|_| Halt)?;
            let high_value = low_value.checked_add(size_value).ok_or(Halt)?;
            let low = self.part_min(partner, world)?;
            let high = low.plus(&self.size_along(partner, world)?)?;
            partners.push(((low_value, high_value), (low, high)));
        }
        let below = |at: ExactLength| -> Vec<&Affine> {
            partners
                .iter()
                .filter(|((_, high), _)| *high <= at)
                .map(|(_, (_, high))| high)
                .collect()
        };
        let above = |at: ExactLength| -> Vec<&Affine> {
            partners
                .iter()
                .filter(|((low, _), _)| *low >= at)
                .map(|(_, (low, _))| low)
                .collect()
        };
        match &term.kind {
            AxisTermKind::CenterOn { .. } | AxisTermKind::CenterBetween { .. } => {
                let centre_value = self.centre_value(&term, world, &env)?;
                let centre = self.centre(&term, world, &env)?;
                match (
                    below(centre_value).as_slice(),
                    above(centre_value).as_slice(),
                ) {
                    ([low], [high]) => high.minus(low),
                    (lows, highs) => {
                        let mut all: Vec<&Affine> = vec![&centre];
                        all.extend(lows.iter().copied());
                        all.extend(highs.iter().copied());
                        Ok(Affine::not_linear(Reason::Fit, &all))
                    }
                }
            }
            AxisTermKind::Against { .. }
            | AxisTermKind::Flush { .. }
            | AxisTermKind::Corner { .. } => {
                let (known, is_low) = self.known_end(&term, &env)?;
                let known_value = self.known_end_value(&term, &env)?;
                let candidates = if is_low {
                    above(known_value)
                } else {
                    below(known_value)
                };
                match candidates.as_slice() {
                    [end] if is_low => end.minus(&known),
                    [end] => known.minus(end),
                    several => {
                        let mut all: Vec<&Affine> = vec![&known];
                        all.extend(several.iter().copied());
                        Ok(Affine::not_linear(Reason::Fit, &all))
                    }
                }
            }
            AxisTermKind::Spaced { .. } => Err(Halt),
        }
    }

    /// The solver's value of the centre a `center` term puts a part at.
    fn centre_value(
        &mut self,
        term: &AxisTerm,
        axis: Axis,
        env: &Env<'static>,
    ) -> Step<ExactLength> {
        let (low, high) = match &term.kind {
            AxisTermKind::CenterOn { item } => {
                let bounds = match self.value(item, env)? {
                    Value::Part(part) => self.solver.drawn_box(part).map_err(|_| Halt)?,
                    Value::Region(region) => {
                        self.solver.region_value(region).map_err(|_| Halt)?.bounds
                    }
                    _ => return Err(Halt),
                };
                (bounds.min(axis), bounds.max(axis))
            }
            AxisTermKind::CenterBetween { from, to } => (
                self.solver
                    .eval_anchor(from, env)
                    .map_err(|_| Halt)?
                    .position,
                self.solver.eval_anchor(to, env).map_err(|_| Halt)?.position,
            ),
            AxisTermKind::Against { .. }
            | AxisTermKind::Flush { .. }
            | AxisTermKind::Corner { .. }
            | AxisTermKind::Spaced { .. } => return Err(Halt),
        };
        crate::eval::midpoint(low, high).ok_or(Halt)
    }

    /// The solver's value of the end [`Self::known_end`] gives.
    fn known_end_value(&mut self, term: &AxisTerm, env: &Env<'static>) -> Step<ExactLength> {
        let length = |this: &mut Self, expr: Option<&RExpr>| -> Step<ExactLength> {
            match expr {
                Some(expr) => this.solver.eval_length(expr, env).map_err(|_| Halt),
                None => Ok(ExactLength::ZERO),
            }
        };
        let (position, offset, positive) = match &term.kind {
            AxisTermKind::Against { face, gap } => {
                let anchor = self.solver.eval_anchor(face, env).map_err(|_| Halt)?;
                let positive = anchor.face.ok_or(Halt)?.is_positive();
                let gap = length(self, *gap)?;
                // A gap moves away from the face: up from a top face.
                let offset = if positive {
                    gap
                } else {
                    gap.checked_neg().ok_or(Halt)?
                };
                return anchor.position.checked_add(offset).ok_or(Halt);
            }
            AxisTermKind::Flush { face, inset } => {
                let anchor = self.solver.eval_anchor(face, env).map_err(|_| Halt)?;
                let positive = anchor.face.ok_or(Halt)?.is_positive();
                (anchor.position, length(self, *inset)?, positive)
            }
            AxisTermKind::Corner { part, inset, face } => {
                let other = self.solver.eval_part(part, env).map_err(|_| Halt)?;
                let position = self
                    .solver
                    .part_anchor(other, *face)
                    .map_err(|_| Halt)?
                    .position;
                (position, length(self, *inset)?, face.is_positive())
            }
            AxisTermKind::CenterOn { .. }
            | AxisTermKind::CenterBetween { .. }
            | AxisTermKind::Spaced { .. } => return Err(Halt),
        };
        // An inset moves into the face's part: down from a top face.
        if positive {
            position.checked_sub(offset).ok_or(Halt)
        } else {
            position.checked_add(offset).ok_or(Halt)
        }
    }

    // -- regions -------------------------------------------------------------

    fn region(&mut self, region: RegionId) -> Step<Bounds> {
        if let Some(bounds) = self.regions.get(&region) {
            return Ok(bounds.clone());
        }
        if !self.busy.insert(Busy::Region(region)) {
            return Ok(Bounds::other("a region worked out from itself"));
        }
        let result = self.compute_region(region);
        self.busy.remove(&Busy::Region(region));
        if let Ok(bounds) = &result {
            self.regions.insert(region, bounds.clone());
        }
        result
    }

    fn compute_region(&mut self, region: RegionId) -> Step<Bounds> {
        let model = self.solver.model;
        let inst = model.regions.get(region.index()).ok_or(Halt)?;
        let def = model.defs.region(inst.def).ok_or(Halt)?;
        if let Some(source) = inst.mirror_of {
            let assembly = inst.assembly.ok_or(Halt)?;
            let (axis, _) = self
                .solver
                .assembly_mirror_plane(assembly)
                .map_err(|_| Halt)?;
            let plane = self.assembly_mirror_plane(assembly)?;
            let bounds = self.region(source)?;
            let twice = plane.scaled(Rational::from(2_i64))?;
            let low = twice.minus(&bounds.high(axis)?)?;
            let high = twice.minus(&bounds.low(axis)?)?;
            return Ok(bounds.with(axis, low, high));
        }
        let id = DefId::Region(inst.def);
        let array = def
            .array
            .as_ref()
            .map(|_| (id, self.solver.array_len(id, inst.ctx)));
        let env = Env {
            ctx: inst.ctx,
            file: def.decl.file,
            kind: None,
            array,
        };
        let mut pieces = self.regions_of(&def.value, &env)?;
        let index = inst.ctx.element.unwrap_or(0);
        if index < pieces.len() {
            Ok(pieces.swap_remove(index))
        } else {
            Err(Halt)
        }
    }

    /// The regions an expression makes or names.
    fn regions_of(&mut self, expr: &RExpr, env: &Env<'static>) -> Step<Vec<Bounds>> {
        let (expr, env) = self.peel(expr, *env);
        let RExprKind::Builtin { function, args } = &expr.kind else {
            return match self.value(&expr, &env)? {
                Value::Region(region) => Ok(vec![self.region(region)?]),
                _ => Ok(vec![Bounds::other("a region")]),
            };
        };
        match function {
            Builtin::Opening => {
                let mut planes = Vec::new();
                for name in ["left", "right", "bottom", "top", "front", "back"] {
                    let plane = arg(args, name).ok_or(Halt)?;
                    planes.push(self.anchor(plane, &env)?);
                }
                let mut planes = planes.into_iter();
                let mut next = || planes.next().ok_or(Halt);
                Ok(vec![Bounds::new(
                    (next()?, next()?),
                    (next()?, next()?),
                    (next()?, next()?),
                )])
            }
            Builtin::Split => self.split(args, &env),
            Builtin::Cover => self.cover(args, &env),
            // The root box: from the origin, as wide, tall and deep as
            // its three lengths.
            _ if ["across", "up", "deep"]
                .iter()
                .all(|name| arg(args, name).is_some()) =>
            {
                let mut ends = Vec::new();
                for name in ["across", "up", "deep"] {
                    let size = arg(args, name).ok_or(Halt)?;
                    ends.push((Affine::zero(), self.affine(size, &env)?));
                }
                let mut ends = ends.into_iter();
                let mut next = || ends.next().ok_or(Halt);
                Ok(vec![Bounds::new(next()?, next()?, next()?)])
            }
            // Any other function makes something that is not a region.
            _ => Ok(vec![Bounds::other("a region")]),
        }
    }

    fn one_region(&mut self, expr: &RExpr, env: &Env<'static>) -> Step<Bounds> {
        let mut list = self.regions_of(expr, env)?;
        match list.len() {
            1 => list.pop().ok_or(Halt),
            _ => Err(Halt),
        }
    }

    /// Where the solver's region came from, for the meeting edges of a
    /// cover.
    fn region_origin(&mut self, expr: &RExpr, env: &Env<'static>) -> Step<RegionOrigin> {
        match self.value(expr, env)? {
            Value::Region(region) => Ok(self.solver.region_value(region).map_err(|_| Halt)?.origin),
            Value::Regions(list) => match list.as_slice() {
                [only] => Ok(only.origin),
                _ => Err(Halt),
            },
            _ => Err(Halt),
        }
    }

    fn split(&mut self, args: &[Setting], env: &Env<'static>) -> Step<Vec<Bounds>> {
        let (Some(region), Some(axis)) = (arg(args, "region"), arg(args, "axis")) else {
            return Err(Halt);
        };
        let bounds = self.one_region(region, env)?;
        let Value::Word(Word::Axis(axis)) = self.value(axis, env)? else {
            return Err(Halt);
        };
        let (low, high) = bounds.get(axis)?.clone();
        if let Some(at) = arg(args, "at") {
            let part = self.solver.eval_part(at, env).map_err(|_| Halt)?;
            let start = self.part_min(part, axis)?;
            let end = start.plus(&self.size_along(part, axis)?)?;
            return Ok(vec![
                bounds.with(axis, low, start),
                bounds.with(axis, end, high),
            ]);
        }
        let count = env.array.map(|(_, n)| n).filter(|n| *n > 0).ok_or(Halt)?;
        let shares = match arg(args, "shares") {
            Some(expr) => self.shares(expr, env, count)?,
            None => Ok(vec![Rational::ONE; count]),
        };
        let size = high.minus(&low)?;
        let shares = match shares {
            Ok(shares) => shares,
            Err(not_linear) => {
                let mark = Affine::joined(&[&not_linear, &low, &high]);
                return Ok((0..count)
                    .map(|_| bounds.with(axis, mark.clone(), mark.clone()))
                    .collect());
            }
        };
        let total = shares
            .iter()
            .try_fold(Rational::ZERO, |sum, share| sum.checked_add(*share))
            .ok_or(Halt)?;
        let mut pieces = Vec::new();
        let mut start = low.clone();
        let mut before = Rational::ZERO;
        for (index, share) in shares.iter().enumerate() {
            before = before.checked_add(*share).ok_or(Halt)?;
            let end = if index + 1 == count {
                high.clone()
            } else {
                low.plus(&size.scaled(before.checked_div(total).ok_or(Halt)?)?)?
            };
            pieces.push(bounds.with(axis, start, end.clone()));
            start = end;
        }
        Ok(pieces)
    }

    /// The shares of a split as numbers; `Err` with the form of a share
    /// that depends on a value (each piece is then that share over the
    /// sum, which has no linear form).
    fn shares(
        &mut self,
        expr: &RExpr,
        env: &Env<'static>,
        count: usize,
    ) -> Step<Result<Vec<Rational>, Affine>> {
        let (written, written_env) = self.peel(expr, *env);
        let RExprKind::List(items) = &written.kind else {
            return Ok(Err(Affine::other("shares that are not written out")));
        };
        if items.len() != count {
            return Err(Halt);
        }
        let mut shares = Vec::new();
        let mut forms = Vec::new();
        for item in items {
            let form = self.affine(item, &written_env)?;
            match form.as_constant() {
                Some(share) => shares.push(share),
                None => forms.push(form),
            }
        }
        if forms.is_empty() {
            return Ok(Ok(shares));
        }
        let refs: Vec<&Affine> = forms.iter().collect();
        Ok(Err(Affine::not_linear(Reason::Division, &refs)))
    }

    fn mount(&mut self, expr: &RExpr, env: &Env<'static>) -> Step<Option<MountForm>> {
        let (written, written_env) = self.peel(expr, *env);
        let RExprKind::Builtin { function, args } = &written.kind else {
            return Ok(None);
        };
        let length = |this: &mut Self, name: &str| -> Step<Option<Affine>> {
            arg(args, name)
                .map(|e| this.affine(e, &written_env))
                .transpose()
        };
        Ok(match function {
            Builtin::Overlay => Some(MountForm::Overlay(Box::new(Overlays {
                left: length(self, "left")?.unwrap_or_else(Affine::zero),
                right: length(self, "right")?.unwrap_or_else(Affine::zero),
                top: length(self, "top")?.unwrap_or_else(Affine::zero),
                bottom: length(self, "bottom")?.unwrap_or_else(Affine::zero),
                meeting: length(self, "meeting")?,
            }))),
            Builtin::Inset => length(self, "reveal")?.map(|reveal| MountForm::Inset { reveal }),
            // Any other function is not a mount.
            _ => None,
        })
    }

    fn cover(&mut self, args: &[Setting], env: &Env<'static>) -> Step<Vec<Bounds>> {
        let (Some(region), Some(mount), Some(thickness)) = (
            arg(args, "region"),
            arg(args, "mount"),
            arg(args, "thickness"),
        ) else {
            return Err(Halt);
        };
        let b = self.one_region(region, env)?;
        let origin = self.region_origin(region, env)?;
        let thickness = self.affine(thickness, env)?;
        let Some(mount) = self.mount(mount, env)? else {
            return Ok(vec![Bounds::other("a mount")]);
        };
        let front = b.low(Axis::Deep)?;
        let bounds = match mount {
            MountForm::Overlay(overlays) => {
                let Overlays {
                    left,
                    right,
                    top,
                    bottom,
                    meeting,
                } = *overlays;
                let shared = |axis: Axis, low_side: bool| match origin {
                    RegionOrigin::Split {
                        axis: split_axis,
                        index,
                        count,
                    } if split_axis == axis => {
                        if low_side {
                            index > 0
                        } else {
                            index + 1 < count
                        }
                    }
                    RegionOrigin::Split { .. } | RegionOrigin::Plain => false,
                };
                let pick = |value: Affine, axis: Axis, low_side: bool| match &meeting {
                    Some(m) if shared(axis, low_side) => m.clone(),
                    _ => value,
                };
                Bounds::new(
                    (
                        b.low(Axis::Across)?
                            .minus(&pick(left, Axis::Across, true))?,
                        b.high(Axis::Across)?
                            .plus(&pick(right, Axis::Across, false))?,
                    ),
                    (
                        b.low(Axis::Up)?.minus(&pick(bottom, Axis::Up, true))?,
                        b.high(Axis::Up)?.plus(&pick(top, Axis::Up, false))?,
                    ),
                    (front.minus(&thickness)?, front),
                )
            }
            MountForm::Inset { reveal } => Bounds::new(
                (
                    b.low(Axis::Across)?.plus(&reveal)?,
                    b.high(Axis::Across)?.minus(&reveal)?,
                ),
                (
                    b.low(Axis::Up)?.plus(&reveal)?,
                    b.high(Axis::Up)?.minus(&reveal)?,
                ),
                (front.clone(), front.plus(&thickness)?),
            ),
        };
        Ok(vec![bounds])
    }
}

/// What allows or stops a smart edit changing a param, from its
/// declaration's kind (`param`, `locked param`, `solved param`).
fn freedom_of(def: &stave_model::defs::ParamDef) -> Freedom {
    match def.kind {
        ParamKind::Plain => Freedom::Plain,
        ParamKind::Locked => Freedom::Locked,
        ParamKind::Solved => Freedom::Solved,
    }
}

/// How many component levels an instance is inside: 0 for a module.
fn instance_depth(model: &Model, instance: InstanceId) -> u32 {
    let mut depth = 0;
    let mut current = instance;
    for _ in 0..=model.instances.len() {
        match model.instance(current).map(|i| i.kind) {
            Some(InstanceKind::Component { assembly, .. }) => {
                depth += 1;
                match model.assemblies.get(assembly.index()) {
                    Some(inst) => current = inst.ctx.instance,
                    None => break,
                }
            }
            Some(InstanceKind::Module(_)) | None => break,
        }
    }
    depth
}
