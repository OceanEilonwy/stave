//! How far one plain number in the text can go each way before the design
//! stops checking (`docs/editing.md`, "Grips"): a grip's stops.
//!
//! ```text
//! path ──▶ the param it sets, its value and its `within`
//!      ──▶ each way: grow the step (fast tier) ─▶ bisect (fast tier) ─▶ confirm (slow tier)
//!      ──▶ ValueStops: the last good value each way, and what stops it
//! ```
//!
//! A value is addressed the way a patch's `set` addresses it: a project
//! param's name (`overall_width`), or an assembly and one of its
//! component's params (`divider.height`). Only a value written in the text
//! as a plain number (`500mm`, `-0.75mm`, `3`) can be searched: that is
//! what a drag changes.
//!
//! Each value tried is a patch (`set divider.height = 180mm`) applied to a
//! scratch copy of the prepared base ([`EditBase::start`]), so the base is
//! parsed and its libraries resolved once. A value is *good* when the edit
//! causes no error the base did not have (the strict commit rule of
//! [`crate::scratch`]). The search grows its step (1, 2, 4, ... steps of the
//! project's `resolution:`) with the fast check tier until a value is bad,
//! bisects back to the boundary with the fast tier, and then confirms the
//! last good value with the slow tier; only when the slow tier finds an
//! error there does it search again, with both tiers, inside that range. So
//! a search is tens of checks, not one per step.

use crate::i18n::{fill_in, tr};
use std::sync::atomic::{AtomicBool, Ordering};

use stave_lang::Diagnostic;
use stave_lang::units::{ExactAngle, ExactLength, Rational};
use stave_model::Model;
use stave_model::defs::{AssemblyKind, RExpr, RExprKind};
use stave_model::ids::{InstanceId, ParamId};
use stave_model::instance::{InstanceKind, ItemSlot, ParamSource};
use stave_model::trace::{Amount, Form, Freedom, ParamDomain, Quantity, Source};

use crate::edit::Verdict;
use crate::repeat::{GeneratedPatch, PatchBuildError};
use crate::scratch::EditBase;

// ---------------------------------------------------------------------------
// The request and the answer
// ---------------------------------------------------------------------------

/// The values a search may try, both ends included, used where the param
/// has no `within` domain: the caller's limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bounds {
    /// The lowest value to try.
    pub low: Amount,
    /// The highest value to try.
    pub high: Amount,
}

/// One search for the stops of a value.
#[derive(Debug, Clone, Copy)]
pub struct StopsRequest<'a> {
    /// The value, as a `set` path: `overall_width`, `divider.height`.
    pub path: &'a str,
    /// How far to look when the param has no `within`. A `within` domain
    /// always wins over it.
    pub limit: Bounds,
}

/// How far a value can go each way.
#[derive(Debug, Clone, PartialEq)]
pub struct ValueStops {
    /// The param the path sets (for an array of assemblies, the first
    /// element's: the argument is shared).
    pub param: ParamId,
    /// The path, as given.
    pub path: String,
    /// The value now.
    pub value: Amount,
    /// The values the search steps through.
    pub steps: SearchSteps,
    /// Going down.
    pub below: Side,
    /// Going up.
    pub above: Side,
    /// How many values were tried, each one check of a scratch copy.
    pub checks: usize,
}

/// The values a search steps through.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SearchSteps {
    /// Multiples of a step from the current value: the project's
    /// `resolution:` for a length, a tenth of a degree for an angle, one
    /// for a count, a hundredth for a ratio.
    Step(Amount),
    /// The values of a `within` list, in order.
    List(Vec<Amount>),
}

/// How far a value can go one way.
#[derive(Debug, Clone, PartialEq)]
pub struct Side {
    /// The last value that way at which the design still checks: the
    /// current value when even one step is too far.
    pub last_good: Amount,
    /// What ends the search there.
    pub end: SideEnd,
}

/// What ends a search one way.
#[derive(Debug, Clone, PartialEq)]
pub enum SideEnd {
    /// The next value causes an error the base did not have.
    Stopped {
        /// The first value tried that way that does not check.
        at: Amount,
        /// The first error it causes. Its spans point into the scratch
        /// copy that was checked, which is gone: show its code, message and
        /// help, not its source excerpt.
        diagnostic: Diagnostic,
    },
    /// The end of the param's `within` domain.
    Domain,
    /// The end of the caller's limit.
    Limit,
}

/// Why the stops of a value can't be searched.
#[derive(Debug, thiserror::Error)]
pub enum StopsError {
    /// The base design does not solve, so nothing can be checked against.
    #[error("{}", self.text())]
    NoDesign,
    /// The path names no param of the project or of a top-level assembly.
    #[error("{}", self.text())]
    UnknownPath(String),
    /// The value is not written as a plain number.
    #[error("{}", self.text())]
    NotANumber(String),
    /// The value can't be traced in the solved design.
    #[error("{}", self.text())]
    NotTraced {
        /// The path.
        path: String,
        /// Why.
        reason: String,
    },
    /// The limit is not the same kind of value, or runs high to low.
    #[error("{}", self.text())]
    BadLimit(String),
    /// A scratch copy could not be loaded.
    #[error(transparent)]
    Load(#[from] stave_modules::LoadError),
    /// A trial patch did not parse: a bug in the writer.
    #[error(transparent)]
    Patch(#[from] PatchBuildError),
    /// The caller cancelled the search.
    #[error("{}", self.text())]
    Cancelled,
    /// The numbers are too large.
    #[error("{}", self.text())]
    Overflow,
}

impl StopsError {
    /// The error in the window's words (translated).
    fn text(&self) -> String {
        match self {
            StopsError::NoDesign => {
                tr("the design doesn't place yet, so no value can be checked; fix its errors first")
            }
            StopsError::UnknownPath(v0) => fill_in(
                &tr("'{value}' is not a param of this project or of one of its assemblies"),
                &[("value", v0)],
            ),
            StopsError::NotANumber(v0) => fill_in(
                &tr("'{value}' is not written as a plain number, so there is nothing to move"),
                &[("value", v0)],
            ),
            StopsError::NotTraced { path, reason } => fill_in(
                &tr("the value of '{path}' can't be worked out: {reason}"),
                &[("path", path), ("reason", reason)],
            ),
            StopsError::BadLimit(v0) => fill_in(
                &tr(
                    "the limit must be the same kind of value as '{value}' and run from low to high",
                ),
                &[("value", v0)],
            ),
            StopsError::Load(error) => error.to_string(),
            StopsError::Patch(error) => error.to_string(),
            StopsError::Cancelled => tr("the search was cancelled"),
            StopsError::Overflow => tr("the numbers are too large"),
        }
    }
}

// ---------------------------------------------------------------------------
// The search
// ---------------------------------------------------------------------------

/// Finds how far the plain number at `request.path` can go each way
/// before the design stops checking.
///
/// From the current value it searches outward, one way and then the
/// other, through multiples of the project's `resolution:` (or through
/// the values of a `within` list), for the nearest value at which the edit
/// causes an error the base design did not have, and returns the last good
/// value each way with the diagnostic that stops it. A `within` range
/// bounds the search; where there is none, `request.limit` does.
///
/// It finds the **nearest stop each way, not every gap in between**: past
/// the first value that does not check it does not look for values that
/// check again, and because it grows its step before bisecting, a band of
/// bad values narrower than the step it jumped over on the way out can be
/// stepped over. Errors that come and go in the middle of the range are
/// for the check of each drag to find.
///
/// Setting `cancel` stops the search between checks and during the slow
/// tier.
///
/// # Errors
///
/// See [`StopsError`]: a design that does not place, a path that names no
/// param, a value that is not a plain number, a bad limit, cancellation.
pub fn find_stops(
    base: &EditBase<'_>,
    request: StopsRequest<'_>,
    cancel: &AtomicBool,
) -> Result<ValueStops, StopsError> {
    let opened = base.opened();
    let (Some(model), Some(design)) = (opened.model.as_ref(), opened.design.as_ref()) else {
        return Err(StopsError::NoDesign);
    };
    let path = request.path;
    let param =
        param_at_path(model, path).ok_or_else(|| StopsError::UnknownPath(path.to_owned()))?;
    let number = plain_number(model, param, path)?;
    let steps = match &number.domain {
        Some(ParamDomain::OneOf(values)) => {
            let mut values = values.clone();
            values.sort_by_key(|value| value.raw());
            values.dedup();
            SearchSteps::List(values)
        }
        Some(ParamDomain::Range { .. }) | None => {
            SearchSteps::Step(step_for(number.value, design.project.resolution.to_exact()))
        }
    };
    let (low, high, end) = match &number.domain {
        Some(ParamDomain::Range { low, high }) => (*low, *high, SideEnd::Domain),
        Some(ParamDomain::OneOf(_)) => (number.value, number.value, SideEnd::Domain),
        None => {
            let limit = request.limit;
            if !limit.low.same_dimension(number.value)
                || !limit.high.same_dimension(number.value)
                || limit.low.raw() > limit.high.raw()
            {
                return Err(StopsError::BadLimit(path.to_owned()));
            }
            (limit.low, limit.high, SideEnd::Limit)
        }
    };
    let mut search = Search {
        base,
        path,
        cancel,
        checks: 0,
    };
    let below = search.side(number.value, &steps, Direction::Down, low, end.clone())?;
    let above = search.side(number.value, &steps, Direction::Up, high, end)?;
    Ok(ValueStops {
        param,
        path: path.to_owned(),
        value: number.value,
        steps,
        below,
        above,
        checks: search.checks,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Direction {
    Down,
    Up,
}

/// What one value tried gave.
enum Probe {
    Good,
    Bad(Diagnostic),
}

struct Search<'b, 'f, 'p> {
    base: &'b EditBase<'f>,
    path: &'p str,
    cancel: &'p AtomicBool,
    checks: usize,
}

impl Search<'_, '_, '_> {
    /// The search one way from `value`, no farther than `bound`.
    fn side(
        &mut self,
        value: Amount,
        steps: &SearchSteps,
        direction: Direction,
        bound: Amount,
        end: SideEnd,
    ) -> Result<Side, StopsError> {
        let candidates = Candidates::new(value, steps, direction, bound)?;
        let count = candidates.count;
        if count == 0 {
            return Ok(Side {
                last_good: value,
                end,
            });
        }
        // The fast tier: grow the step, then bisect.
        let fast = self.boundary(&candidates, 0, count, Tier::Fast)?;
        // The slow tier at the last good value.
        let (good, bad) = if fast.good == 0 {
            (0, fast.bad)
        } else {
            match self.probe(candidates.at(fast.good)?, Tier::Full)? {
                Probe::Good => (fast.good, fast.bad),
                Probe::Bad(diagnostic) => {
                    // An error only the slow tier finds comes nearer: look
                    // again inside, with both tiers.
                    let inside = fast.good - 1;
                    let full = self.boundary(&candidates, 0, inside, Tier::Full)?;
                    match full.bad {
                        Some(found) => (full.good, Some(found)),
                        None => (inside, Some((fast.good, diagnostic))),
                    }
                }
            }
        };
        let last_good = if good == 0 {
            value
        } else {
            candidates.at(good)?
        };
        let end = match bad {
            Some((index, diagnostic)) => SideEnd::Stopped {
                at: candidates.at(index)?,
                diagnostic,
            },
            None => end,
        };
        Ok(Side { last_good, end })
    }

    /// The boundary between good and bad among candidates `1..=last`,
    /// index 0 (the current value, or a value known good) being good:
    /// grows the step until a candidate is bad or `last` is reached, then
    /// bisects. `None` for `bad` when every candidate tried is good.
    fn boundary(
        &mut self,
        candidates: &Candidates,
        first_good: usize,
        last: usize,
        tier: Tier,
    ) -> Result<Boundary, StopsError> {
        let mut good = first_good;
        let mut stride = 1usize;
        let mut bad: Option<(usize, Diagnostic)> = None;
        while good < last {
            let next = good.saturating_add(stride).min(last);
            match self.probe(candidates.at(next)?, tier)? {
                Probe::Good => {
                    good = next;
                    stride = stride.saturating_mul(2);
                }
                Probe::Bad(diagnostic) => {
                    bad = Some((next, diagnostic));
                    break;
                }
            }
        }
        let Some((mut high, mut diagnostic)) = bad else {
            return Ok(Boundary { good, bad: None });
        };
        while high - good > 1 {
            let middle = good + (high - good) / 2;
            match self.probe(candidates.at(middle)?, tier)? {
                Probe::Good => good = middle,
                Probe::Bad(found) => {
                    high = middle;
                    diagnostic = found;
                }
            }
        }
        Ok(Boundary {
            good,
            bad: Some((high, diagnostic)),
        })
    }

    /// Tries one value.
    fn probe(&mut self, value: Amount, tier: Tier) -> Result<Probe, StopsError> {
        if self.cancel.load(Ordering::Relaxed) {
            return Err(StopsError::Cancelled);
        }
        self.checks = self.checks.saturating_add(1);
        // Trial values are the written number plus whole steps (a
        // hundredth for a ratio, one for a count), so they always have
        // plain-number text unless they overflow.
        let text = amount_text(value).ok_or(StopsError::Overflow)?;
        let patch = GeneratedPatch::new(
            &format!("Try {} = {text}", self.path),
            &[format!("set {} = {text}", self.path)],
        )?;
        let scratch = self.base.start("stops.stave", &patch.parse)?;
        let verdict = match tier {
            Tier::Fast => scratch.verdict(),
            Tier::Full => scratch
                .finish(self.cancel)
                .map_err(|_| StopsError::Cancelled)?
                .verdict()
                .clone(),
        };
        Ok(match verdict {
            Verdict::Refused(refusal) => {
                Probe::Bad(refusal.problems().first().diagnostic().clone())
            }
            Verdict::Fits(_) | Verdict::Checking => Probe::Good,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tier {
    /// Layers 1 to 3 and box overlap.
    Fast,
    /// Both tiers.
    Full,
}

struct Boundary {
    /// The last good index.
    good: usize,
    /// The first bad index and what it causes.
    bad: Option<(usize, Diagnostic)>,
}

/// The values one way from the current value, by index from 1 (index 0 is
/// the current value).
struct Candidates {
    kind: CandidateKind,
    count: usize,
}

enum CandidateKind {
    Step {
        value: Amount,
        /// The step, signed for the direction.
        step: Rational,
    },
    List(Vec<Amount>),
}

impl Candidates {
    fn new(
        value: Amount,
        steps: &SearchSteps,
        direction: Direction,
        bound: Amount,
    ) -> Result<Self, StopsError> {
        match steps {
            SearchSteps::Step(step) => {
                let room = match direction {
                    Direction::Down => value.raw().checked_sub(bound.raw()),
                    Direction::Up => bound.raw().checked_sub(value.raw()),
                }
                .ok_or(StopsError::Overflow)?;
                let count = if room.is_negative() {
                    0
                } else {
                    let whole = room.checked_div(step.raw()).ok_or(StopsError::Overflow)?;
                    usize::try_from(whole.floor()).map_err(|_| StopsError::Overflow)?
                };
                let step = match direction {
                    Direction::Down => step.raw().checked_neg().ok_or(StopsError::Overflow)?,
                    Direction::Up => step.raw(),
                };
                Ok(Self {
                    kind: CandidateKind::Step { value, step },
                    count,
                })
            }
            SearchSteps::List(values) => {
                let mut list: Vec<Amount> = match direction {
                    Direction::Down => values
                        .iter()
                        .copied()
                        .filter(|v| v.raw() < value.raw())
                        .collect(),
                    Direction::Up => values
                        .iter()
                        .copied()
                        .filter(|v| v.raw() > value.raw())
                        .collect(),
                };
                if direction == Direction::Down {
                    list.reverse();
                }
                Ok(Self {
                    count: list.len(),
                    kind: CandidateKind::List(list),
                })
            }
        }
    }

    /// The candidate at `index` (from 1).
    fn at(&self, index: usize) -> Result<Amount, StopsError> {
        match &self.kind {
            CandidateKind::Step { value, step } => {
                let times = i128::try_from(index).map_err(|_| StopsError::Overflow)?;
                let raw = step
                    .checked_mul(Rational::from_int(times))
                    .and_then(|change| value.raw().checked_add(change))
                    .ok_or(StopsError::Overflow)?;
                Ok(value.with_raw(raw))
            }
            CandidateKind::List(list) => index
                .checked_sub(1)
                .and_then(|i| list.get(i))
                .copied()
                .ok_or(StopsError::Overflow),
        }
    }
}

// ---------------------------------------------------------------------------
// Shared with the grips
// ---------------------------------------------------------------------------

/// A param whose value is written as a plain number, as the trace sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PlainNumber {
    pub value: Amount,
    pub freedom: Freedom,
    pub domain: Option<ParamDomain>,
}

/// The value, kind and domain of a param written as a plain number.
pub(crate) fn plain_number(
    model: &Model,
    param: ParamId,
    path: &str,
) -> Result<PlainNumber, StopsError> {
    if written_number(model, param).is_none() {
        return Err(StopsError::NotANumber(path.to_owned()));
    }
    let trace = stave_solve::trace::trace(model, Quantity::Param(param)).map_err(|error| {
        StopsError::NotTraced {
            path: path.to_owned(),
            reason: error.to_string(),
        }
    })?;
    let Form::Linear(linear) = &trace.form else {
        return Err(StopsError::NotANumber(path.to_owned()));
    };
    let own = original(model, param);
    let term = linear
        .term(&Source::Param(own))
        .ok_or_else(|| StopsError::NotANumber(path.to_owned()))?;
    Ok(PlainNumber {
        value: trace.value,
        freedom: term.dependence.freedom,
        domain: term.dependence.domain.clone(),
    })
}

/// The param a mirrored assembly's param stands for: itself otherwise.
pub(crate) fn original(model: &Model, mut param: ParamId) -> ParamId {
    for _ in 0..model.params.len() {
        match model.params.get(param.index()).map(|p| p.source) {
            Some(ParamSource::SameAs(other)) => param = other,
            _ => break,
        }
    }
    param
}

/// The expression a param's value is written as in the text: its default,
/// or what its assembly gives it.
pub(crate) fn written(model: &Model, param: ParamId) -> Option<&RExpr> {
    let inst = model.params.get(original(model, param).index())?;
    match inst.source {
        ParamSource::Default => model.defs.param(inst.def)?.default.as_ref(),
        ParamSource::Argument {
            assembly, index, ..
        } => {
            let AssemblyKind::Instance { args, .. } = &model.defs.assembly(assembly)?.kind else {
                return None;
            };
            args.get(index).map(|(_, expr)| expr)
        }
        ParamSource::SameAs(_) => None,
    }
}

/// The plain number a param is written as (`500mm`, `-0.75mm`, `3`), or
/// `None` when it is written as anything else.
pub(crate) fn written_number(model: &Model, param: ParamId) -> Option<&RExpr> {
    let expr = written(model, param)?;
    is_number(expr).then_some(expr)
}

#[expect(
    clippy::wildcard_enum_match_arm,
    reason = "picks out only the variants it names: any other, a new one included, is not one of them"
)]
fn is_number(expr: &RExpr) -> bool {
    match &expr.kind {
        RExprKind::Length(_) | RExprKind::Angle(_) | RExprKind::Number(_) => true,
        RExprKind::Unary { op, operand } => {
            *op == stave_lang::ast::UnaryOp::Neg
                && matches!(
                    operand.kind,
                    RExprKind::Length(_) | RExprKind::Angle(_) | RExprKind::Number(_)
                )
        }
        _ => false,
    }
}

/// The param a `set` path addresses: a project param (`overall_width`),
/// or a param of a top-level assembly's component (`divider.height`; for
/// an array, its first element's).
pub fn param_at_path(model: &Model, path: &str) -> Option<ParamId> {
    match path.split_once('.') {
        None => {
            let root = model.module_instance(model.root)?;
            param_named(model, root, path)
        }
        Some((assembly, param)) => {
            let found = model.assemblies.iter().find(|inst| {
                inst.parent.is_none()
                    && model
                        .defs
                        .assembly(inst.def)
                        .is_some_and(|def| def.decl.name == assembly)
            })?;
            param_named(model, found.inner, param)
        }
    }
}

fn param_named(model: &Model, instance: InstanceId, name: &str) -> Option<ParamId> {
    model
        .instance(instance)?
        .items
        .values()
        .find_map(|slot| match slot {
            ItemSlot::Param(param) => {
                let inst = model.params.get(param.index())?;
                let def = model.defs.param(inst.def)?;
                (def.decl.name == name).then_some(*param)
            }
            ItemSlot::Let(_)
            | ItemSlot::Part(_)
            | ItemSlot::Region(_)
            | ItemSlot::Joint(_)
            | ItemSlot::Hardware(_)
            | ItemSlot::Assembly(_)
            | ItemSlot::Require(_) => None,
        })
}

/// The `set` path of a param, when a patch can write it: a project
/// param's name, or `<assembly>.<param>` for a param of a top-level
/// assembly's component (the assembly's declared name, without an
/// element). `None` for a param deeper in, or of a library module.
pub fn path_of_param(model: &Model, param: ParamId) -> Option<String> {
    let param = original(model, param);
    let inst = model.params.get(param.index())?;
    let name = &model.defs.param(inst.def)?.decl.name;
    let instance = model.instance(inst.instance)?;
    match instance.kind {
        InstanceKind::Module(module) => (module == model.root).then(|| name.clone()),
        InstanceKind::Component { assembly, .. } => {
            let assembly = model.assemblies.get(assembly.index())?;
            if assembly.parent.is_some() {
                return None;
            }
            let decl = &model.defs.assembly(assembly.def)?.decl;
            Some(format!("{}.{name}", decl.name))
        }
    }
}

/// The step a value of this kind is searched and snapped in: the
/// project's resolution for a length, a tenth of a degree for an angle,
/// one for a count, a hundredth for a ratio.
pub(crate) fn step_for(value: Amount, resolution: ExactLength) -> Amount {
    match value {
        Amount::Length(_) => Amount::Length(resolution),
        Amount::Angle(_) => Amount::Angle(ExactAngle::new(Rational::from_int(100_000))),
        Amount::Count(_) => Amount::Count(Rational::ONE),
        Amount::Ratio(_) => Amount::Ratio(Rational::new(1, 100).unwrap_or(Rational::ONE)),
    }
}

/// `value` rounded to the nearest multiple of `step` (halves away from
/// zero). `None` on overflow or a step of zero.
pub(crate) fn snap(value: Amount, step: Amount) -> Option<Amount> {
    let times = value.raw().checked_div(step.raw())?.round();
    Some(value.with_raw(step.raw().checked_mul(Rational::from_int(times))?))
}

/// An amount as a plain number in `.stave` text: `180mm`, `-0.75mm`,
/// `12.5deg`, `3`, `0.37`. A length or angle that is not a whole number of
/// nanometres or micro-degrees is written as an exact fractional quantity
/// (`1/3mm`). A count or ratio is written as an integer or a decimal: the
/// grammar has no unitless fraction literal, so a value with no finite
/// decimal (a third) has no plain-number text and gives `None`, as does a
/// number that does not fit.
pub fn amount_text(value: Amount) -> Option<String> {
    match value {
        Amount::Length(length) => match length.to_length() {
            Some(length) => Some(length.to_string()),
            None => fraction_text(
                length.value(),
                i128::from(stave_lang::units::NM_PER_MM),
                "mm",
            ),
        },
        Amount::Angle(angle) => match angle.to_angle() {
            Some(angle) => Some(angle.to_string()),
            None => fraction_text(
                angle.value(),
                i128::from(stave_lang::units::MICRODEG_PER_DEG),
                "deg",
            ),
        },
        Amount::Count(n) | Amount::Ratio(n) => decimal_text(n),
    }
}

/// The fewest decimal places that write `n` exactly, or `None` when its
/// denominator has a prime factor other than 2 and 5 (no finite decimal).
pub(crate) fn decimal_places(n: Rational) -> Option<u32> {
    let mut rest = n.denominator();
    let (mut twos, mut fives) = (0u32, 0u32);
    while rest % 2 == 0 {
        rest /= 2;
        twos += 1;
    }
    while rest % 5 == 0 {
        rest /= 5;
        fives += 1;
    }
    (rest == 1).then_some(twos.max(fives))
}

/// `n` as an INT or DECIMAL literal (with a leading `-` for a negative
/// value): `3`, `0.37`, `-1.5`. `None` when its denominator has a prime
/// factor other than 2 and 5 (no finite decimal), or the digits don't fit
/// a literal.
fn decimal_text(n: Rational) -> Option<String> {
    if n.is_integer() {
        return Some(n.to_string());
    }
    let places = decimal_places(n)?;
    let scale = 10i128.checked_pow(places)?;
    let digits = n.numerator().checked_mul(scale)? / n.denominator();
    let magnitude = u64::try_from(digits.unsigned_abs()).ok()?;
    let places = usize::try_from(places).ok()?;
    let text = format!("{magnitude:0>width$}", width = places + 1);
    let (whole, fraction) = text.split_at(text.len() - places);
    let sign = if digits < 0 { "-" } else { "" };
    Some(format!("{sign}{whole}.{fraction}"))
}

fn fraction_text(raw: Rational, per_unit: i128, unit: &str) -> Option<String> {
    let units = raw.checked_div(Rational::from_int(per_unit))?;
    Some(format!(
        "{}/{}{unit}",
        units.numerator(),
        units.denominator()
    ))
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::sync::atomic::AtomicBool;

    use stave_lang::Code;
    use stave_modules::MemoryFiles;

    use super::*;

    const FOLDER: &str = "scratch/clothing_cabinet_from_box";

    /// A copy of `examples/clothing_cabinet_from_box`, in memory.
    fn cabinet() -> MemoryFiles {
        let example =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/clothing_cabinet_from_box");
        let mut files = MemoryFiles::new();
        for entry in std::fs::read_dir(&example).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_some_and(|e| e == "stave") {
                let name = path.file_name().unwrap().to_owned();
                files.insert(
                    PathBuf::from(FOLDER).join(name),
                    std::fs::read_to_string(&path).unwrap(),
                );
            }
        }
        files
    }

    fn mm(n: i64) -> Amount {
        Amount::Length(ExactLength::new(Rational::from_int(
            i128::from(n) * 1_000_000,
        )))
    }

    fn request(path: &str) -> StopsRequest<'_> {
        StopsRequest {
            path,
            limit: Bounds {
                low: mm(0),
                high: mm(2000),
            },
        }
    }

    /// The divider's height searched each way from 500mm in 0.5mm steps:
    /// down to 180mm, where the drawer boxes become too shallow (see the
    /// grips' test for the arithmetic), and up to where the shelf's hinge
    /// plate runs into the shelf, in tens of checks rather than the 4000
    /// steps the limit allows.
    #[test]
    fn the_divider_height_is_searched_in_tens_of_checks() {
        let files = cabinet();
        let base = EditBase::new(Path::new(FOLDER), &files).unwrap();
        let stops = find_stops(&base, request("divider.height"), &AtomicBool::new(false)).unwrap();
        assert_eq!(stops.value, mm(500));
        assert_eq!(stops.below.last_good, mm(180));
        let SideEnd::Stopped { diagnostic, .. } = &stops.below.end else {
            panic!("a lower stop");
        };
        assert_eq!(diagnostic.code, Code::Require);
        assert!(matches!(stops.above.end, SideEnd::Stopped { .. }));
        assert!(stops.checks < 100, "{} checks", stops.checks);
    }

    #[test]
    fn a_search_can_be_cancelled() {
        let files = cabinet();
        let base = EditBase::new(Path::new(FOLDER), &files).unwrap();
        let cancelled = AtomicBool::new(true);
        let result = find_stops(&base, request("divider.height"), &cancelled);
        assert!(matches!(result, Err(StopsError::Cancelled)), "{result:?}");
    }

    #[test]
    fn only_a_plain_number_can_be_searched() {
        let files = cabinet();
        let base = EditBase::new(Path::new(FOLDER), &files).unwrap();
        let cancel = AtomicBool::new(false);
        let result = find_stops(&base, request("drawer.box_height"), &cancel);
        assert!(
            matches!(result, Err(StopsError::NotANumber(_))),
            "{result:?}"
        );
        let result = find_stops(&base, request("divider.nothing"), &cancel);
        assert!(
            matches!(result, Err(StopsError::UnknownPath(_))),
            "{result:?}"
        );
        let result = find_stops(
            &base,
            StopsRequest {
                path: "divider.height",
                limit: Bounds {
                    low: mm(600),
                    high: mm(400),
                },
            },
            &cancel,
        );
        assert!(matches!(result, Err(StopsError::BadLimit(_))), "{result:?}");
    }

    #[test]
    fn values_are_written_as_plain_numbers_or_exact_fractions() {
        assert_eq!(amount_text(mm(180)).as_deref(), Some("180mm"));
        let third = Amount::Length(ExactLength::new(Rational::new(1_000_000, 3).unwrap()));
        assert_eq!(amount_text(third).as_deref(), Some("1/3mm"));
        let ratio = |n, d| Amount::Ratio(Rational::new(n, d).unwrap());
        assert_eq!(amount_text(ratio(37, 100)).as_deref(), Some("0.37"));
        assert_eq!(amount_text(ratio(1, 2)).as_deref(), Some("0.5"));
        assert_eq!(amount_text(ratio(1, 40)).as_deref(), Some("0.025"));
        assert_eq!(amount_text(ratio(-3, 2)).as_deref(), Some("-1.5"));
        assert_eq!(amount_text(ratio(7, 1)).as_deref(), Some("7"));
        assert_eq!(
            amount_text(Amount::Count(Rational::from_int(3))).as_deref(),
            Some("3")
        );
        // No finite decimal: no plain number, and never a division.
        assert_eq!(amount_text(ratio(1, 3)), None);
        let step = mm(1).with_raw(Rational::from_int(500_000));
        assert_eq!(
            snap(mm(1).with_raw(Rational::from_int(400_200_000)), step),
            Some(mm(400))
        );
    }

    #[test]
    fn paths_address_project_and_assembly_params() {
        let files = cabinet();
        let base = EditBase::new(Path::new(FOLDER), &files).unwrap();
        let model = base.opened().model.as_ref().unwrap();
        for path in ["overall_width", "divider.height", "drawer.top_gap"] {
            let param = param_at_path(model, path).unwrap();
            assert_eq!(path_of_param(model, param).as_deref(), Some(path));
        }
    }
}
