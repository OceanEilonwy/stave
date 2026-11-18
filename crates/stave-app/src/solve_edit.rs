//! Smart edits over solved params, headless (grammar, "Smart edits", rule
//! 3): drag a size, and z3 picks which `solved param`s change and by how
//! much.
//!
//! ```text
//! size ──trace──▶ unknowns (solved params, linear) ─┐
//! requires ──trace──▶ linear constraints ────────────┼─▶ stave-smt ─▶ exact values
//! within ──▶ bounds or lists ────────────────────────┘        │
//!            snap to the resolution ◀──────────────────────────┘
//!            one patch of `set`s ──▶ scratch check ──▶ fits or refused
//! ```
//!
//! **From a target to a patch** ([`solve_size`]). The size is traced
//! ([`stave_solve::trace::trace`]). Its unknowns are the solved params it
//! depends on linearly that have a `within` and that a patch can set (a
//! project param, `set overall_width = ...`, or a param of an assembly
//! declared in the project, `set drawer.setback = ...`; a param of an
//! array's element or of an assembly nested inside a component is held,
//! because one `set` would change every element or no path reaches it).
//! Plain and locked params, stocks' fields and kinds' sizes are held at
//! their values. When no solved param is involved the answer is
//! [`SizeSolve::Fallback`], so the caller uses single-param inversion
//! ([`stave_model::trace::invert`]) instead. A size that is not linear as a
//! whole is solved only when exactly one solved param is linear in it with
//! the others held; with more it falls back too, since their coefficients
//! hold only one at a time.
//!
//! The problem given to [`stave_smt`]: each unknown's domain is its
//! `within` (a range or a list); the target is an equality; and every
//! `require` (the project's and those inside its assemblies) whose two
//! sides trace as linear in the unknowns becomes a constraint labelled
//! with the require's name. The objective is the grammar's ordering: how
//! many params change, how far as a fraction of their domain, how many
//! component levels from the size, then declaration order (file, then
//! position). Requires that are not linear, the requires of kinds, and
//! every geometric check are not in the problem; the scratch check
//! ([`crate::scratch`]) enforces them, so the answer is always judged by
//! it, and a refusal is reported as [`SizeSolve::Refused`].
//!
//! The exact answer is snapped to the project's `resolution:` (a count to
//! a whole number, an angle to a whole micro-degree; a ratio and a value
//! from a `within` list are left as they are). When snapping breaks the
//! equality, the problem is solved again with the snapped values pinned,
//! each changed param in turn left free (in declaration order), and the
//! first answer that lands on the resolution exactly is taken. When none
//! does, the snapped values are kept and [`SolvedEdit::reached`] says what
//! size they give.
//!
//! When there is no answer, [`SizeSolve::Unsatisfiable`] names the
//! smallest set of things that conflict ([`Blocker`]): the requires by
//! name with their messages, the `within`s, and the dragged size itself.
//!
//! **A drag** ([`Drag`]). Starting a drag solves once, at the first
//! target, to choose which params move, and keeps the answer as a
//! *piece*: the params that moved and are not at a bound change at exact
//! rates along the drag, fixed by the constraints that are tight at the
//! answer (the target and any require on its edge), so for any target in
//! the piece the values are worked out by exact rational arithmetic
//! without the solver. A piece starts at the target it was found at
//! (that end included) and reaches only away from where the drag started
//! (the size as it is now); it ends where a moving param reaches a bound
//! of its domain or its starting value, or where a require that was not
//! tight reaches its edge. A target outside every piece, including one
//! between the start and the target a piece was found at, solves again
//! and adds a piece. [`Drag::reach`] is the range of targets that can be
//! reached at all, with every domain and linear require kept.
//!
//! Why a piece only reaches away from the start: towards the start, a
//! cheaper set of params that a require blocked at the found target can
//! be allowed again. Away from it, that can't happen. The targets a set
//! of params can reach (the others held) form an interval holding the
//! start, so a set that can't reach the found target can't reach
//! anything further out, and the number of params that change never
//! drops. And the least fraction a set needs grows at least in
//! proportion to the distance from the start (it is convex in the target
//! and zero at the start). So when a piece's values are in proportion
//! too (each param that moves is on a line through its starting value at
//! the start, and every other param is at its starting value), the piece is the solver's answer at every target in it, up to
//! a tie between different values that are exactly as good. A piece
//! whose values are not in proportion (a param held at a bound it moved
//! to, or a require tight off the start) is the solver's answer where it
//! was found and follows the same tight constraints; it stays the
//! solver's answer unless a different set of the same number of params
//! becomes cheaper part way along, which the tests check against solving
//! but which is not proven. A param restricted to a `within` list has no
//! rate: a piece in which one moves holds only its own target. The same
//! drag gives the same values every time.

use std::collections::BTreeMap;
use std::sync::atomic::AtomicBool;

use stave_lang::units::{ExactLength, Rational, Resolution};
use stave_model::Model;
use stave_model::defs::DefId;
use stave_model::ids::ParamId;
use stave_model::instance::InstanceKind;
use stave_model::trace::{Amount, Form, Freedom, ParamDomain, Quantity, Source, Term, Trace};
use stave_smt as smt;
use stave_solve::trace::{Comparison, TraceError, trace, trace_requires};

use crate::edit::Verdict;
use crate::repeat::{GeneratedPatch, PatchBuildError};
use crate::scratch::{EditBase, Tried, TryError};

/// The label of the target equality in the problem; require labels start
/// with `require `, so it can't clash with one.
const TARGET_LABEL: &str = "size";

/// The name a solve's patch file is shown under.
const PATCH_NAME: &str = "solve.stave";

/// Comparisons of plain numbers (counts and ratios) are scaled by this
/// before solving, so that the strict margin of 1 is a billionth.
const NUMBER_SCALE: i64 = 1_000_000_000;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Why a smart edit couldn't be worked out at all. An edit with no answer
/// is not an error: it is [`SizeSolve::Unsatisfiable`].
#[derive(Debug, thiserror::Error)]
pub enum SolveEditError {
    /// The project's design doesn't solve, so there is nothing to edit.
    #[error("the design has errors that stop it being placed; fix them first")]
    NoDesign,
    /// The size couldn't be traced.
    #[error(transparent)]
    Trace(#[from] TraceError),
    /// The target is not the same kind of value as the size.
    #[error("the target is not the same kind of value as the size")]
    Mismatch,
    /// The numbers are too large to work with exactly.
    #[error("the numbers are too large")]
    Overflow,
    /// The solver failed.
    #[error(transparent)]
    Smt(#[from] smt::SmtError),
    /// The written patch didn't parse: a bug.
    #[error(transparent)]
    Patch(#[from] PatchBuildError),
    /// The scratch check couldn't run, or was cancelled.
    #[error(transparent)]
    Try(#[from] TryError),
}

/// Why a size is left to single-param inversion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FallbackReason {
    /// The size depends on no solved param that a patch can set (with a
    /// `within`, linearly).
    NoSolvedParam,
    /// The size is not linear as a whole and more than one solved param
    /// is linear in it only with the others held.
    NotLinear,
}

/// A size the solver doesn't handle, with its trace, for single-param
/// inversion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fallback {
    /// The trace of the size.
    pub trace: Trace,
    /// Why.
    pub reason: FallbackReason,
}

/// One thing that stops a target being reached.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Blocker {
    /// The `within` of a solved param.
    Within {
        /// The param.
        param: ParamId,
        /// How a patch names it (`overall_width`, `drawer.setback`).
        path: String,
    },
    /// A `require`.
    Require {
        /// Its name; inside an assembly, prefixed with the assembly's.
        name: String,
        /// Its message, with values filled in at the current design.
        message: String,
    },
    /// The dragged size equal to the target.
    Size,
}

/// One param a smart edit changes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParamChange {
    /// The param.
    pub param: ParamId,
    /// How the patch names it.
    pub path: String,
    /// Its value now.
    pub from: Amount,
    /// Its new value, on the project's resolution.
    pub to: Amount,
}

/// A solver's answer as a patch, judged by the scratch check.
#[derive(Debug)]
pub struct SolvedEdit {
    /// The params it changes, in declaration order.
    pub changes: Vec<ParamChange>,
    /// The size the new values give: the target, unless snapping to the
    /// resolution made that impossible.
    pub reached: Amount,
    /// The patch: one `set` per change.
    pub patch: GeneratedPatch,
    /// The patch tried on a scratch copy.
    pub tried: Tried,
}

impl SolvedEdit {
    /// The scratch check's verdict: fits or refused.
    pub fn verdict(&self) -> &Verdict {
        self.tried.verdict()
    }
}

/// What [`solve_size`] found.
#[derive(Debug)]
pub enum SizeSolve {
    /// The solver doesn't handle this size: use single-param inversion.
    Fallback(Fallback),
    /// The size is already the target: nothing changes.
    Unchanged,
    /// The answer, and the scratch check says it fits.
    Fits(SolvedEdit),
    /// The answer, but the scratch check refuses it (a require that is
    /// not linear, a kind's require, a geometric check): its verdict says
    /// why.
    Refused(SolvedEdit),
    /// No answer keeps every `within` and linear `require`: the smallest
    /// set that conflicts, sorted.
    Unsatisfiable(Vec<Blocker>),
    /// The solver ran out of time.
    TimedOut,
}

/// Solves for the solved params a size depends on so that it becomes
/// `target`, writes the answer as one patch of `set`s on the project's
/// resolution, and tries it on a scratch copy of `base` (see the [module
/// documentation](self)). `cancel` cancels the scratch check's slow tier.
///
/// # Errors
///
/// [`SolveEditError`] when the base design doesn't solve, the size can't
/// be traced, the target is of another kind, the numbers overflow, the
/// solver fails, or the scratch check can't run or is cancelled.
pub fn solve_size(
    base: &EditBase<'_>,
    quantity: Quantity,
    target: Amount,
    cancel: &AtomicBool,
) -> Result<SizeSolve, SolveEditError> {
    let setup = match Setup::prepare(base, quantity)? {
        Ready::Fallback(fallback) => return Ok(SizeSolve::Fallback(*fallback)),
        Ready::Setup(setup) => setup,
    };
    if !target.same_dimension(setup.current) {
        return Err(SolveEditError::Mismatch);
    }
    let target = target.raw();
    let values = match setup.solve_at(target, &BTreeMap::new())? {
        Answer::Solved(values) => values,
        Answer::Unsatisfiable(blockers) => return Ok(SizeSolve::Unsatisfiable(blockers)),
        Answer::TimedOut => return Ok(SizeSolve::TimedOut),
    };
    let (values, reached) = setup.snapped(target, values)?;
    let changes = setup.changes(&values);
    if changes.is_empty() {
        return Ok(SizeSolve::Unchanged);
    }
    let mut ops = Vec::with_capacity(changes.len());
    let mut said = Vec::with_capacity(changes.len());
    for change in &changes {
        let text = value_text(change.to).ok_or(SolveEditError::Overflow)?;
        ops.push(format!("set {} = {text}", change.path));
        said.push(format!("{} to {text}", change.path));
    }
    let patch = GeneratedPatch::new(&format!("Set {}", said.join(", ")), &ops)?;
    let tried = base.try_patch(PATCH_NAME, &patch.parse, cancel)?;
    let edit = SolvedEdit {
        changes,
        reached: setup.current.with_raw(reached),
        patch,
        tried,
    };
    Ok(if edit.tried.fits() {
        SizeSolve::Fits(edit)
    } else {
        SizeSolve::Refused(edit)
    })
}

/// The patch that writes `changes`: one `set` each, in their order.
///
/// # Errors
///
/// [`SolveEditError::Overflow`] when a value has no text, or the patch
/// doesn't parse (a bug).
pub fn changes_patch(changes: &[ParamChange]) -> Result<GeneratedPatch, SolveEditError> {
    let mut ops = Vec::with_capacity(changes.len());
    let mut said = Vec::with_capacity(changes.len());
    for change in changes {
        let text = value_text(change.to).ok_or(SolveEditError::Overflow)?;
        ops.push(format!("set {} = {text}", change.path));
        said.push(format!("{} to {text}", change.path));
    }
    Ok(GeneratedPatch::new(
        &format!("Set {}", said.join(", ")),
        &ops,
    )?)
}

// ---------------------------------------------------------------------------
// The drag
// ---------------------------------------------------------------------------

/// What [`Drag::start`] gives.
#[derive(Debug)]
pub enum DragStart {
    /// The solver doesn't handle this size: use single-param inversion.
    Fallback(Fallback),
    /// The drag, ready for targets.
    Ready(Drag),
}

/// The param values at one target of a drag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DragValues {
    /// Every unknown of the drag and its exact value, in declaration
    /// order (unsnapped: a drag shows the exact answer; the patch written
    /// at the end of the drag is [`solve_size`]'s).
    pub values: Vec<(ParamId, Amount)>,
    /// `true` when the solver ran for this target; `false` when the
    /// values came from a piece by arithmetic.
    pub solved: bool,
}

/// One step of a drag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DragStep {
    /// The values for the target.
    Values(DragValues),
    /// The target can't be reached: the smallest set that conflicts.
    Unreachable(Vec<Blocker>),
    /// The solver ran out of time.
    TimedOut,
}

/// A drag on a size in progress: gives the solved params' values for any
/// target, solving only when a target leaves every piece found so far
/// (see the [module documentation](self)).
#[derive(Debug)]
pub struct Drag {
    setup: Setup,
    pieces: Vec<Piece>,
    reach: Option<(Amount, Amount)>,
    solves: usize,
}

/// A stretch of targets over which the values change at fixed rates.
#[derive(Debug, Clone)]
struct Piece {
    /// The target it was found at.
    at: Rational,
    /// The values there, one per unknown.
    values: Vec<Rational>,
    /// How fast each value changes per raw unit of target.
    rates: Vec<Rational>,
    /// How far below `at` the piece reaches, and whether that end is left
    /// out; `None` for no end.
    below: Option<(Rational, bool)>,
    /// How far above `at` it reaches, likewise.
    above: Option<(Rational, bool)>,
}

impl Piece {
    fn holds(&self, target: Rational) -> Option<bool> {
        let step = target.checked_sub(self.at)?;
        let low_ok = match self.below {
            None => true,
            Some((reach, open)) => {
                let lowest = reach.checked_neg()?;
                if open { step > lowest } else { step >= lowest }
            }
        };
        let high_ok = match self.above {
            None => true,
            Some((reach, open)) => {
                if open {
                    step < reach
                } else {
                    step <= reach
                }
            }
        };
        Some(low_ok && high_ok)
    }

    fn values_at(&self, target: Rational) -> Option<Vec<Rational>> {
        let step = target.checked_sub(self.at)?;
        self.values
            .iter()
            .zip(&self.rates)
            .map(|(value, rate)| value.checked_add(rate.checked_mul(step)?))
            .collect()
    }
}

impl Drag {
    /// Starts a drag on `quantity`: solves once at `first` to choose which
    /// params move, and works out the range of targets that can be
    /// reached at all.
    ///
    /// # Errors
    ///
    /// As for [`solve_size`], without the scratch check.
    pub fn start(
        base: &EditBase<'_>,
        quantity: Quantity,
        first: Amount,
    ) -> Result<DragStart, SolveEditError> {
        let setup = match Setup::prepare(base, quantity)? {
            Ready::Fallback(fallback) => return Ok(DragStart::Fallback(*fallback)),
            Ready::Setup(setup) => setup,
        };
        if !first.same_dimension(setup.current) {
            return Err(SolveEditError::Mismatch);
        }
        let reach = setup.reach()?;
        let mut drag = Drag {
            setup,
            pieces: Vec::new(),
            reach,
            solves: 0,
        };
        // The first answer becomes the first piece; an unreachable first
        // target is reported again by `values_at`.
        drag.solve(first.raw())?;
        Ok(DragStart::Ready(drag))
    }

    /// The least and greatest targets the drag can reach at all, keeping
    /// every `within` and linear `require`; `None` when even the design
    /// as it is breaks one of them.
    pub fn reach(&self) -> Option<(Amount, Amount)> {
        self.reach
    }

    /// The params the drag may change, with how a patch names them, in
    /// declaration order.
    pub fn params(&self) -> impl Iterator<Item = (ParamId, &str)> {
        self.setup
            .unknowns
            .iter()
            .map(|unknown| (unknown.param, unknown.path.as_str()))
    }

    /// The changes a drag that stops at `values` writes: every param that
    /// moved, its value snapped to the project's resolution as
    /// [`solve_size`] snaps it (without solving again), and the size the
    /// snapped values give, which can miss the target by less than a step.
    ///
    /// # Errors
    ///
    /// [`SolveEditError::Overflow`] when the numbers are too large.
    pub fn changes_at(
        &self,
        values: &DragValues,
    ) -> Result<(Vec<ParamChange>, Amount), SolveEditError> {
        let raw: Vec<Rational> = values.values.iter().map(|(_, value)| value.raw()).collect();
        let snapped = self.setup.snap_all(&raw)?;
        let size = self
            .setup
            .size_of(&snapped)
            .ok_or(SolveEditError::Overflow)?;
        Ok((
            self.setup.changes(&snapped),
            self.setup.current.with_raw(size),
        ))
    }

    /// How many times the solver has run for targets (the reach is not
    /// counted).
    pub fn solves(&self) -> usize {
        self.solves
    }

    /// The values at `target`: by arithmetic inside a piece found before,
    /// otherwise by solving again.
    ///
    /// # Errors
    ///
    /// [`SolveEditError::Mismatch`] for a target of another kind; as for
    /// [`solve_size`] when solving fails.
    pub fn values_at(&mut self, target: Amount) -> Result<DragStep, SolveEditError> {
        if !target.same_dimension(self.setup.current) {
            return Err(SolveEditError::Mismatch);
        }
        let raw = target.raw();
        for piece in &self.pieces {
            if piece.holds(raw).ok_or(SolveEditError::Overflow)? {
                let values = piece.values_at(raw).ok_or(SolveEditError::Overflow)?;
                return Ok(DragStep::Values(self.setup.drag_values(&values, false)));
            }
        }
        self.solve(raw)
    }

    fn solve(&mut self, target: Rational) -> Result<DragStep, SolveEditError> {
        self.solves += 1;
        match self.setup.solve_at(target, &BTreeMap::new())? {
            Answer::Solved(values) => {
                let piece = self.setup.piece(target, values)?;
                let step = DragStep::Values(self.setup.drag_values(&piece.values, true));
                self.pieces.push(piece);
                Ok(step)
            }
            Answer::Unsatisfiable(blockers) => Ok(DragStep::Unreachable(blockers)),
            Answer::TimedOut => Ok(DragStep::TimedOut),
        }
    }
}

// ---------------------------------------------------------------------------
// The problem
// ---------------------------------------------------------------------------

/// One solved param the solver may change.
#[derive(Debug, Clone)]
struct Unknown {
    param: ParamId,
    /// How a patch names it; also its name in the problem.
    path: String,
    current: Amount,
    /// Its coefficient in the size.
    coefficient: Rational,
    domain: ParamDomain,
    levels: u32,
    order: u32,
}

impl Unknown {
    fn bounds(&self) -> Option<(Rational, Rational)> {
        match &self.domain {
            ParamDomain::Range { low, high } => Some((low.raw(), high.raw())),
            ParamDomain::OneOf(values) => {
                let lowest = values.iter().map(|v| v.raw()).min()?;
                let highest = values.iter().map(|v| v.raw()).max()?;
                Some((lowest, highest))
            }
        }
    }

    fn listed(&self) -> bool {
        matches!(self.domain, ParamDomain::OneOf(_))
    }
}

/// A linear comparison of a require: `Σ coefficient × unknown  relation
/// constant`.
#[derive(Debug, Clone)]
struct Row {
    label: String,
    name: String,
    message: String,
    terms: Vec<(usize, Rational)>,
    relation: smt::Relation,
    constant: Rational,
}

/// A size ready to solve for.
#[derive(Debug, Clone)]
struct Setup {
    /// The size now.
    current: Amount,
    /// The size is `constant + Σ coefficient × unknown`.
    constant: Rational,
    unknowns: Vec<Unknown>,
    rows: Vec<Row>,
    resolution: Resolution,
}

enum Ready {
    Setup(Setup),
    Fallback(Box<Fallback>),
}

enum Answer {
    Solved(Vec<Rational>),
    Unsatisfiable(Vec<Blocker>),
    TimedOut,
}

impl Setup {
    fn prepare(base: &EditBase<'_>, quantity: Quantity) -> Result<Ready, SolveEditError> {
        let opened = base.opened();
        let (Some(model), Some(design)) = (opened.model.as_ref(), opened.design.as_ref()) else {
            return Err(SolveEditError::NoDesign);
        };
        let traced = trace(model, quantity)?;
        let is_usable = |term: &&Term| usable(model, term).is_some();
        let fallback = |trace: &Trace, reason| {
            Ok(Ready::Fallback(Box::new(Fallback {
                trace: trace.clone(),
                reason,
            })))
        };
        let (constant, chosen): (Rational, Vec<&Term>) = match &traced.form {
            Form::Linear(linear) => {
                let chosen: Vec<&Term> = linear.terms.iter().filter(is_usable).collect();
                let mut constant = linear.constant.raw();
                for term in &linear.terms {
                    if !chosen
                        .iter()
                        .any(|c| c.dependence.source == term.dependence.source)
                    {
                        constant = constant
                            .checked_add(
                                term.coefficient
                                    .checked_mul(term.dependence.value.raw())
                                    .ok_or(SolveEditError::Overflow)?,
                            )
                            .ok_or(SolveEditError::Overflow)?;
                    }
                }
                (constant, chosen)
            }
            Form::NotLinear(not) => {
                let chosen: Vec<&Term> = not.held.iter().filter(is_usable).collect();
                match chosen.as_slice() {
                    [] => return fallback(&traced, FallbackReason::NoSolvedParam),
                    [one] => {
                        let part = one
                            .coefficient
                            .checked_mul(one.dependence.value.raw())
                            .ok_or(SolveEditError::Overflow)?;
                        let constant = traced
                            .value
                            .raw()
                            .checked_sub(part)
                            .ok_or(SolveEditError::Overflow)?;
                        (constant, chosen)
                    }
                    _ => return fallback(&traced, FallbackReason::NotLinear),
                }
            }
        };
        if chosen.is_empty() {
            return fallback(&traced, FallbackReason::NoSolvedParam);
        }
        let mut unknowns = Vec::with_capacity(chosen.len());
        for term in chosen {
            let Some(Usable {
                param,
                path,
                domain,
                place,
            }) = usable(model, term)
            else {
                continue;
            };
            unknowns.push((
                place,
                Unknown {
                    param,
                    path,
                    current: term.dependence.value,
                    coefficient: term.coefficient,
                    domain,
                    levels: term.dependence.levels,
                    order: 0,
                },
            ));
        }
        unknowns.sort_by_key(|(place, _)| *place);
        let unknowns: Vec<Unknown> = unknowns
            .into_iter()
            .enumerate()
            .map(|(order, (_, mut unknown))| {
                unknown.order = u32::try_from(order).unwrap_or(u32::MAX);
                unknown
            })
            .collect();
        let resolution = Resolution::new(design.project.resolution).unwrap_or(Resolution::DEFAULT);
        let mut setup = Setup {
            current: traced.value,
            constant,
            unknowns,
            rows: Vec::new(),
            resolution,
        };
        setup.rows = setup.require_rows(model, design)?;
        Ok(Ready::Setup(setup))
    }

    fn index_of(&self, param: ParamId) -> Option<usize> {
        self.unknowns.iter().position(|u| u.param == param)
    }

    /// Every comparison of a require that is linear in the unknowns and
    /// depends on at least one.
    fn require_rows(
        &self,
        model: &Model,
        design: &stave_model::solved::SolvedDesign,
    ) -> Result<Vec<Row>, SolveEditError> {
        let overflow = || SolveEditError::Overflow;
        let mut rows = Vec::new();
        for (id, traced) in trace_requires(model) {
            let Ok(comparisons) = traced else {
                continue;
            };
            let Some(inst) = model.requires.get(id.index()) else {
                continue;
            };
            let message = design
                .requires
                .iter()
                .find(|result| result.name == inst.name)
                .map(|result| result.message.clone())
                .unwrap_or_default();
            let mut written = 0_usize;
            for comparison in comparisons {
                let Form::Linear(linear) = &comparison.form else {
                    continue;
                };
                let mut constant = linear.constant.raw();
                let mut terms = Vec::new();
                for term in &linear.terms {
                    let index = match &term.dependence.source {
                        Source::Param(param) => self.index_of(*param),
                        Source::StockField { .. } | Source::KindSize { .. } => None,
                    };
                    match index {
                        Some(index) => terms.push((index, term.coefficient)),
                        None => {
                            constant = constant
                                .checked_add(
                                    term.coefficient
                                        .checked_mul(term.dependence.value.raw())
                                        .ok_or_else(overflow)?,
                                )
                                .ok_or_else(overflow)?;
                        }
                    }
                }
                if terms.is_empty() {
                    continue;
                }
                // Σ a × x + constant  relation  0, as Σ a × x relation −constant;
                // plain numbers scaled so the strict margin is small.
                let scale = match comparison.value {
                    Amount::Count(_) | Amount::Ratio(_) => Rational::from(NUMBER_SCALE),
                    Amount::Length(_) | Amount::Angle(_) => Rational::ONE,
                };
                let terms = terms
                    .into_iter()
                    .map(|(index, a)| Some((index, a.checked_mul(scale)?)))
                    .collect::<Option<Vec<_>>>()
                    .ok_or_else(overflow)?;
                let constant = constant
                    .checked_neg()
                    .and_then(|c| c.checked_mul(scale))
                    .ok_or_else(overflow)?;
                written += 1;
                let label = if written == 1 {
                    format!("require {}", inst.name)
                } else {
                    format!("require {} ({written})", inst.name)
                };
                rows.push(Row {
                    label,
                    name: inst.name.clone(),
                    message: message.clone(),
                    terms,
                    relation: relation(comparison.comparison),
                    constant,
                });
            }
        }
        Ok(rows)
    }

    /// The problem at `target` (without the target equality for `None`),
    /// with the unknowns in `pins` held at the given values.
    fn problem(
        &self,
        target: Option<Rational>,
        pins: &BTreeMap<usize, Rational>,
    ) -> Result<smt::Problem, SolveEditError> {
        let mut problem = smt::Problem::new();
        for (index, unknown) in self.unknowns.iter().enumerate() {
            let domain = match pins.get(&index) {
                Some(pin) => smt::Domain::values([exact(*pin)?])?,
                None => match &unknown.domain {
                    ParamDomain::Range { low, high } => {
                        smt::Domain::range(exact(low.raw())?, exact(high.raw())?)?
                    }
                    ParamDomain::OneOf(values) => smt::Domain::values(
                        values
                            .iter()
                            .map(|v| exact(v.raw()))
                            .collect::<Result<Vec<_>, _>>()?,
                    )?,
                },
            };
            problem.add_unknown(smt::Unknown {
                name: unknown.path.clone(),
                current: exact(unknown.current.raw())?,
                domain,
                rank: unknown.levels,
                order: unknown.order,
            });
        }
        if let Some(target) = target {
            let wanted = target
                .checked_sub(self.constant)
                .ok_or(SolveEditError::Overflow)?;
            problem.add_constraint(smt::Constraint::new(
                TARGET_LABEL,
                self.size_terms()?,
                smt::Relation::Equal,
                exact(wanted)?,
            ));
        }
        for row in &self.rows {
            let terms = row
                .terms
                .iter()
                .map(|(index, a)| {
                    let unknown = self.unknowns.get(*index).ok_or(SolveEditError::Overflow)?;
                    Ok((unknown.path.clone(), exact(*a)?))
                })
                .collect::<Result<Vec<_>, SolveEditError>>()?;
            problem.add_constraint(smt::Constraint::new(
                row.label.clone(),
                terms,
                row.relation,
                exact(row.constant)?,
            ));
        }
        Ok(problem)
    }

    fn solve_at(
        &self,
        target: Rational,
        pins: &BTreeMap<usize, Rational>,
    ) -> Result<Answer, SolveEditError> {
        match self.problem(Some(target), pins)?.solve()? {
            smt::Outcome::Solved(solution) => {
                let values = self
                    .unknowns
                    .iter()
                    .map(|u| {
                        let value = solution.value(&u.path).ok_or(SolveEditError::Overflow)?;
                        Rational::new(value.numerator(), value.denominator())
                            .ok_or(SolveEditError::Overflow)
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(Answer::Solved(values))
            }
            smt::Outcome::Unsatisfiable(conflicts) => {
                Ok(Answer::Unsatisfiable(self.blockers(&conflicts)))
            }
            smt::Outcome::TimedOut => Ok(Answer::TimedOut),
        }
    }

    fn blockers(&self, conflicts: &[smt::Conflict]) -> Vec<Blocker> {
        let mut blockers = Vec::new();
        for conflict in conflicts {
            let blocker = match conflict {
                smt::Conflict::Domain(name) => {
                    self.unknowns
                        .iter()
                        .find(|u| &u.path == name)
                        .map(|u| Blocker::Within {
                            param: u.param,
                            path: u.path.clone(),
                        })
                }
                smt::Conflict::Constraint(label) if label == TARGET_LABEL => Some(Blocker::Size),
                smt::Conflict::Constraint(label) => self
                    .rows
                    .iter()
                    .find(|r| &r.label == label)
                    .map(|r| Blocker::Require {
                        name: r.name.clone(),
                        message: r.message.clone(),
                    }),
            };
            if let Some(blocker) = blocker
                && !blockers.contains(&blocker)
            {
                blockers.push(blocker);
            }
        }
        blockers.sort();
        blockers
    }

    /// The range of targets: the least and greatest size over every answer
    /// that keeps the domains and the linear requires.
    fn reach(&self) -> Result<Option<(Amount, Amount)>, SolveEditError> {
        let problem = self.problem(None, &BTreeMap::new())?;
        match problem.reach(self.size_terms()?)? {
            smt::Reach::Between { lowest, highest } => {
                let end = |value: smt::Rational| {
                    Rational::new(value.numerator(), value.denominator())
                        .and_then(|v| v.checked_add(self.constant))
                        .map(|v| self.current.with_raw(v))
                        .ok_or(SolveEditError::Overflow)
                };
                Ok(Some((end(lowest)?, end(highest)?)))
            }
            smt::Reach::Unsatisfiable(_) | smt::Reach::TimedOut => Ok(None),
        }
    }

    /// The size's terms in the problem: each unknown with its
    /// coefficient.
    fn size_terms(&self) -> Result<Vec<(String, smt::Rational)>, SolveEditError> {
        self.unknowns
            .iter()
            .map(|u| Ok((u.path.clone(), exact(u.coefficient)?)))
            .collect()
    }

    /// The size the values give.
    fn size_of(&self, values: &[Rational]) -> Option<Rational> {
        let mut size = self.constant;
        for (unknown, value) in self.unknowns.iter().zip(values) {
            size = size.checked_add(unknown.coefficient.checked_mul(*value)?)?;
        }
        Some(size)
    }

    /// A value on the project's resolution, inside the unknown's range.
    fn snap(&self, index: usize, value: Rational) -> Result<Rational, SolveEditError> {
        let Some(unknown) = self.unknowns.get(index) else {
            return Err(SolveEditError::Overflow);
        };
        let overflow = || SolveEditError::Overflow;
        if unknown.listed() {
            return Ok(value);
        }
        let (step, snapped) = match unknown.current {
            Amount::Length(_) => {
                let rounded = ExactLength::new(value)
                    .round_to(self.resolution)
                    .ok_or_else(overflow)?;
                (
                    Rational::from(self.resolution.step().nm()),
                    rounded.length.to_exact().value(),
                )
            }
            Amount::Count(_) | Amount::Angle(_) => {
                (Rational::ONE, Rational::from_int(value.round()))
            }
            Amount::Ratio(_) => return Ok(value),
        };
        let Some((lowest, highest)) = unknown.bounds() else {
            return Ok(snapped);
        };
        let inside = if snapped > highest {
            snapped.checked_sub(step).ok_or_else(overflow)?
        } else if snapped < lowest {
            snapped.checked_add(step).ok_or_else(overflow)?
        } else {
            snapped
        };
        // A range narrower than a step has no value on the resolution.
        Ok(if inside < lowest || inside > highest {
            value
        } else {
            inside
        })
    }

    /// The values snapped, each changed one on its own.
    fn snap_all(&self, values: &[Rational]) -> Result<Vec<Rational>, SolveEditError> {
        values
            .iter()
            .enumerate()
            .map(|(index, value)| {
                let current = self
                    .unknowns
                    .get(index)
                    .map(|u| u.current.raw())
                    .ok_or(SolveEditError::Overflow)?;
                if *value == current {
                    Ok(*value)
                } else {
                    self.snap(index, *value)
                }
            })
            .collect()
    }

    /// The answer on the resolution, and the size it gives.
    fn snapped(
        &self,
        target: Rational,
        values: Vec<Rational>,
    ) -> Result<(Vec<Rational>, Rational), SolveEditError> {
        let snapped = self.snap_all(&values)?;
        let size = self.size_of(&snapped).ok_or(SolveEditError::Overflow)?;
        if size == target {
            return Ok((snapped, size));
        }
        let changed: Vec<usize> = (0..values.len())
            .filter(|index| {
                self.unknowns
                    .get(*index)
                    .zip(values.get(*index))
                    .is_some_and(|(u, v)| u.current.raw() != *v)
            })
            .collect();
        for free in &changed {
            let pins: BTreeMap<usize, Rational> = changed
                .iter()
                .filter(|index| *index != free)
                .filter_map(|index| Some((*index, *snapped.get(*index)?)))
                .collect();
            let Answer::Solved(again) = self.solve_at(target, &pins)? else {
                continue;
            };
            let again = self.snap_all(&again)?;
            if self.size_of(&again) == Some(target) {
                return Ok((again, target));
            }
        }
        Ok((snapped, size))
    }

    fn changes(&self, values: &[Rational]) -> Vec<ParamChange> {
        self.unknowns
            .iter()
            .zip(values)
            .filter(|(unknown, value)| unknown.current.raw() != **value)
            .map(|(unknown, value)| ParamChange {
                param: unknown.param,
                path: unknown.path.clone(),
                from: unknown.current,
                to: unknown.current.with_raw(*value),
            })
            .collect()
    }

    fn drag_values(&self, values: &[Rational], solved: bool) -> DragValues {
        DragValues {
            values: self
                .unknowns
                .iter()
                .zip(values)
                .map(|(unknown, value)| (unknown.param, unknown.current.with_raw(*value)))
                .collect(),
            solved,
        }
    }

    /// The piece through an answer `values` at `target`.
    fn piece(&self, target: Rational, values: Vec<Rational>) -> Result<Piece, SolveEditError> {
        let point = |values: Vec<Rational>| Piece {
            at: target,
            rates: vec![Rational::ZERO; values.len()],
            values,
            below: Some((Rational::ZERO, false)),
            above: Some((Rational::ZERO, false)),
        };
        let overflow = || SolveEditError::Overflow;
        let mut free = Vec::new();
        for (index, (unknown, value)) in self.unknowns.iter().zip(&values).enumerate() {
            if unknown.current.raw() == *value {
                continue;
            }
            if unknown.listed() {
                return Ok(point(values));
            }
            let (lowest, highest) = unknown.bounds().ok_or_else(overflow)?;
            if *value != lowest && *value != highest {
                free.push(index);
            }
        }
        // The tight rows: the target first, then each require on its edge.
        let margin = Rational::ONE;
        let mut matrix: Vec<Vec<Rational>> = Vec::new();
        let mut right: Vec<Rational> = Vec::new();
        matrix.push(
            free.iter()
                .map(|index| self.unknowns.get(*index).map(|u| u.coefficient))
                .collect::<Option<Vec<_>>>()
                .ok_or_else(overflow)?,
        );
        right.push(Rational::ONE);
        // Each require with room left: its coefficients along the free
        // unknowns (turned round for `>=` and `>`) and the room it has.
        let mut slack_rows: Vec<(Vec<Rational>, Rational)> = Vec::new();
        for row in &self.rows {
            let left = row_sum(row, &values).ok_or_else(overflow)?;
            let along: Vec<Rational> = free
                .iter()
                .map(|index| {
                    row.terms
                        .iter()
                        .filter(|(i, _)| i == index)
                        .try_fold(Rational::ZERO, |sum, (_, a)| sum.checked_add(*a))
                })
                .collect::<Option<Vec<_>>>()
                .ok_or_else(overflow)?;
            // Slack: how far the row is from its edge, kept at or above 0
            // (for `>=` and `>` the sign is turned round).
            let (slack, along) = match row.relation {
                smt::Relation::Equal => {
                    matrix.push(along);
                    right.push(Rational::ZERO);
                    continue;
                }
                smt::Relation::LessOrEqual => (row.constant.checked_sub(left), along),
                smt::Relation::Less => (
                    row.constant
                        .checked_sub(left)
                        .and_then(|s| s.checked_sub(margin)),
                    along,
                ),
                smt::Relation::GreaterOrEqual | smt::Relation::Greater => {
                    let extra = if row.relation == smt::Relation::Greater {
                        margin
                    } else {
                        Rational::ZERO
                    };
                    (
                        left.checked_sub(row.constant)
                            .and_then(|s| s.checked_sub(extra)),
                        along
                            .iter()
                            .map(|a| a.checked_neg())
                            .collect::<Option<Vec<_>>>()
                            .ok_or_else(overflow)?,
                    )
                }
            };
            let slack = slack.ok_or_else(overflow)?;
            if slack == Rational::ZERO {
                matrix.push(along);
                right.push(Rational::ZERO);
            } else {
                slack_rows.push((along, slack));
            }
        }
        let Some(free_rates) = solve_unique(matrix, right, free.len()) else {
            return Ok(point(values));
        };
        let mut rates = vec![Rational::ZERO; values.len()];
        for (index, rate) in free.iter().zip(&free_rates) {
            if let Some(slot) = rates.get_mut(*index) {
                *slot = *rate;
            }
        }
        let mut ends = Ends::default();
        for (index, rate) in free.iter().zip(&free_rates) {
            let (Some(unknown), Some(value)) = (self.unknowns.get(*index), values.get(*index))
            else {
                return Err(overflow());
            };
            let (lowest, highest) = unknown.bounds().ok_or_else(overflow)?;
            ends.keep_under(*value, *rate, highest)
                .ok_or_else(overflow)?;
            let turned = |v: Rational| v.checked_neg().ok_or_else(overflow);
            ends.keep_under(turned(*value)?, turned(*rate)?, turned(lowest)?)
                .ok_or_else(overflow)?;
            // Back at its starting value it no longer counts as changed.
            ends.cross(*value, *rate, unknown.current.raw())
                .ok_or_else(overflow)?;
        }
        for (along, slack) in &slack_rows {
            let rate = along
                .iter()
                .zip(&free_rates)
                .try_fold(Rational::ZERO, |sum, (a, r)| {
                    sum.checked_add(a.checked_mul(*r)?)
                })
                .ok_or_else(overflow)?;
            // The left side moves at `rate` and may use up the slack.
            ends.keep_under(Rational::ZERO, rate, *slack)
                .ok_or_else(overflow)?;
        }
        // A piece reaches only away from where the drag started, beyond
        // the target it was found at. Between the start and that target a
        // cheaper set of params, blocked at the target, can be allowed
        // again, so the piece says nothing there: a target in between is
        // solved.
        let start = self.current.raw();
        let (below, above) = if target > start {
            (Some((Rational::ZERO, false)), ends.above)
        } else if target < start {
            (ends.below, Some((Rational::ZERO, false)))
        } else {
            (Some((Rational::ZERO, false)), Some((Rational::ZERO, false)))
        };
        Ok(Piece {
            at: target,
            values,
            rates,
            below,
            above,
        })
    }
}

/// The ends of a piece as they are narrowed.
#[derive(Default)]
struct Ends {
    below: Option<(Rational, bool)>,
    above: Option<(Rational, bool)>,
}

impl Ends {
    fn narrow_above(&mut self, reach: Rational, open: bool) {
        let tighter = match self.above {
            None => true,
            Some((now, now_open)) => reach < now || (reach == now && open && !now_open),
        };
        if tighter {
            self.above = Some((reach, open));
        }
    }

    fn narrow_below(&mut self, reach: Rational, open: bool) {
        let tighter = match self.below {
            None => true,
            Some((now, now_open)) => reach < now || (reach == now && open && !now_open),
        };
        if tighter {
            self.below = Some((reach, open));
        }
    }

    /// `value + rate × step <= ceiling` must hold for every step of
    /// target in the piece.
    fn keep_under(&mut self, value: Rational, rate: Rational, ceiling: Rational) -> Option<()> {
        if rate == Rational::ZERO {
            return Some(());
        }
        let step = ceiling.checked_sub(value)?.checked_div(rate)?;
        if rate > Rational::ZERO {
            self.narrow_above(step, false);
        } else {
            self.narrow_below(step.checked_neg()?, false);
        }
        Some(())
    }

    /// A value changing at `rate` must not reach `start`.
    fn cross(&mut self, value: Rational, rate: Rational, start: Rational) -> Option<()> {
        if rate == Rational::ZERO {
            return Some(());
        }
        let step = start.checked_sub(value)?.checked_div(rate)?;
        if step > Rational::ZERO {
            self.narrow_above(step, true);
        } else if step < Rational::ZERO {
            self.narrow_below(step.checked_neg()?, true);
        }
        Some(())
    }
}

/// The left side of a row at `values`.
fn row_sum(row: &Row, values: &[Rational]) -> Option<Rational> {
    row.terms
        .iter()
        .try_fold(Rational::ZERO, |sum, (index, a)| {
            sum.checked_add(a.checked_mul(*values.get(*index)?)?)
        })
}

/// The one solution of `matrix × x = right` with `columns` unknowns, by
/// exact elimination; `None` when there is none or more than one.
fn solve_unique(
    mut matrix: Vec<Vec<Rational>>,
    mut right: Vec<Rational>,
    columns: usize,
) -> Option<Vec<Rational>> {
    let rows = matrix.len();
    let mut pivot_row = 0;
    let mut pivots = Vec::with_capacity(columns);
    for column in 0..columns {
        let found = (pivot_row..rows).find(|r| {
            matrix
                .get(*r)
                .and_then(|row| row.get(column))
                .is_some_and(|v| *v != Rational::ZERO)
        })?;
        matrix.swap(pivot_row, found);
        right.swap(pivot_row, found);
        let pivot = *matrix.get(pivot_row)?.get(column)?;
        let pivot_values = matrix.get(pivot_row)?.clone();
        let pivot_right = *right.get(pivot_row)?;
        for other in 0..rows {
            if other == pivot_row {
                continue;
            }
            let factor = matrix.get(other)?.get(column)?.checked_div(pivot)?;
            if factor == Rational::ZERO {
                continue;
            }
            let row = matrix.get_mut(other)?;
            for (cell, top) in row.iter_mut().zip(&pivot_values) {
                *cell = cell.checked_sub(factor.checked_mul(*top)?)?;
            }
            let slot = right.get_mut(other)?;
            *slot = slot.checked_sub(factor.checked_mul(pivot_right)?)?;
        }
        pivots.push((pivot_row, pivot));
        pivot_row += 1;
    }
    // Rows left over must say 0 = 0.
    if right.iter().skip(pivot_row).any(|v| *v != Rational::ZERO) {
        return None;
    }
    pivots
        .iter()
        .map(|(row, pivot)| right.get(*row)?.checked_div(*pivot))
        .collect()
}

fn relation(comparison: Comparison) -> smt::Relation {
    match comparison {
        Comparison::Equal => smt::Relation::Equal,
        Comparison::Less => smt::Relation::Less,
        Comparison::LessOrEqual => smt::Relation::LessOrEqual,
        Comparison::Greater => smt::Relation::Greater,
        Comparison::GreaterOrEqual => smt::Relation::GreaterOrEqual,
    }
}

fn exact(value: Rational) -> Result<smt::Rational, SolveEditError> {
    Ok(smt::Rational::new(value.numerator(), value.denominator())?)
}

/// A param that can be an unknown.
struct Usable {
    param: ParamId,
    path: String,
    domain: ParamDomain,
    /// Its place in declaration order: file, position, id.
    place: (u32, u32, usize),
}

/// A term's param as an unknown, when it is a solved param with a
/// `within` that a patch can set.
fn usable(model: &Model, term: &Term) -> Option<Usable> {
    let Source::Param(param) = term.dependence.source else {
        return None;
    };
    if term.dependence.freedom != Freedom::Solved || term.coefficient == Rational::ZERO {
        return None;
    }
    let domain = term.dependence.domain.clone()?;
    let path = param_path(model, param)?;
    let inst = model.params.get(param.index())?;
    let def = model.defs.param(inst.def)?;
    Some(Usable {
        param,
        path,
        domain,
        place: (def.decl.file.index(), def.decl.span.start(), param.index()),
    })
}

/// How a `set` names a param: `name` for the project's own, `assembly.name`
/// for a param of an assembly declared in the project (neither an array
/// nor a mirror image). `None` for anything else.
fn param_path(model: &Model, param: ParamId) -> Option<String> {
    let inst = model.params.get(param.index())?;
    let def = model.defs.param(inst.def)?;
    let root_instance = |instance| {
        matches!(
            model.instance(instance).map(|i| i.kind),
            Some(InstanceKind::Module(module)) if module == model.root
        )
    };
    match model.instance(inst.instance)?.kind {
        InstanceKind::Module(module) => (module == model.root).then(|| def.decl.name.clone()),
        InstanceKind::Component { assembly, .. } => {
            let assembly = model.assemblies.get(assembly.index())?;
            let plain = assembly.parent.is_none()
                && assembly.mirror_of.is_none()
                && model.defs.array(DefId::Assembly(assembly.def)).is_none()
                && root_instance(assembly.ctx.instance);
            plain.then(|| format!("{}.{}", assembly.name, def.decl.name))
        }
    }
}

/// An exact value as `.stave` text: a length in millimetres, an angle in
/// degrees, a count as a whole number, a ratio as a fraction. `None` for a
/// count that is not whole or a value too large to write.
fn value_text(amount: Amount) -> Option<String> {
    match amount {
        Amount::Length(length) => length
            .to_length()
            .or_else(|| length.to_nearest_length())
            .map(|length| length.to_string()),
        Amount::Angle(angle) => angle
            .to_angle()
            .or_else(|| angle.to_nearest_angle())
            .map(|angle| angle.to_string()),
        Amount::Count(count) => count.is_integer().then(|| count.numerator().to_string()),
        Amount::Ratio(ratio) if ratio.is_integer() => Some(ratio.numerator().to_string()),
        Amount::Ratio(ratio) => Some(format!("({} / {})", ratio.numerator(), ratio.denominator())),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use stave_lang::placement::{Axis, PartAxis};
    use stave_lang::units::Length;
    use stave_modules::DiskFiles;

    use super::*;

    /// A copy of `examples/clothing_cabinet_from_box` in a folder of its
    /// own, removed when dropped.
    struct Project {
        folder: PathBuf,
    }

    impl Drop for Project {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.folder);
        }
    }

    /// The example with its three `overall_` params made solved params
    /// with ranges, then `edit` applied to the text.
    fn project(name: &str, edit: impl Fn(String) -> String) -> Project {
        let example = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples/clothing_cabinet_from_box/cabinet.stave");
        let text = fs::read_to_string(example).unwrap();
        let text = replace(
            text,
            "param overall_width  = 940mm",
            "solved param overall_width  = 940mm within 600mm..1200mm",
        );
        let text = replace(
            text,
            "param overall_height = 1780mm",
            "solved param overall_height = 1780mm within 1500mm..2100mm",
        );
        let text = replace(
            text,
            "param overall_depth  = 580mm",
            "solved param overall_depth  = 580mm within 450mm..700mm",
        );
        let folder =
            std::env::temp_dir().join(format!("stave-solve-edit-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&folder);
        fs::create_dir_all(&folder).unwrap();
        fs::write(folder.join("cabinet.stave"), edit(text)).unwrap();
        Project { folder }
    }

    fn replace(text: String, from: &str, to: &str) -> String {
        assert!(text.contains(from), "the example no longer has `{from}`");
        text.replacen(from, to, 1)
    }

    fn mm(value: i64) -> Amount {
        Amount::Length(Length::from_mm(value).to_exact())
    }

    fn part(base: &EditBase<'_>, name: &str, axis: PartAxis) -> Quantity {
        let model = base.opened().model.as_ref().unwrap();
        let part = model
            .parts
            .iter()
            .find(|p| p.name == name)
            .unwrap_or_else(|| panic!("no part {name}"));
        Quantity::PartSize {
            part: part.id,
            axis,
        }
    }

    fn region(base: &EditBase<'_>, name: &str, axis: Axis) -> Quantity {
        let model = base.opened().model.as_ref().unwrap();
        let region = model
            .regions
            .iter()
            .find(|r| r.name == name)
            .unwrap_or_else(|| panic!("no region {name}"));
        Quantity::RegionSize {
            region: region.id,
            axis,
        }
    }

    /// The width of the top drawer's box between its sides: the sub
    /// front's length. By hand: the case's inside width (`overall_width`
    /// less two 19mm sides and the top's 20mm overhang each side), less two
    /// 12.7mm slide clearances and two 12mm box sides: 940 − 40 − 38 −
    /// 25.4 − 24 = 812.6mm. It moves one for one with `overall_width`.
    fn box_width(base: &EditBase<'_>) -> Quantity {
        part(base, "drawer[top].sub_front", PartAxis::Length)
    }

    fn tenths(value: i64) -> Amount {
        Amount::Length(Length::from_nm(value * 100_000).to_exact())
    }

    fn changes(edit: &SolvedEdit) -> Vec<(String, Amount)> {
        edit.changes
            .iter()
            .map(|change| (change.path.clone(), change.to))
            .collect()
    }

    fn solve(base: &EditBase<'_>, quantity: Quantity, target: Amount) -> SizeSolve {
        solve_size(base, quantity, target, &AtomicBool::new(false)).unwrap()
    }

    #[test]
    fn a_wider_drawer_box_widens_the_cabinet_alone() {
        let project = project("wider", |text| text);
        let base = EditBase::new(&project.folder, &DiskFiles).unwrap();
        assert_eq!(base.opened().error_count(), 0);
        let quantity = box_width(&base);
        // 862.6mm is 50mm wider, so overall_width goes from 940mm to 990mm.
        let SizeSolve::Fits(edit) = solve(&base, quantity, tenths(8626)) else {
            panic!("the wider box should fit");
        };
        assert_eq!(changes(&edit), vec![("overall_width".to_owned(), mm(990))]);
        assert_eq!(edit.reached, tenths(8626));
        assert_eq!(
            edit.patch.text,
            "stave 1\n\npatch \"Set overall_width to 990mm\" {\n  set overall_width = 990mm\n}\n"
        );
        // The scratch copy's box is the width asked for.
        let scratch = edit.tried.opened().unwrap();
        let model = scratch.model.as_ref().unwrap();
        let traced = trace(model, part_in(model, "drawer[top].sub_front")).unwrap();
        assert_eq!(traced.value, tenths(8626));
    }

    fn part_in(model: &Model, name: &str) -> Quantity {
        let part = model.parts.iter().find(|p| p.name == name).unwrap();
        Quantity::PartSize {
            part: part.id,
            axis: PartAxis::Length,
        }
    }

    #[test]
    fn a_require_that_caps_the_width_refuses_by_name() {
        let project = project("capped", |text| {
            replace(
                text,
                "region envelope =",
                "require width_cap : overall_width <= 960mm \"the cabinet must go through a 960mm door\"\n\nregion envelope =",
            )
        });
        let base = EditBase::new(&project.folder, &DiskFiles).unwrap();
        assert_eq!(base.opened().error_count(), 0);
        let quantity = box_width(&base);
        let SizeSolve::Unsatisfiable(blockers) = solve(&base, quantity, tenths(8626)) else {
            panic!("the cap should refuse the wider box");
        };
        assert_eq!(
            blockers,
            vec![
                Blocker::Require {
                    name: "width_cap".to_owned(),
                    message: "the cabinet must go through a 960mm door".to_owned(),
                },
                Blocker::Size,
            ]
        );
        // Under the cap it still works: 832.6mm needs 960mm.
        let SizeSolve::Fits(edit) = solve(&base, quantity, tenths(8326)) else {
            panic!("a box under the cap should fit");
        };
        assert_eq!(changes(&edit), vec![("overall_width".to_owned(), mm(960))]);
    }

    #[test]
    fn a_require_that_is_not_linear_is_left_to_the_scratch_check() {
        // A division by a param is not linear, so the solver doesn't see
        // it; the scratch check does, and refuses.
        let project = project("proportion", |text| {
            replace(
                text,
                "region envelope =",
                "require proportion : overall_width / overall_height <= 0.53 \"keep it tall and narrow\"\n\nregion envelope =",
            )
        });
        let base = EditBase::new(&project.folder, &DiskFiles).unwrap();
        assert_eq!(base.opened().error_count(), 0);
        let quantity = box_width(&base);
        let SizeSolve::Refused(edit) = solve(&base, quantity, tenths(8626)) else {
            panic!("the scratch check should refuse");
        };
        assert_eq!(changes(&edit), vec![("overall_width".to_owned(), mm(990))]);
        let Verdict::Refused(refusal) = edit.verdict() else {
            panic!("refused");
        };
        assert!(
            refusal
                .problems()
                .iter()
                .any(|p| p.diagnostic().message.contains("proportion")),
            "{refusal:?}"
        );
    }

    /// The example with the divider's height a project solved param, so
    /// the space above the divider (1780 − 80 raise − 500 − three 19mm
    /// boards = 1143mm) depends on two solved params, with `widths` the
    /// `within`s of `overall_height` and `drawer_height`.
    fn two_params(name: &str, height: &str, drawer: &str) -> Project {
        let height = height.to_owned();
        let drawer = drawer.to_owned();
        project(name, move |text| {
            let text = replace(
                text,
                "solved param overall_height = 1780mm within 1500mm..2100mm",
                &format!("solved param overall_height = 1780mm within {height}"),
            );
            let text = replace(
                text,
                "region envelope =",
                &format!("solved param drawer_height = 500mm within {drawer}\n\nregion envelope ="),
            );
            replace(text, "height:      500mm", "height:      drawer_height")
        })
    }

    #[test]
    fn the_param_that_moves_the_smaller_fraction_of_its_range_is_chosen() {
        // 20mm more space: 20 of overall_height's 600mm is a smaller
        // fraction than 20 of drawer_height's 200mm.
        let project = two_params("fraction-a", "1500mm..2100mm", "400mm..600mm");
        let base = EditBase::new(&project.folder, &DiskFiles).unwrap();
        assert_eq!(base.opened().error_count(), 0);
        let above = region(&base, "divider.above", Axis::Up);
        let SizeSolve::Fits(edit) = solve(&base, above, mm(1163)) else {
            panic!("should fit");
        };
        assert_eq!(
            changes(&edit),
            vec![("overall_height".to_owned(), mm(1800))]
        );

        // With the ranges the other way round, the drawers give up 20mm.
        let project = two_params("fraction-b", "1700mm..1900mm", "200mm..800mm");
        let base = EditBase::new(&project.folder, &DiskFiles).unwrap();
        assert_eq!(base.opened().error_count(), 0);
        let above = region(&base, "divider.above", Axis::Up);
        let SizeSolve::Fits(edit) = solve(&base, above, mm(1163)) else {
            panic!("should fit");
        };
        assert_eq!(changes(&edit), vec![("drawer_height".to_owned(), mm(480))]);
    }

    #[test]
    fn a_size_with_no_solved_param_falls_back() {
        let project = project("plain", |text| {
            replace(
                text,
                "solved param overall_width  = 940mm within 600mm..1200mm",
                "param overall_width  = 940mm",
            )
        });
        let base = EditBase::new(&project.folder, &DiskFiles).unwrap();
        let quantity = box_width(&base);
        let SizeSolve::Fallback(fallback) = solve(&base, quantity, tenths(8626)) else {
            panic!("should fall back");
        };
        assert_eq!(fallback.reason, FallbackReason::NoSolvedParam);
    }

    #[test]
    fn an_answer_off_the_resolution_is_snapped() {
        // 812.8mm needs overall_width 940.2mm, which is not on the 0.5mm
        // resolution: the nearest is 940mm, and no other param can make
        // up the difference.
        let project = project("snap", |text| text);
        let base = EditBase::new(&project.folder, &DiskFiles).unwrap();
        let quantity = box_width(&base);
        let SizeSolve::Unchanged = solve(&base, quantity, tenths(8128)) else {
            panic!("940.2mm snaps back to 940mm");
        };
        let SizeSolve::Fits(edit) = solve(&base, quantity, tenths(8129)) else {
            panic!("should fit");
        };
        assert_eq!(
            changes(&edit),
            vec![("overall_width".to_owned(), tenths(9405))]
        );
        assert_eq!(edit.reached, tenths(8131));
    }

    fn drag_values(drag: &mut Drag, target: Amount) -> Vec<(ParamId, Amount)> {
        match drag.values_at(target).unwrap() {
            DragStep::Values(values) => values.values,
            other => panic!("{target:?}: {other:?}"),
        }
    }

    fn solved_values(setup: &Setup, target: Amount) -> Vec<(ParamId, Amount)> {
        let Answer::Solved(values) = setup.solve_at(target.raw(), &BTreeMap::new()).unwrap() else {
            panic!("{target:?} should solve");
        };
        setup.drag_values(&values, true).values
    }

    /// Drags the space above the divider of `project` from 1143mm to
    /// `high`, then down to `low`, in 10mm steps; checks every value
    /// against a solve at that target and that a second run gives the same
    /// values; and gives the reach and how many times the drag solved.
    fn drag_across(project: &Project, low: i64, high: i64) -> (Option<(Amount, Amount)>, usize) {
        let base = EditBase::new(&project.folder, &DiskFiles).unwrap();
        assert_eq!(base.opened().error_count(), 0);
        let above = region(&base, "divider.above", Axis::Up);
        let Ready::Setup(setup) = Setup::prepare(&base, above).unwrap() else {
            panic!("two solved params");
        };
        let run = || {
            let DragStart::Ready(mut drag) = Drag::start(&base, above, mm(1153)).unwrap() else {
                panic!("two solved params");
            };
            let targets: Vec<i64> = (1143..=high)
                .step_by(10)
                .chain((low..=high).rev().step_by(10))
                .collect();
            let seen: Vec<_> = targets
                .into_iter()
                .map(|target| (target, drag_values(&mut drag, mm(target))))
                .collect();
            (seen, drag.solves(), drag.reach())
        };
        let (seen, solves, reach) = run();
        for (target, values) in &seen {
            assert_eq!(values, &solved_values(&setup, mm(*target)), "at {target}mm");
        }
        assert!(
            solves < seen.len() / 10,
            "{solves} solves for {} targets",
            seen.len()
        );
        assert_eq!(run(), (seen, solves, reach));
        (reach, solves)
    }

    #[test]
    fn a_drag_gives_the_solver_s_answer_at_every_target() {
        // overall_height reaches from 1500mm to 2100mm and drawer_height
        // from 400mm to 600mm, so the space reaches from 1143 − 280 − 100
        // to 1143 + 320 + 100. Up from the start, overall_height moves
        // until it reaches 2100mm, then the drawers give up their height;
        // down, overall_height moves to 1500mm, then the drawers grow.
        let project = two_params("drag", "1500mm..2100mm", "400mm..600mm");
        let (reach, solves) = drag_across(&project, 763, 1563);
        assert_eq!(reach, Some((mm(763), mm(1563))));
        // The start itself and four pieces.
        assert_eq!(solves, 5);
    }

    #[test]
    fn a_require_on_its_edge_ends_a_piece() {
        // A require caps overall_height at 1900mm: going up, it moves to
        // 1900mm (1263mm of space), then the drawers give way.
        let project = two_params("drag-cap", "1500mm..2100mm", "400mm..600mm");
        let text = fs::read_to_string(project.folder.join("cabinet.stave")).unwrap();
        let text = replace(
            text,
            "region envelope =",
            "require height_cap : overall_height <= 1900mm \"the ceiling is low\"\n\nregion envelope =",
        );
        fs::write(project.folder.join("cabinet.stave"), text).unwrap();
        let (reach, _) = drag_across(&project, 763, 1363);
        assert_eq!(reach, Some((mm(763), mm(1363))));
        let base = EditBase::new(&project.folder, &DiskFiles).unwrap();
        let above = region(&base, "divider.above", Axis::Up);
        let SizeSolve::Fits(edit) = solve(&base, above, mm(1283)) else {
            panic!("should fit");
        };
        assert_eq!(
            changes(&edit),
            vec![
                ("overall_height".to_owned(), mm(1900)),
                ("drawer_height".to_owned(), mm(480)),
            ]
        );
    }

    #[test]
    fn a_first_target_past_a_require_s_edge_doesn_t_reach_back() {
        // drawer_height may not go under 490mm. At 1163mm it would have to
        // be 480mm, so overall_height takes the 20mm. Back at 1148mm the
        // drawers can give 5mm again, which is 5/600 of their range
        // against 5/200 of overall_height's: the drag must solve there,
        // not carry on with overall_height from the piece found at 1163mm.
        let project = two_params("drag-back", "1700mm..1900mm", "200mm..800mm");
        let text = fs::read_to_string(project.folder.join("cabinet.stave")).unwrap();
        let text = replace(
            text,
            "region envelope =",
            "require drawer_floor : drawer_height >= 490mm \"the socks need room\"\n\nregion envelope =",
        );
        fs::write(project.folder.join("cabinet.stave"), text).unwrap();
        let base = EditBase::new(&project.folder, &DiskFiles).unwrap();
        assert_eq!(base.opened().error_count(), 0);
        let above = region(&base, "divider.above", Axis::Up);
        let Ready::Setup(setup) = Setup::prepare(&base, above).unwrap() else {
            panic!("two solved params");
        };
        let DragStart::Ready(mut drag) = Drag::start(&base, above, mm(1163)).unwrap() else {
            panic!("two solved params");
        };
        let ids: Vec<ParamId> = drag.params().map(|(id, _)| id).collect();
        let expect = |height: i64, drawer: i64| {
            ids.iter()
                .copied()
                .zip([mm(height), mm(drawer)])
                .collect::<Vec<_>>()
        };
        assert_eq!(drag.solves(), 1);
        // Further out, the piece holds: arithmetic, no solve.
        let DragStep::Values(out) = drag.values_at(mm(1173)).unwrap() else {
            panic!("1173mm should be reached");
        };
        assert!(!out.solved);
        assert_eq!(out.values, expect(1810, 500));
        assert_eq!(out.values, solved_values(&setup, mm(1173)));
        // Back towards the start, it solves.
        let DragStep::Values(back) = drag.values_at(mm(1148)).unwrap() else {
            panic!("1148mm should be reached");
        };
        assert!(back.solved);
        assert_eq!(back.values, expect(1780, 495));
        assert_eq!(drag.solves(), 2);
        // Every millimetre between the start and the first target, and a
        // little beyond, is the solver's answer.
        for target in (1133..=1183).chain((1133..=1183).rev()) {
            assert_eq!(
                drag_values(&mut drag, mm(target)),
                solved_values(&setup, mm(target)),
                "at {target}mm"
            );
        }
        // And solve_size writes what the drag showed.
        let SizeSolve::Fits(edit) = solve(&base, above, mm(1148)) else {
            panic!("should fit");
        };
        assert_eq!(changes(&edit), vec![("drawer_height".to_owned(), mm(495))]);
    }

    #[test]
    fn elimination_finds_the_one_solution_or_none() {
        let r = |value: i64| Rational::from(value);
        // x + y = 3, x − y = 1.
        assert_eq!(
            solve_unique(
                vec![vec![r(1), r(1)], vec![r(1), r(-1)]],
                vec![r(3), r(1)],
                2
            ),
            Some(vec![r(2), r(1)])
        );
        // One equation, two unknowns: more than one solution.
        assert_eq!(solve_unique(vec![vec![r(1), r(1)]], vec![r(3)], 2), None);
        // x = 1 and x = 2: none.
        assert_eq!(
            solve_unique(vec![vec![r(1)], vec![r(1)]], vec![r(1), r(2)], 1),
            None
        );
        // x = 1 said twice: one.
        assert_eq!(
            solve_unique(vec![vec![r(2)], vec![r(1)]], vec![r(2), r(1)], 1),
            Some(vec![r(1)])
        );
    }
}
