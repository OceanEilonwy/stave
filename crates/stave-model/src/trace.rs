//! The dependency trace of a size: which params, written where, a size in a
//! design comes from, and how (grammar, "Smart edits"; `docs/editing.md`,
//! "Grips").
//!
//! A [`Trace`] answers, for one [`Quantity`] of a solved design: is it
//! `constant + Σ coefficient × value` over the values it is worked out
//! from, with every other value held, and if so with which exact
//! coefficients ([`Linear`]); or which values it depends on in a way that
//! has no such form ([`NotLinear`]). Each value it depends on is a
//! [`Dependence`]: a param, or a stock's field or a kind's size, which stop
//! the chain; with what allows or stops a change ([`Freedom`]) and how many
//! component levels lie between the size and it.
//!
//! The trace itself is worked out by `stave_solve::trace`, beside the
//! placement it follows. This module holds what it returns and the pure
//! functions on it, such as single-param inversion ([`invert`]).
//!
//! Everything is exact: values are [`ExactLength`], [`ExactAngle`] and
//! [`Rational`]; there are no floats. Terms are in a fixed order (params in
//! instance order, then stock fields, then kinds' sizes), so a trace is the
//! same on every run.

use std::cmp::Ordering;

use stave_lang::placement::{Axis, PartAxis};
use stave_lang::units::{ExactAngle, ExactLength, Rational};

use crate::defs::StockField;
use crate::ids::{KindId, LetId, ParamId, PartId, RegionId, StockId};
use crate::value::Value;

/// A size in a design that the editor can drag.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Quantity {
    /// A part's drawn length, width or thickness.
    PartSize {
        /// The part.
        part: PartId,
        /// Which size.
        axis: PartAxis,
    },
    /// A region's size across, up or deep.
    RegionSize {
        /// The region.
        region: RegionId,
        /// Which size.
        axis: Axis,
    },
    /// The `gap:` of an `against` or the `inset:` of a `flush` or of
    /// `corners`, in the placement term that fixes one world axis of a
    /// part. A term that writes none has an offset of zero.
    PlaceOffset {
        /// The part.
        part: PartId,
        /// The world axis the term fixes.
        axis: Axis,
    },
    /// The value of a param: the project's, an assembly's or a
    /// component's.
    Param(ParamId),
    /// The share of one piece of a `split(..., shares: [...])`: the
    /// region is the piece.
    Share {
        /// One element of the region array the split fills.
        region: RegionId,
    },
}

/// An exact value with its dimension.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Amount {
    /// A length.
    Length(ExactLength),
    /// An angle.
    Angle(ExactAngle),
    /// A count (a whole number).
    Count(Rational),
    /// A ratio.
    Ratio(Rational),
}

impl Amount {
    /// The amount of a value, when it is a quantity.
    pub fn from_value(value: &Value) -> Option<Amount> {
        Some(match value {
            Value::Length(length) => Amount::Length(*length),
            Value::Angle(angle) => Amount::Angle(*angle),
            Value::Count(count) => Amount::Count(*count),
            Value::Ratio(ratio) => Amount::Ratio(*ratio),
            _ => return None,
        })
    }

    /// The exact number it is, in its dimension's base unit (nanometres
    /// for a length, micro-degrees for an angle). Coefficients relate
    /// these numbers.
    pub fn raw(self) -> Rational {
        match self {
            Amount::Length(length) => length.value(),
            Amount::Angle(angle) => angle.value(),
            Amount::Count(n) | Amount::Ratio(n) => n,
        }
    }

    /// An amount of the same dimension with another raw number.
    pub fn with_raw(self, raw: Rational) -> Amount {
        match self {
            Amount::Length(_) => Amount::Length(ExactLength::new(raw)),
            Amount::Angle(_) => Amount::Angle(ExactAngle::new(raw)),
            Amount::Count(_) => Amount::Count(raw),
            Amount::Ratio(_) => Amount::Ratio(raw),
        }
    }

    /// `true` when both are of the same dimension. A count and a ratio
    /// are both plain numbers and count as the same.
    pub fn same_dimension(self, other: Amount) -> bool {
        matches!(
            (self, other),
            (Amount::Length(_), Amount::Length(_))
                | (Amount::Angle(_), Amount::Angle(_))
                | (
                    Amount::Count(_) | Amount::Ratio(_),
                    Amount::Count(_) | Amount::Ratio(_)
                )
        )
    }
}

/// A value a size is worked out from.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Source {
    /// A param (the project's, or one component instance's).
    Param(ParamId),
    /// A stock's field, such as its thickness: what you buy is not changed
    /// by a drag.
    StockField {
        /// The stock.
        stock: StockId,
        /// The field.
        field: StockField,
    },
    /// A size a kind gives itself, such as a slide's clearance: not changed
    /// by a drag.
    KindSize {
        /// The kind.
        kind: KindId,
        /// The param or field of the kind.
        name: String,
    },
}

impl Source {
    fn rank(&self) -> (u8, u32, u8, &str) {
        match self {
            Source::Param(param) => (0, param.get(), 0, ""),
            Source::StockField { stock, field } => (1, stock.get(), field_rank(*field), ""),
            Source::KindSize { kind, name } => (2, kind.get(), 0, name.as_str()),
        }
    }
}

fn field_rank(field: StockField) -> u8 {
    match field {
        StockField::Species => 0,
        StockField::Form => 1,
        StockField::Thickness => 2,
        StockField::Diameter => 3,
        StockField::Width => 4,
        StockField::MaxWidth => 5,
        StockField::MaxLength => 6,
    }
}

impl PartialOrd for Source {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Source {
    fn cmp(&self, other: &Self) -> Ordering {
        self.rank().cmp(&other.rank())
    }
}

/// What allows or stops a smart edit changing a value (grammar, "Param
/// kinds" and "Smart edits").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Freedom {
    /// A plain `param`: changed when it is the single candidate or the
    /// user picks it.
    Plain,
    /// A `locked param`: never changed; the chain stops there.
    Locked,
    /// A `solved param`: the solver may pick it. Until z3 over solved
    /// params is built it is edited as a plain one.
    Solved,
    /// A stock's field: stops the chain.
    StockField,
    /// A kind's own size: stops the chain.
    KindSize,
}

impl Freedom {
    /// `true` for a value a smart edit may change.
    pub fn is_candidate(self) -> bool {
        match self {
            Freedom::Plain | Freedom::Solved => true,
            Freedom::Locked | Freedom::StockField | Freedom::KindSize => false,
        }
    }
}

/// The values a param may take, worked out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParamDomain {
    /// `low..high`, both ends included.
    Range {
        /// The smallest allowed value.
        low: Amount,
        /// The largest allowed value.
        high: Amount,
    },
    /// `[a, b, c]`
    OneOf(Vec<Amount>),
}

/// One value a size depends on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dependence {
    /// The value.
    pub source: Source,
    /// What allows or stops a change to it.
    pub freedom: Freedom,
    /// How many component levels lie between the size and where the
    /// value is declared: 0 when both are in the same instance, 1 for a
    /// project param read by a part of an assembly's component.
    pub levels: u32,
    /// Its current value.
    pub value: Amount,
    /// For a param with `within`: the values it may take.
    pub domain: Option<ParamDomain>,
}

/// One term of a [`Linear`] trace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Term {
    /// The value.
    pub dependence: Dependence,
    /// The exact coefficient: the size's raw number changes by this much
    /// for each raw unit the value changes by (see [`Amount::raw`]). For a
    /// length worked out from lengths it is a plain fraction (`1/2` for
    /// half of an opening).
    pub coefficient: Rational,
}

/// A size that is exactly `constant + Σ coefficient × value`, with every
/// other value held.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Linear {
    /// The part written as plain numbers.
    pub constant: Amount,
    /// The values, in a fixed order, each with a coefficient that is not
    /// zero.
    pub terms: Vec<Term>,
}

impl Linear {
    /// The term of a value, when the size depends on it.
    pub fn term(&self, source: &Source) -> Option<&Term> {
        self.terms.iter().find(|t| &t.dependence.source == source)
    }

    /// The coefficient of a param; zero when the size does not depend on
    /// it.
    pub fn coefficient(&self, param: ParamId) -> Rational {
        self.term(&Source::Param(param))
            .map_or(Rational::ZERO, |t| t.coefficient)
    }

    /// The size at the values' current values: `constant + Σ coefficient
    /// × value`. `None` on overflow.
    pub fn evaluate(&self) -> Option<Amount> {
        let mut raw = self.constant.raw();
        for term in &self.terms {
            raw = raw.checked_add(term.coefficient.checked_mul(term.dependence.value.raw())?)?;
        }
        Some(self.constant.with_raw(raw))
    }

    /// The same size with the values that stop the chain (stock fields
    /// and kinds' sizes) folded into the constant, leaving only params.
    /// `None` on overflow.
    pub fn holding_stops(&self) -> Option<Linear> {
        let mut constant = self.constant.raw();
        let mut terms = Vec::new();
        for term in &self.terms {
            match term.dependence.source {
                Source::Param(_) => terms.push(term.clone()),
                Source::StockField { .. } | Source::KindSize { .. } => {
                    constant = constant
                        .checked_add(term.coefficient.checked_mul(term.dependence.value.raw())?)?;
                }
            }
        }
        Some(Linear {
            constant: self.constant.with_raw(constant),
            terms,
        })
    }

    /// The params a smart edit may change, in order.
    pub fn candidates(&self) -> impl Iterator<Item = &Term> {
        self.terms
            .iter()
            .filter(|t| t.dependence.freedom.is_candidate())
    }
}

/// Why a size has no linear form in the values it depends on.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Reason {
    /// Two values that both depend on params are multiplied.
    Product,
    /// A value is divided by one that depends on params.
    Division,
    /// A `fit` that more than one part could end: which one does depends
    /// on where they are.
    Fit,
    /// A distance measured whichever way round (`between`) or around
    /// several parts (`extent`): the nearer or farther end decides it.
    Measured,
    /// Something else with no linear form, said in words.
    Other(String),
}

/// A size that is not linear as a whole. Linearity is then decided for
/// each value on its own (grammar, "Smart edits", rule 2): with every other
/// value held at its current value, the size may still be `a × p + b` in a
/// value `p` (`held`), or not (`dependences`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotLinear {
    /// Why it is not linear as a whole, in a fixed order.
    pub reasons: Vec<Reason>,
    /// The values the size is linear in with every other value held, in a
    /// fixed order, each with its coefficient worked out at the other
    /// values' current values (in `width * factor`, `width`'s coefficient
    /// is `factor`'s value). Each holds only while every other value does.
    pub held: Vec<Term>,
    /// The values the size is not linear in even with every other value
    /// held (a value times itself, a division by it, a `fit` or a measure
    /// it decides), in a fixed order.
    pub dependences: Vec<Dependence>,
}

/// How a size depends on the values it is worked out from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Form {
    /// Exactly linear.
    Linear(Linear),
    /// Not linear in these values.
    NotLinear(NotLinear),
}

/// Something the trace followed through to what it is made of: never
/// itself a candidate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Followed {
    /// A `let`.
    Let {
        /// The let.
        id: LetId,
        /// Component levels between the size and it.
        levels: u32,
    },
    /// A param whose value is an expression (its default or what an
    /// assembly gives it), followed to the values in it.
    Param {
        /// The param.
        id: ParamId,
        /// What allows or stops a change to it, were it written as a
        /// number.
        freedom: Freedom,
        /// Component levels between the size and it.
        levels: u32,
    },
}

/// The trace of one size.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Trace {
    /// What was traced.
    pub quantity: Quantity,
    /// Its current value, exact.
    pub value: Amount,
    /// How it depends on the values it is worked out from.
    pub form: Form,
    /// What the trace followed on the way, in the order it was met.
    pub followed: Vec<Followed>,
}

/// Why single-param inversion has no answer.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum InvertError {
    /// The size is not linear in the param, even with every other value
    /// held.
    #[error("the size is not worked out linearly from this param")]
    NotLinear,
    /// The size does not change with the param.
    #[error("the size does not change with this param")]
    ZeroCoefficient,
    /// The param is locked.
    #[error("the param is locked")]
    Locked,
    /// The target is not of the size's dimension.
    #[error("the target is not the same kind of value as the size")]
    Mismatch,
    /// The answer is outside the param's `within` range (the range is the
    /// param's [`Dependence::domain`]).
    #[error("the param would need to be {value:?}, outside its range")]
    OutsideRange {
        /// The value that would give the target.
        value: Amount,
    },
    /// The answer is not on the param's `within` list.
    #[error("the param would need to be {value:?}, which is not on its list")]
    NotOnList {
        /// The value that would give the target.
        value: Amount,
    },
    /// The answer for a count is not a whole number.
    #[error("the count would need to be {value:?}, which is not a whole number")]
    NotWhole {
        /// The value that would give the target.
        value: Amount,
    },
    /// The arithmetic overflowed.
    #[error("the numbers are too large")]
    Overflow,
}

/// Single-param inversion (grammar, "Smart edits", rule 2): the value of
/// `param` that makes the traced size exactly `target`, with every other
/// value held at its current value. With them held the size is `a × p +
/// b`, so the answer is `p + (target − size) / a` at the current `p` and
/// size. It is found for a param the size is linear in on its own even
/// when the size is not linear as a whole ([`NotLinear::held`]). The
/// answer is exact; writing it to a file may need rounding
/// (`ExactLength::to_length`).
pub fn invert(trace: &Trace, param: ParamId, target: Amount) -> Result<Amount, InvertError> {
    if !target.same_dimension(trace.value) {
        return Err(InvertError::Mismatch);
    }
    let source = Source::Param(param);
    let chosen = match &trace.form {
        Form::Linear(linear) => linear.term(&source),
        Form::NotLinear(not) => {
            if not.dependences.iter().any(|d| d.source == source) {
                return Err(InvertError::NotLinear);
            }
            not.held.iter().find(|t| t.dependence.source == source)
        }
    };
    let Some(chosen) = chosen else {
        return Err(InvertError::ZeroCoefficient);
    };
    if chosen.coefficient == Rational::ZERO {
        return Err(InvertError::ZeroCoefficient);
    }
    if chosen.dependence.freedom == Freedom::Locked {
        return Err(InvertError::Locked);
    }
    let change = target
        .raw()
        .checked_sub(trace.value.raw())
        .and_then(|d| d.checked_div(chosen.coefficient))
        .ok_or(InvertError::Overflow)?;
    let raw = chosen
        .dependence
        .value
        .raw()
        .checked_add(change)
        .ok_or(InvertError::Overflow)?;
    let value = chosen.dependence.value.with_raw(raw);
    if matches!(value, Amount::Count(_)) && !raw.is_integer() {
        return Err(InvertError::NotWhole { value });
    }
    match &chosen.dependence.domain {
        None => {}
        Some(ParamDomain::Range { low, high }) => {
            if raw < low.raw() || raw > high.raw() {
                return Err(InvertError::OutsideRange { value });
            }
        }
        Some(ParamDomain::OneOf(values)) if !values.iter().any(|v| v.raw() == raw) => {
            return Err(InvertError::NotOnList { value });
        }
        Some(ParamDomain::OneOf(_)) => {}
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mm(n: i64) -> Amount {
        Amount::Length(ExactLength::new(Rational::from(n * 1_000_000)))
    }

    fn param(index: usize, value: Amount, domain: Option<ParamDomain>) -> Dependence {
        Dependence {
            source: Source::Param(ParamId::from_usize(index)),
            freedom: Freedom::Plain,
            levels: 0,
            value,
            domain,
        }
    }

    fn half() -> Rational {
        Rational::new(1, 2).unwrap_or(Rational::ONE)
    }

    /// `size = width / 2 - 19mm` at width 900mm.
    fn bay(domain: Option<ParamDomain>) -> Trace {
        Trace {
            quantity: Quantity::Param(ParamId::from_usize(0)),
            value: mm(431),
            form: Form::Linear(Linear {
                constant: mm(-19),
                terms: vec![Term {
                    dependence: param(0, mm(900), domain),
                    coefficient: half(),
                }],
            }),
            followed: Vec::new(),
        }
    }

    #[test]
    fn inverts_one_param_exactly() {
        let trace = bay(None);
        assert_eq!(invert(&trace, ParamId::from_usize(0), mm(400)), Ok(mm(838)));
    }

    #[test]
    fn a_param_the_size_does_not_follow_has_no_answer() {
        let trace = bay(None);
        assert_eq!(
            invert(&trace, ParamId::from_usize(1), mm(400)),
            Err(InvertError::ZeroCoefficient)
        );
    }

    #[test]
    fn the_answer_must_be_within_the_range() {
        let trace = bay(Some(ParamDomain::Range {
            low: mm(600),
            high: mm(800),
        }));
        assert_eq!(
            invert(&trace, ParamId::from_usize(0), mm(400)),
            Err(InvertError::OutsideRange { value: mm(838) })
        );
    }

    #[test]
    fn the_answer_must_be_on_the_list() {
        let trace = bay(Some(ParamDomain::OneOf(vec![mm(800), mm(900)])));
        assert_eq!(
            invert(&trace, ParamId::from_usize(0), mm(400)),
            Err(InvertError::NotOnList { value: mm(838) })
        );
        assert_eq!(invert(&trace, ParamId::from_usize(0), mm(431)), Ok(mm(900)));
    }

    #[test]
    fn a_size_that_is_not_linear_has_no_answer() {
        let trace = Trace {
            form: Form::NotLinear(NotLinear {
                reasons: vec![Reason::Product],
                held: Vec::new(),
                dependences: vec![param(0, mm(900), None)],
            }),
            ..bay(None)
        };
        assert_eq!(
            invert(&trace, ParamId::from_usize(0), mm(400)),
            Err(InvertError::NotLinear)
        );
    }

    /// `size = width * factor + extra` at width 600mm, factor 3/2 and
    /// extra 100mm: not linear as a whole, but linear in each value with
    /// the others held.
    #[test]
    fn a_param_linear_on_its_own_inverts_in_a_size_that_is_not() {
        let factor = Amount::Ratio(Rational::new(3, 2).unwrap_or(Rational::ONE));
        let trace = Trace {
            quantity: Quantity::Param(ParamId::from_usize(3)),
            value: mm(1000),
            form: Form::NotLinear(NotLinear {
                reasons: vec![Reason::Product],
                held: vec![
                    Term {
                        dependence: param(0, mm(600), None),
                        coefficient: Rational::new(3, 2).unwrap_or(Rational::ONE),
                    },
                    Term {
                        dependence: param(1, factor, None),
                        coefficient: Rational::from(600_000_000_i64),
                    },
                    Term {
                        dependence: param(2, mm(100), None),
                        coefficient: Rational::ONE,
                    },
                ],
                dependences: vec![param(4, mm(5), None)],
            }),
            followed: Vec::new(),
        };
        assert_eq!(
            invert(&trace, ParamId::from_usize(2), mm(1100)),
            Ok(mm(200))
        );
        assert_eq!(
            invert(&trace, ParamId::from_usize(0), mm(1150)),
            Ok(mm(700))
        );
        assert_eq!(
            invert(&trace, ParamId::from_usize(1), mm(1200)),
            Ok(Amount::Ratio(Rational::new(11, 6).unwrap_or(Rational::ONE)))
        );
        assert_eq!(
            invert(&trace, ParamId::from_usize(4), mm(1100)),
            Err(InvertError::NotLinear)
        );
        assert_eq!(
            invert(&trace, ParamId::from_usize(5), mm(1100)),
            Err(InvertError::ZeroCoefficient)
        );
    }

    #[test]
    fn a_target_of_another_kind_is_refused() {
        let trace = bay(None);
        assert_eq!(
            invert(&trace, ParamId::from_usize(0), Amount::Count(Rational::ONE)),
            Err(InvertError::Mismatch)
        );
    }

    #[test]
    fn stops_fold_into_the_constant() {
        let linear = Linear {
            constant: mm(0),
            terms: vec![
                Term {
                    dependence: param(0, mm(900), None),
                    coefficient: Rational::ONE,
                },
                Term {
                    dependence: Dependence {
                        source: Source::StockField {
                            stock: StockId::from_usize(0),
                            field: StockField::Thickness,
                        },
                        freedom: Freedom::StockField,
                        levels: 0,
                        value: mm(19),
                        domain: None,
                    },
                    coefficient: Rational::from(-2_i64),
                },
            ],
        };
        let held = linear.holding_stops().unwrap();
        assert_eq!(held.constant, mm(-38));
        assert_eq!(held.terms.len(), 1);
        assert_eq!(held.evaluate(), linear.evaluate());
        assert_eq!(linear.evaluate(), Some(mm(862)));
    }
}
