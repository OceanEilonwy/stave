//! The solver behind Stave's smart edits, and the only crate that may depend
//! on z3.
//!
//! A smart edit asks: which values, changed as little as possible, give the
//! size the user dragged to? (`docs/grammar.md`, "Smart edits".) This crate
//! answers that one kind of question and nothing else, behind a small typed
//! API in which no z3 type appears. It knows nothing about params, parts or
//! lengths: its callers turn those into a [`Problem`]. It depends on no other
//! Stave crate, so it sits at the bottom of the dependency order.
//!
//! # The problem
//!
//! - **Unknowns** ([`Unknown`]): each an exact [`Rational`] with a current
//!   value, a [`Domain`] (a closed range or a finite list of allowed values),
//!   a rank (its distance from the thing dragged) and a place in declaration
//!   order.
//! - **Constraints** ([`Constraint`]): `Σ coefficient × unknown` compared
//!   with a constant by `=`, `<=`, `<`, `>=` or `>` ([`Relation`]), every
//!   number exact. A strict inequality is solved with the problem's strict
//!   margin: `sum < constant` as `sum + margin <= constant`. That keeps every
//!   problem's best answer a real answer rather than a value approached but
//!   never reached; with lengths in nanometres the default margin of 1 is one
//!   nanometre.
//!
//! # The answer
//!
//! [`Problem::solve`] gives an [`Outcome`]:
//!
//! - [`Outcome::Solved`]: an exact value for every unknown. Among all answers
//!   that keep every domain and constraint, it is the least by, in order:
//!   1. how many unknowns differ from their current value;
//!   2. the sum over unknowns of how far each moved as a fraction of its
//!      domain's width (lowest to highest allowed value; a domain with no
//!      width counts its movement as is);
//!   3. the sum of the ranks of the unknowns that changed, so changes go to
//!      low ranks first;
//!   4. declaration order: the last-declared unknown moves as little as it
//!      can, then the one before it, and so on, so the change goes to the
//!      earliest-declared unknown;
//!   5. when an unknown restricted to a list could still sit the same
//!      distance above or below its current value, the lower value.
//! - [`Outcome::Unsatisfiable`]: the domains and constraints cannot all
//!   hold. It names a smallest set of them that conflict ([`Conflict`]):
//!   removing any one of the named items would leave the rest satisfiable.
//! - [`Outcome::TimedOut`]: the problem's time limit ran out.
//!
//! # Determinism
//!
//! The same problem gives the same answer on every run and every machine,
//! whatever order its unknowns, constraints and terms were added in. The
//! crate puts the problem in a canonical order first (unknowns by
//! declaration order, constraints by label, terms by unknown) and then has
//! z3 minimise the keys above lexicographically: each key at its least value
//! among the answers least on the keys before it (key 4 is one key per
//! unknown, the last-declared first; key 5 one per listed unknown). The
//! ordering is total, so after the last key exactly one answer is left,
//! whichever path z3's search took to it. The conflict set is
//! found the same way: each item is tried for removal in a fixed order,
//! domains first, so it does not depend on z3's search either. Only
//! [`Outcome::TimedOut`] depends on the machine.
//!
//! # Building z3
//!
//! z3 comes from the `z3` crate (0.23) with its `bundled` feature (the
//! successor of the old `static-link-z3` feature), which builds z3 5.4.1
//! (the `z3-src` crate) from C++ source with CMake and links it statically;
//! no system `libz3` is used, and nothing is downloaded at build time beyond
//! the crates themselves. The build needs `cmake`, a C++ compiler and
//! `clang` (bindgen uses libclang). Do not use the `gh-release` feature: it
//! downloads a prebuilt library.
//!
//! On the owner's machine (32 threads, 2026-11-18) the first
//! `cargo build -p stave-smt` took 4 minutes 49 seconds of wall time (about
//! 70 minutes of CPU time), almost all of it compiling z3, and left about
//! 4.4 GB in `target/`. The result is cached in the target directory, so
//! later builds, clippy and tests in the same target directory do not pay it
//! again; every fresh target directory (each worktree's own `target/`, or a
//! new `CARGO_TARGET_DIR`) pays it once more, and a `cargo clean` throws it
//! away. Parallel agents each building z3 at once will each be much slower.
//!
//! In the `dev` and `test` profiles the `cmake` crate builds z3 as CMake's
//! `Debug` (unoptimised, with z3's internal assertions), because it follows
//! the opt-level of the `z3-sys` package. Solving is then slow: about 100 ms
//! for a problem of six unknowns, and the determinism test takes several
//! seconds (an optimised z3 should be many times faster; that was not
//! measured here). The cure is a profile override in the workspace root
//! `Cargo.toml` (the only place Cargo reads one), which this crate does not
//! own:
//!
//! ```toml
//! [profile.dev.package.z3-sys]
//! opt-level = 3
//! debug = false
//! ```
//!
//! It makes CMake build z3 as `Release` and costs one more full z3 build.
//!
//! # Example
//!
//! ```
//! use stave_smt::{Constraint, Domain, Outcome, Problem, Rational, Relation, Unknown};
//!
//! # fn main() -> Result<(), stave_smt::SmtError> {
//! let mut problem = Problem::new();
//! problem.add_unknown(Unknown {
//!     name: "width".into(),
//!     current: Rational::from_integer(900),
//!     domain: Domain::range(Rational::from_integer(600), Rational::from_integer(1200))?,
//!     rank: 0,
//!     order: 0,
//! });
//! problem.add_constraint(Constraint::new(
//!     "dragged size",
//!     [("width", Rational::from_integer(2))],
//!     Relation::Equal,
//!     Rational::from_integer(1900),
//! ));
//! let Outcome::Solved(solution) = problem.solve()? else {
//!     panic!("expected an answer");
//! };
//! assert_eq!(solution.value("width"), Some(Rational::from_integer(950)));
//! # Ok(())
//! # }
//! ```

mod problem;
mod rational;
mod solve;

pub use problem::{Constraint, Domain, Problem, Relation, Unknown};
pub use rational::Rational;

/// What went wrong building or solving a problem. A problem that has no
/// answer is not an error: it is [`Outcome::Unsatisfiable`].
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SmtError {
    /// A fraction was given a zero denominator.
    #[error("a fraction cannot have a denominator of zero")]
    ZeroDenominator,
    /// An exact result did not fit in 128-bit integers.
    #[error("a number is too large to hold exactly")]
    Overflow,
    /// A range's lowest value is above its highest.
    #[error("the range from {lowest} to {highest} is the wrong way round")]
    InvertedRange {
        /// The lowest value given.
        lowest: Rational,
        /// The highest value given.
        highest: Rational,
    },
    /// A list of allowed values was empty.
    #[error("an unknown's list of allowed values is empty")]
    EmptyDomain,
    /// Two unknowns have the same name.
    #[error("two unknowns are both named `{0}`")]
    DuplicateUnknown(String),
    /// Two unknowns have the same place in declaration order.
    #[error("unknowns `{first}` and `{second}` both have order {order}")]
    DuplicateOrder {
        /// The shared order.
        order: u32,
        /// The first unknown with it.
        first: String,
        /// The second unknown with it.
        second: String,
    },
    /// Two constraints have the same label.
    #[error("two constraints are both labelled `{0}`")]
    DuplicateConstraint(String),
    /// A constraint names an unknown the problem does not have.
    #[error("constraint `{constraint}` uses `{unknown}`, which is not an unknown of the problem")]
    UndeclaredUnknown {
        /// The constraint's label.
        constraint: String,
        /// The name it used.
        unknown: String,
    },
    /// The strict margin is zero or negative.
    #[error("the strict margin must be above zero, not {0}")]
    NonPositiveMargin(Rational),
    /// z3 gave a value this crate could not read exactly.
    #[error("the solver gave a value that could not be read exactly: {0}")]
    UnreadableValue(String),
    /// z3 failed in a way that is not a timeout.
    #[error("the solver failed: {0}")]
    Solver(String),
}

/// One item of a conflict: something that cannot hold together with the
/// other items named beside it.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Conflict {
    /// The allowed values (range or list) of the named unknown.
    Domain(String),
    /// The constraint with this label.
    Constraint(String),
}

/// The result of [`Problem::solve`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The best answer.
    Solved(Solution),
    /// No answer: a smallest set of domains and constraints that conflict,
    /// sorted (domains first, by name, then constraints, by label).
    Unsatisfiable(Vec<Conflict>),
    /// The time limit ran out before an answer was proved best.
    TimedOut,
}

/// An exact value for every unknown of a solved problem.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Solution {
    /// `(name, current, solved)` in declaration order.
    answers: Vec<(String, Rational, Rational)>,
}

impl Solution {
    fn new(answers: Vec<(String, Rational, Rational)>) -> Solution {
        Solution { answers }
    }

    /// The solved value of the named unknown, or `None` for a name the
    /// problem did not have.
    #[must_use]
    pub fn value(&self, name: &str) -> Option<Rational> {
        self.answers
            .iter()
            .find(|(known, _, _)| known == name)
            .map(|(_, _, value)| *value)
    }

    /// Every unknown's name and solved value, in declaration order.
    pub fn values(&self) -> impl Iterator<Item = (&str, Rational)> {
        self.answers
            .iter()
            .map(|(name, _, value)| (name.as_str(), *value))
    }

    /// The names of the unknowns whose solved value differs from their
    /// current value, in declaration order.
    pub fn changed(&self) -> impl Iterator<Item = &str> {
        self.answers
            .iter()
            .filter(|(_, current, value)| current != value)
            .map(|(name, _, _)| name.as_str())
    }
}

impl Problem {
    /// Solves the problem: the best answer, the conflict that rules every
    /// answer out, or a timeout. See the [crate documentation](crate) for
    /// what "best" means.
    ///
    /// # Errors
    ///
    /// An [`SmtError`] when the problem is malformed (duplicate names, labels
    /// or orders; a constraint naming an unknown that is not there; an
    /// inverted range or empty list; a strict margin that is not positive),
    /// when a number overflows, or when z3 fails other than by timing out.
    pub fn solve(&self) -> Result<Outcome, SmtError> {
        solve::solve(self)
    }
}
