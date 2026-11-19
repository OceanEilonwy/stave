//! The problem a caller builds: unknowns, their domains, and linear
//! constraints over them.

use std::time::Duration;

use crate::{Rational, SmtError};

/// The values an unknown may take.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Domain {
    /// Every value from `lowest` to `highest`, both included.
    Range {
        /// The smallest allowed value.
        lowest: Rational,
        /// The largest allowed value.
        highest: Rational,
    },
    /// Only these values: never empty, sorted from smallest, no repeats.
    Values(Vec<Rational>),
}

impl Domain {
    /// Every value from `lowest` to `highest`, both included.
    ///
    /// # Errors
    ///
    /// [`SmtError::InvertedRange`] when `lowest` is greater than `highest`.
    pub fn range(lowest: Rational, highest: Rational) -> Result<Domain, SmtError> {
        if lowest > highest {
            return Err(SmtError::InvertedRange { lowest, highest });
        }
        Ok(Domain::Range { lowest, highest })
    }

    /// Only the listed values, in any order and with repeats allowed; they
    /// are sorted and repeats removed.
    ///
    /// # Errors
    ///
    /// [`SmtError::EmptyDomain`] when the list is empty.
    pub fn values(values: impl IntoIterator<Item = Rational>) -> Result<Domain, SmtError> {
        let mut values: Vec<Rational> = values.into_iter().collect();
        values.sort_unstable();
        values.dedup();
        if values.is_empty() {
            return Err(SmtError::EmptyDomain);
        }
        Ok(Domain::Values(values))
    }

    /// The smallest and largest allowed values, or `None` for an empty list
    /// (which [`Domain::values`] never makes, but the variant's field is
    /// public).
    #[must_use]
    pub fn bounds(&self) -> Option<(Rational, Rational)> {
        match self {
            Domain::Range { lowest, highest } => Some((*lowest, *highest)),
            Domain::Values(values) => Some((*values.iter().min()?, *values.iter().max()?)),
        }
    }
}

/// One value the solver may change.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unknown {
    /// The caller's name for it, unique in the problem. Solutions and
    /// conflicts name unknowns by it.
    pub name: String,
    /// The value it has now; the solver prefers to leave it there.
    pub current: Rational,
    /// The values it may take. The current value need not be among them.
    pub domain: Domain,
    /// How far it is from the thing the user dragged (0 nearest). Among
    /// answers that change equally many unknowns by equal fractions, the one
    /// whose changed unknowns have the smallest total rank wins.
    pub rank: u32,
    /// Its place in the caller's declaration order, unique in the problem.
    /// The last tiebreak: an unknown earlier in this order is changed in
    /// preference to a later one. The order unknowns are added in does not
    /// matter; only this number does.
    pub order: u32,
}

/// How the two sides of a [`Constraint`] compare.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Relation {
    /// The sum equals the constant.
    Equal,
    /// The sum is at most the constant.
    LessOrEqual,
    /// The sum is below the constant by at least the problem's strict margin.
    Less,
    /// The sum is at least the constant.
    GreaterOrEqual,
    /// The sum is above the constant by at least the problem's strict margin.
    Greater,
}

/// A linear constraint: the sum of `coefficient × unknown` over its terms,
/// compared with a constant.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Constraint {
    /// The caller's name for it, unique in the problem; an unsatisfiable
    /// problem names its conflicting constraints by it.
    pub label: String,
    /// `(unknown name, coefficient)` pairs. An unknown may appear more than
    /// once; its coefficients are added.
    pub terms: Vec<(String, Rational)>,
    /// How the sum compares with `constant`.
    pub relation: Relation,
    /// The right-hand side.
    pub constant: Rational,
}

impl Constraint {
    /// A constraint `Σ coefficient × unknown  relation  constant`.
    pub fn new<Name: Into<String>>(
        label: impl Into<String>,
        terms: impl IntoIterator<Item = (Name, Rational)>,
        relation: Relation,
        constant: Rational,
    ) -> Constraint {
        Constraint {
            label: label.into(),
            terms: terms
                .into_iter()
                .map(|(name, coefficient)| (name.into(), coefficient))
                .collect(),
            relation,
            constant,
        }
    }
}

/// A whole problem: unknowns, constraints, the margin that makes strict
/// inequalities exact, and the time the solver may take.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Problem {
    pub(crate) unknowns: Vec<Unknown>,
    pub(crate) constraints: Vec<Constraint>,
    pub(crate) strict_margin: Rational,
    pub(crate) time_limit: Duration,
}

impl Default for Problem {
    fn default() -> Problem {
        Problem::new()
    }
}

impl Problem {
    /// The time limit a new problem has: ten seconds.
    pub const DEFAULT_TIME_LIMIT: Duration = Duration::from_secs(10);

    /// An empty problem with a strict margin of 1 and the default time limit.
    #[must_use]
    pub fn new() -> Problem {
        Problem {
            unknowns: Vec::new(),
            constraints: Vec::new(),
            strict_margin: Rational::ONE,
            time_limit: Problem::DEFAULT_TIME_LIMIT,
        }
    }

    /// Adds an unknown. Names and orders are checked when solving.
    pub fn add_unknown(&mut self, unknown: Unknown) -> &mut Problem {
        self.unknowns.push(unknown);
        self
    }

    /// Adds a constraint. Labels and the unknowns it names are checked when
    /// solving.
    pub fn add_constraint(&mut self, constraint: Constraint) -> &mut Problem {
        self.constraints.push(constraint);
        self
    }

    /// Sets the strict margin: `sum < constant` is solved as
    /// `sum + margin <= constant`, and `>` likewise, so that every problem
    /// has a closest answer instead of an endless approach to a bound. In
    /// the unknowns' own unit (with lengths in nanometres, 1 is one
    /// nanometre, the default). It must be positive; that is checked when
    /// solving.
    pub fn set_strict_margin(&mut self, margin: Rational) -> &mut Problem {
        self.strict_margin = margin;
        self
    }

    /// Sets how long [`Problem::solve`] may take in all before it gives up
    /// with [`Outcome::TimedOut`](crate::Outcome::TimedOut).
    pub fn set_time_limit(&mut self, limit: Duration) -> &mut Problem {
        self.time_limit = limit;
        self
    }

    /// The unknowns, in the order they were added.
    #[must_use]
    pub fn unknowns(&self) -> &[Unknown] {
        &self.unknowns
    }

    /// The constraints, in the order they were added.
    #[must_use]
    pub fn constraints(&self) -> &[Constraint] {
        &self.constraints
    }

    /// The strict margin.
    #[must_use]
    pub fn strict_margin(&self) -> Rational {
        self.strict_margin
    }

    /// The time limit.
    #[must_use]
    pub fn time_limit(&self) -> Duration {
        self.time_limit
    }
}
