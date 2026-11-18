//! The z3 encoding. Nothing in this module is public: z3 types never leave
//! the crate.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant;

use z3::ast::{Bool, Real};
use z3::{Model, Optimize, Params, SatResult, Solver};

use crate::{Conflict, Domain, Outcome, Problem, Rational, Reach, Relation, SmtError, Solution};

/// Why a solve stopped early: out of time, or an error.
enum Halt {
    TimedOut,
    Failed(SmtError),
}

impl From<SmtError> for Halt {
    fn from(error: SmtError) -> Halt {
        Halt::Failed(error)
    }
}

/// An unknown after checking, in canonical position.
struct CheckedUnknown {
    name: String,
    current: Rational,
    domain: Domain,
    rank: u32,
    /// `1 / width` of the domain, or 1 when the domain has no width.
    weight: Rational,
}

/// A constraint after checking: terms merged by unknown, in canonical order,
/// with zero coefficients dropped.
struct CheckedConstraint {
    label: String,
    terms: Vec<(usize, Rational)>,
    relation: Relation,
    constant: Rational,
}

/// The problem in a canonical form that does not depend on the order the
/// caller added things in: unknowns by declaration order, constraints by
/// label.
struct Checked {
    unknowns: Vec<CheckedUnknown>,
    constraints: Vec<CheckedConstraint>,
    strict_margin: Rational,
}

pub(crate) fn solve(problem: &Problem) -> Result<Outcome, SmtError> {
    let deadline = Instant::now().checked_add(problem.time_limit);
    let checked = check(problem)?;
    match run(&checked, deadline) {
        Ok(outcome) => Ok(outcome),
        Err(Halt::TimedOut) => Ok(Outcome::TimedOut),
        Err(Halt::Failed(error)) => Err(error),
    }
}

pub(crate) fn reach(problem: &Problem, terms: &[(String, Rational)]) -> Result<Reach, SmtError> {
    let deadline = Instant::now().checked_add(problem.time_limit);
    let checked = check(problem)?;
    let mut indexed = Vec::with_capacity(terms.len());
    for (name, coefficient) in terms {
        let index = checked
            .unknowns
            .iter()
            .position(|unknown| &unknown.name == name)
            .ok_or_else(|| SmtError::UndeclaredUnknown {
                constraint: "the sum whose reach is asked for".into(),
                unknown: name.clone(),
            })?;
        indexed.push((index, *coefficient));
    }
    match run_reach(&checked, &indexed, deadline) {
        Ok(reach) => Ok(reach),
        Err(Halt::TimedOut) => Ok(Reach::TimedOut),
        Err(Halt::Failed(error)) => Err(error),
    }
}

fn run_reach(
    checked: &Checked,
    terms: &[(usize, Rational)],
    deadline: Option<Instant>,
) -> Result<Reach, Halt> {
    let encoding = encode(checked)?;
    if let Some(conflicts) = find_conflict(checked, &encoding, deadline)? {
        return Ok(Reach::Unsatisfiable(conflicts));
    }
    let products = terms
        .iter()
        .map(|(index, coefficient)| {
            let value = encoding
                .values
                .get(*index)
                .ok_or(SmtError::Solver("an unknown lost its variables".into()))?;
            Ok(Real::mul(&[numeral(*coefficient)?, value.clone()]))
        })
        .collect::<Result<Vec<Real>, SmtError>>()?;
    let total = sum(&products)?;
    let mut ends = Vec::with_capacity(2);
    for lowest in [true, false] {
        let optimize = Optimize::new();
        for formula in encoding.domains.iter().chain(&encoding.constraints) {
            optimize.assert(formula);
        }
        if lowest {
            optimize.minimize(&total);
        } else {
            optimize.maximize(&total);
        }
        optimize.set_params(&time_left(deadline)?);
        let model = match optimize.check(&[]) {
            SatResult::Sat => optimize
                .get_model()
                .ok_or(Halt::Failed(SmtError::Solver("z3 gave no answer".into())))?,
            SatResult::Unsat => {
                return Err(Halt::Failed(SmtError::Solver(
                    "a problem that was satisfiable stopped being so while it was optimised".into(),
                )));
            }
            SatResult::Unknown => return Err(unknown_result(optimize.get_reason_unknown())),
        };
        let evaluated = model
            .eval(&total, true)
            .ok_or(SmtError::Solver("z3 gave no value for the sum".into()))?;
        ends.push(parse_numeral(&evaluated.to_string())?);
    }
    match ends.as_slice() {
        [lowest, highest] => Ok(Reach::Between {
            lowest: *lowest,
            highest: *highest,
        }),
        _ => Err(Halt::Failed(SmtError::Solver(
            "the reach lost one of its ends".into(),
        ))),
    }
}

fn check(problem: &Problem) -> Result<Checked, SmtError> {
    if problem.strict_margin <= Rational::ZERO {
        return Err(SmtError::NonPositiveMargin(problem.strict_margin));
    }
    let mut by_order: BTreeMap<u32, &crate::Unknown> = BTreeMap::new();
    let mut names = BTreeSet::new();
    for unknown in &problem.unknowns {
        if !names.insert(unknown.name.as_str()) {
            return Err(SmtError::DuplicateUnknown(unknown.name.clone()));
        }
        if let Some(other) = by_order.insert(unknown.order, unknown) {
            return Err(SmtError::DuplicateOrder {
                order: unknown.order,
                first: other.name.clone(),
                second: unknown.name.clone(),
            });
        }
    }
    let mut unknowns = Vec::with_capacity(by_order.len());
    let mut index_of: BTreeMap<&str, usize> = BTreeMap::new();
    for unknown in by_order.values() {
        let domain = match &unknown.domain {
            Domain::Range { lowest, highest } => Domain::range(*lowest, *highest)?,
            Domain::Values(values) => Domain::values(values.iter().copied())?,
        };
        let (lowest, highest) = domain.bounds().ok_or(SmtError::EmptyDomain)?;
        let width = highest.checked_sub(lowest)?;
        let weight = width.reciprocal().unwrap_or(Rational::ONE);
        index_of.insert(unknown.name.as_str(), unknowns.len());
        unknowns.push(CheckedUnknown {
            name: unknown.name.clone(),
            current: unknown.current,
            domain,
            rank: unknown.rank,
            weight,
        });
    }
    let mut constraints = Vec::with_capacity(problem.constraints.len());
    let mut labels = BTreeSet::new();
    for constraint in &problem.constraints {
        if !labels.insert(constraint.label.as_str()) {
            return Err(SmtError::DuplicateConstraint(constraint.label.clone()));
        }
        // Coefficients of one unknown are summed in z3, not here, so that no
        // sum can overflow; here they are only grouped.
        let mut grouped: BTreeMap<usize, Vec<Rational>> = BTreeMap::new();
        for (name, coefficient) in &constraint.terms {
            let index =
                *index_of
                    .get(name.as_str())
                    .ok_or_else(|| SmtError::UndeclaredUnknown {
                        constraint: constraint.label.clone(),
                        unknown: name.clone(),
                    })?;
            if !coefficient.is_zero() {
                grouped.entry(index).or_default().push(*coefficient);
            }
        }
        let terms = grouped
            .into_iter()
            .flat_map(|(index, coefficients)| {
                coefficients
                    .into_iter()
                    .map(move |coefficient| (index, coefficient))
            })
            .collect();
        constraints.push(CheckedConstraint {
            label: constraint.label.clone(),
            terms,
            relation: constraint.relation,
            constant: constraint.constant,
        });
    }
    constraints.sort_by(|left, right| left.label.cmp(&right.label));
    Ok(Checked {
        unknowns,
        constraints,
        strict_margin: problem.strict_margin,
    })
}

/// A z3 numeral for an exact rational.
fn numeral(value: Rational) -> Result<Real, SmtError> {
    Real::from_rational_str(
        &value.numerator().to_string(),
        &value.denominator().to_string(),
    )
    .ok_or(SmtError::Solver(format!(
        "z3 did not accept the number {value}"
    )))
}

/// The sum of `terms`, or the numeral zero when there are none.
fn sum(terms: &[Real]) -> Result<Real, SmtError> {
    if terms.is_empty() {
        numeral(Rational::ZERO)
    } else {
        Ok(Real::add(terms))
    }
}

/// The z3 variables and formulas for a checked problem.
struct Encoding {
    /// The value of each unknown.
    values: Vec<Real>,
    /// Whether each unknown may leave its current value.
    changed: Vec<Bool>,
    /// How far each unknown moved, at least.
    distances: Vec<Real>,
    /// Each unknown's domain as a formula.
    domains: Vec<Bool>,
    /// Each constraint as a formula, in canonical order.
    constraints: Vec<Bool>,
}

fn encode(checked: &Checked) -> Result<Encoding, SmtError> {
    let mut values = Vec::with_capacity(checked.unknowns.len());
    let mut changed = Vec::with_capacity(checked.unknowns.len());
    let mut distances = Vec::with_capacity(checked.unknowns.len());
    let mut domains = Vec::with_capacity(checked.unknowns.len());
    for (index, unknown) in checked.unknowns.iter().enumerate() {
        let value = Real::new_const(format!("value_{index}"));
        let domain = match &unknown.domain {
            Domain::Range { lowest, highest } => {
                Bool::and(&[value.ge(numeral(*lowest)?), value.le(numeral(*highest)?)])
            }
            Domain::Values(allowed) => {
                let choices = allowed
                    .iter()
                    .map(|allowed| Ok(value.eq(numeral(*allowed)?)))
                    .collect::<Result<Vec<Bool>, SmtError>>()?;
                Bool::or(&choices)
            }
        };
        values.push(value);
        changed.push(Bool::new_const(format!("changed_{index}")));
        distances.push(Real::new_const(format!("distance_{index}")));
        domains.push(domain);
    }
    let mut constraints = Vec::with_capacity(checked.constraints.len());
    for constraint in &checked.constraints {
        let products = constraint
            .terms
            .iter()
            .map(|(index, coefficient)| {
                let value = values.get(*index).ok_or(SmtError::Solver(format!(
                    "constraint {} names an unknown that was lost",
                    constraint.label
                )))?;
                Ok(Real::mul(&[numeral(*coefficient)?, value.clone()]))
            })
            .collect::<Result<Vec<Real>, SmtError>>()?;
        let left = sum(&products)?;
        let constant = numeral(constraint.constant)?;
        let margin = numeral(checked.strict_margin)?;
        let formula = match constraint.relation {
            Relation::Equal => left.eq(constant),
            Relation::LessOrEqual => left.le(constant),
            Relation::Less => Real::add(&[left, margin]).le(constant),
            Relation::GreaterOrEqual => left.ge(constant),
            Relation::Greater => left.ge(Real::add(&[constant, margin])),
        };
        constraints.push(formula);
    }
    Ok(Encoding {
        values,
        changed,
        distances,
        domains,
        constraints,
    })
}

fn time_left(deadline: Option<Instant>) -> Result<Params, Halt> {
    let mut params = Params::new();
    if let Some(deadline) = deadline {
        let left = deadline.saturating_duration_since(Instant::now());
        let milliseconds = u32::try_from(left.as_millis()).unwrap_or(u32::MAX);
        if milliseconds == 0 {
            return Err(Halt::TimedOut);
        }
        params.set_u32("timeout", milliseconds);
    }
    Ok(params)
}

fn unknown_result(reason: Option<String>) -> Halt {
    let reason = reason.unwrap_or_default();
    if reason.contains("timeout") || reason.contains("canceled") || reason.contains("cancelled") {
        Halt::TimedOut
    } else {
        Halt::Failed(SmtError::Solver(format!(
            "z3 could not decide the problem: {reason}"
        )))
    }
}

fn run(checked: &Checked, deadline: Option<Instant>) -> Result<Outcome, Halt> {
    let encoding = encode(checked)?;
    if let Some(conflicts) = find_conflict(checked, &encoding, deadline)? {
        return Ok(Outcome::Unsatisfiable(conflicts));
    }
    optimise(checked, &encoding, deadline)
}

/// Checks the domains and constraints alone. When they cannot all hold,
/// returns a smallest conflicting set, found by trying to drop each item in
/// a fixed order (domains by declaration order, then constraints by label),
/// so the set depends on the problem only, never on z3's search.
fn find_conflict(
    checked: &Checked,
    encoding: &Encoding,
    deadline: Option<Instant>,
) -> Result<Option<Vec<Conflict>>, Halt> {
    let solver = Solver::new();
    let mut items: Vec<(Conflict, Bool)> = Vec::new();
    let formulas = encoding.domains.iter().chain(encoding.constraints.iter());
    let names = checked
        .unknowns
        .iter()
        .map(|unknown| Conflict::Domain(unknown.name.clone()))
        .chain(
            checked
                .constraints
                .iter()
                .map(|constraint| Conflict::Constraint(constraint.label.clone())),
        );
    for (position, (conflict, formula)) in names.zip(formulas).enumerate() {
        let switch = Bool::new_const(format!("item_{position}"));
        solver.assert(switch.implies(formula));
        items.push((conflict, switch));
    }
    let holds = |switches: &[Bool]| -> Result<bool, Halt> {
        solver.set_params(&time_left(deadline)?);
        match solver.check_assumptions(switches) {
            SatResult::Sat => Ok(true),
            SatResult::Unsat => Ok(false),
            SatResult::Unknown => Err(unknown_result(solver.get_reason_unknown())),
        }
    };
    let all: Vec<Bool> = items.iter().map(|(_, switch)| switch.clone()).collect();
    if holds(&all)? {
        return Ok(None);
    }
    // Deletion: keep an item only if the rest cannot conflict without it.
    let mut kept = vec![true; items.len()];
    for position in 0..items.len() {
        if let Some(slot) = kept.get_mut(position) {
            *slot = false;
        }
        let trial: Vec<Bool> = items
            .iter()
            .zip(&kept)
            .filter(|(_, keep)| **keep)
            .map(|((_, switch), _)| switch.clone())
            .collect();
        if holds(&trial)?
            && let Some(slot) = kept.get_mut(position)
        {
            *slot = true;
        }
    }
    let mut conflicts: Vec<Conflict> = items
        .into_iter()
        .zip(kept)
        .filter(|(_, keep)| *keep)
        .map(|((conflict, _), _)| conflict)
        .collect();
    conflicts.sort();
    Ok(Some(conflicts))
}

/// Minimises `keys` lexicographically (each at its least value among the
/// answers least on the keys before it) and returns the model that reaches
/// all of them.
fn minimise_lexicographically(
    optimize: &Optimize,
    keys: &[Real],
    deadline: Option<Instant>,
) -> Result<Model, Halt> {
    for key in keys {
        optimize.minimize(key);
    }
    let mut params = time_left(deadline)?;
    params.set_symbol("priority", "lex");
    optimize.set_params(&params);
    match optimize.check(&[]) {
        SatResult::Sat => optimize
            .get_model()
            .ok_or(Halt::Failed(SmtError::Solver("z3 gave no answer".into()))),
        SatResult::Unsat => Err(Halt::Failed(SmtError::Solver(
            "a problem that was satisfiable stopped being so while it was optimised".into(),
        ))),
        SatResult::Unknown => Err(unknown_result(optimize.get_reason_unknown())),
    }
}

fn optimise(
    checked: &Checked,
    encoding: &Encoding,
    deadline: Option<Instant>,
) -> Result<Outcome, Halt> {
    let optimize = Optimize::new();
    for formula in encoding.domains.iter().chain(&encoding.constraints) {
        optimize.assert(formula);
    }
    let zero = numeral(Rational::ZERO)?;
    let one = numeral(Rational::ONE)?;
    let mut counts = Vec::with_capacity(checked.unknowns.len());
    let mut fractions = Vec::with_capacity(checked.unknowns.len());
    let mut ranks = Vec::with_capacity(checked.unknowns.len());
    for (index, unknown) in checked.unknowns.iter().enumerate() {
        let (Some(value), Some(changed), Some(distance)) = (
            encoding.values.get(index),
            encoding.changed.get(index),
            encoding.distances.get(index),
        ) else {
            return Err(SmtError::Solver("an unknown lost its variables".into()).into());
        };
        let current = numeral(unknown.current)?;
        // An unknown not marked changed keeps its current value.
        optimize.assert(changed.not().implies(value.eq(&current)));
        // The distance is at least the absolute movement; minimising the
        // fraction sum makes it exactly that.
        optimize.assert(distance.ge(Real::sub(&[value, &current])));
        optimize.assert(distance.ge(Real::sub(&[&current, value])));
        counts.push(changed.ite(&one, &zero));
        fractions.push(Real::mul(&[numeral(unknown.weight)?, distance.clone()]));
        let rank = numeral(Rational::from_integer(i128::from(unknown.rank)))?;
        ranks.push(changed.ite(&rank, &zero));
    }
    // The keys, most important first; z3 minimises them lexicographically.
    // 1. How many unknowns change.
    // 2. How far they move, each as a fraction of its domain's width.
    // 3. The total rank of the unknowns that change.
    let mut keys = vec![sum(&counts)?, sum(&fractions)?, sum(&ranks)?];
    // 4. Declaration order: the last-declared unknown moves least, then the
    //    one before it, so earlier unknowns take the change.
    keys.extend(encoding.distances.iter().rev().cloned());
    // 5. An unknown limited to a list could still sit the same distance
    //    above or below its current value: take the lower, in order.
    //
    // After that every unknown has one possible value, so the model is the
    // answer whichever way z3 reached it. An unknown with a range needs no
    // step 5: if two answers tied on every key above and agreed on every
    // listed value, their midpoint would keep every domain and constraint
    // and move strictly less, beating them on key 2.
    for (unknown, value) in checked.unknowns.iter().zip(&encoding.values) {
        match unknown.domain {
            Domain::Values(_) => keys.push(value.clone()),
            Domain::Range { .. } => {}
        }
    }
    let model = minimise_lexicographically(&optimize, &keys, deadline)?;
    let mut answers = Vec::with_capacity(checked.unknowns.len());
    for (unknown, value) in checked.unknowns.iter().zip(&encoding.values) {
        let evaluated = model.eval(value, true).ok_or(SmtError::Solver(format!(
            "z3 gave no value for {}",
            unknown.name
        )))?;
        let exact = parse_numeral(&evaluated.to_string())?;
        answers.push((unknown.name.clone(), unknown.current, exact));
    }
    Ok(Outcome::Solved(Solution::new(answers)))
}

/// Reads a z3 real numeral as printed: `5.0`, `1.25`, `(- 5.0)`,
/// `(/ 3.0 2.0)` or `(- (/ 3.0 2.0))`.
fn parse_numeral(text: &str) -> Result<Rational, SmtError> {
    let unreadable = || SmtError::UnreadableValue(text.to_owned());
    let text = text.trim();
    if let Some(inner) = text
        .strip_prefix("(-")
        .and_then(|rest| rest.strip_suffix(')'))
    {
        let value = parse_numeral(inner)?;
        let negated = value.numerator().checked_neg().ok_or(SmtError::Overflow)?;
        return Rational::new(negated, value.denominator());
    }
    if let Some(inner) = text
        .strip_prefix("(/")
        .and_then(|rest| rest.strip_suffix(')'))
    {
        let mut parts = inner.split_whitespace();
        let (Some(top), Some(bottom), None) = (parts.next(), parts.next(), parts.next()) else {
            return Err(unreadable());
        };
        let top = parse_decimal(top).ok_or_else(unreadable)?;
        let bottom = parse_decimal(bottom).ok_or_else(unreadable)?;
        let numerator = top
            .numerator()
            .checked_mul(bottom.denominator())
            .ok_or(SmtError::Overflow)?;
        let denominator = top
            .denominator()
            .checked_mul(bottom.numerator())
            .ok_or(SmtError::Overflow)?;
        return Rational::new(numerator, denominator);
    }
    parse_decimal(text).ok_or_else(unreadable)
}

/// Reads an unsigned decimal such as `5`, `5.0` or `1.25` exactly.
fn parse_decimal(text: &str) -> Option<Rational> {
    let (whole, fraction) = text.split_once('.').unwrap_or((text, ""));
    if whole.is_empty() || !whole.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    if !fraction.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let digits = format!("{whole}{fraction}");
    let numerator: i128 = digits.parse().ok()?;
    let exponent = u32::try_from(fraction.len()).ok()?;
    let denominator = 10_i128.checked_pow(exponent)?;
    Rational::new(numerator, denominator).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fraction(numerator: i128, denominator: i128) -> Rational {
        match Rational::new(numerator, denominator) {
            Ok(value) => value,
            Err(error) => panic!("{error}"),
        }
    }

    #[test]
    fn reads_every_numeral_form() {
        assert_eq!(parse_numeral("5.0"), Ok(fraction(5, 1)));
        assert_eq!(parse_numeral("1.25"), Ok(fraction(5, 4)));
        assert_eq!(parse_numeral("(- 5.0)"), Ok(fraction(-5, 1)));
        assert_eq!(parse_numeral("(/ 3.0 2.0)"), Ok(fraction(3, 2)));
        assert_eq!(parse_numeral("(- (/ 3.0 2.0))"), Ok(fraction(-3, 2)));
        assert!(parse_numeral("(root-obj x 1)").is_err());
        assert!(parse_numeral("epsilon").is_err());
    }

    #[test]
    fn reads_what_z3_prints() {
        for value in [
            fraction(3, 2),
            fraction(-3, 2),
            fraction(0, 1),
            fraction(-7, 1),
            fraction(1_000_000_000_000_000_001, 3),
        ] {
            let printed = match numeral(value) {
                Ok(real) => real.to_string(),
                Err(error) => panic!("{error}"),
            };
            assert_eq!(parse_numeral(&printed), Ok(value), "{printed}");
        }
    }
}
