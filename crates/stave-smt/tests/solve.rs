//! Behaviour of `Problem::solve`: the answer it picks, its conflicts, and
//! that it picks the same answer every time.

use std::time::Duration;

use stave_smt::{
    Conflict, Constraint, Domain, Outcome, Problem, Rational, Relation, SmtError, Solution, Unknown,
};

fn whole(value: i128) -> Rational {
    Rational::from_integer(value)
}

fn fraction(numerator: i128, denominator: i128) -> Rational {
    match Rational::new(numerator, denominator) {
        Ok(value) => value,
        Err(error) => panic!("{error}"),
    }
}

fn range(lowest: i128, highest: i128) -> Domain {
    match Domain::range(whole(lowest), whole(highest)) {
        Ok(domain) => domain,
        Err(error) => panic!("{error}"),
    }
}

fn unknown(name: &str, current: i128, domain: Domain, rank: u32, order: u32) -> Unknown {
    Unknown {
        name: name.into(),
        current: whole(current),
        domain,
        rank,
        order,
    }
}

fn constraint(
    label: &str,
    terms: &[(&str, i128)],
    relation: Relation,
    constant: i128,
) -> Constraint {
    Constraint::new(
        label,
        terms
            .iter()
            .map(|(name, coefficient)| (*name, whole(*coefficient))),
        relation,
        whole(constant),
    )
}

fn solved(problem: &Problem) -> Solution {
    match problem.solve() {
        Ok(Outcome::Solved(solution)) => solution,
        other => panic!("expected an answer, got {other:?}"),
    }
}

fn values(solution: &Solution) -> Vec<(String, Rational)> {
    solution
        .values()
        .map(|(name, value)| (name.to_owned(), value))
        .collect()
}

#[test]
fn one_unknown_meets_one_equality() {
    let mut problem = Problem::new();
    problem.add_unknown(unknown("width", 900, range(600, 1200), 0, 0));
    problem.add_constraint(constraint(
        "dragged",
        &[("width", 3)],
        Relation::Equal,
        2851,
    ));
    let solution = solved(&problem);
    assert_eq!(solution.value("width"), Some(fraction(2851, 3)));
    assert_eq!(solution.changed().collect::<Vec<_>>(), ["width"]);
}

#[test]
fn changing_fewer_unknowns_wins() {
    // a + b = 1000 can be met by moving only `b` (to 600) or by moving both
    // a little; moving one wins, though it moves further.
    let mut problem = Problem::new();
    problem.add_unknown(unknown("a", 400, range(0, 1000), 0, 0));
    problem.add_unknown(unknown("b", 400, range(0, 1000), 0, 1));
    problem.add_constraint(constraint(
        "sum",
        &[("a", 1), ("b", 1)],
        Relation::Equal,
        1000,
    ));
    // `b` alone may not move: it is capped at 500, so only `a` can take it all.
    problem.add_constraint(constraint(
        "b small",
        &[("b", 1)],
        Relation::LessOrEqual,
        500,
    ));
    let solution = solved(&problem);
    assert_eq!(solution.changed().collect::<Vec<_>>(), ["a"]);
    assert_eq!(solution.value("a"), Some(whole(600)));
    assert_eq!(solution.value("b"), Some(whole(400)));
}

#[test]
fn a_tie_on_count_goes_to_the_smaller_fraction_moved() {
    // Either unknown alone can absorb 100; `wide` has the wider domain, so
    // moving it is the smaller fraction, though it is declared later and has
    // the higher rank.
    let mut problem = Problem::new();
    problem.add_unknown(unknown("narrow", 500, range(400, 600), 0, 0));
    problem.add_unknown(unknown("wide", 500, range(0, 1000), 5, 1));
    problem.add_constraint(constraint(
        "sum",
        &[("narrow", 1), ("wide", 1)],
        Relation::Equal,
        1100,
    ));
    let solution = solved(&problem);
    assert_eq!(solution.changed().collect::<Vec<_>>(), ["wide"]);
    assert_eq!(solution.value("wide"), Some(whole(600)));
}

#[test]
fn a_tie_on_fraction_goes_to_the_lower_rank() {
    let mut problem = Problem::new();
    problem.add_unknown(unknown("project", 500, range(0, 1000), 2, 0));
    problem.add_unknown(unknown("component", 500, range(0, 1000), 0, 1));
    problem.add_constraint(constraint(
        "sum",
        &[("project", 1), ("component", 1)],
        Relation::Equal,
        1100,
    ));
    let solution = solved(&problem);
    assert_eq!(solution.changed().collect::<Vec<_>>(), ["component"]);
    assert_eq!(solution.value("component"), Some(whole(600)));
}

#[test]
fn a_tie_on_rank_goes_to_the_earlier_declared() {
    let mut problem = Problem::new();
    problem.add_unknown(unknown("second", 500, range(0, 1000), 1, 7));
    problem.add_unknown(unknown("first", 500, range(0, 1000), 1, 3));
    problem.add_constraint(constraint(
        "sum",
        &[("first", 1), ("second", 1)],
        Relation::Equal,
        1100,
    ));
    let solution = solved(&problem);
    assert_eq!(solution.changed().collect::<Vec<_>>(), ["first"]);
    assert_eq!(solution.value("first"), Some(whole(600)));
    assert_eq!(solution.value("second"), Some(whole(500)));
    // Values come back in declaration order, not the order they were added.
    assert_eq!(
        values(&solution),
        [
            ("first".to_owned(), whole(600)),
            ("second".to_owned(), whole(500))
        ]
    );
}

#[test]
fn a_bound_forces_the_other_unknown_to_move() {
    // By order `first` would take the change, but its range stops 20 above
    // where it is, short of the 40 needed; `second` alone can, so it moves.
    let mut problem = Problem::new();
    problem.add_unknown(unknown("first", 500, range(0, 520), 0, 0));
    problem.add_unknown(unknown("second", 500, range(0, 550), 0, 1));
    problem.add_constraint(constraint(
        "sum",
        &[("first", 1), ("second", 1)],
        Relation::Equal,
        1040,
    ));
    let solution = solved(&problem);
    assert_eq!(solution.changed().collect::<Vec<_>>(), ["second"]);
    assert_eq!(solution.value("second"), Some(whole(540)));

    // When neither alone can reach it, both move: the fraction sum is the
    // same however the 80 is split, so order sends as much as it can to
    // `first` (up to its bound) and the rest to `second`.
    let mut problem = Problem::new();
    problem.add_unknown(unknown("first", 500, range(0, 550), 0, 0));
    problem.add_unknown(unknown("second", 500, range(0, 550), 0, 1));
    problem.add_constraint(constraint(
        "sum",
        &[("first", 1), ("second", 1)],
        Relation::Equal,
        1080,
    ));
    let solution = solved(&problem);
    assert_eq!(solution.value("first"), Some(whole(550)));
    assert_eq!(solution.value("second"), Some(whole(530)));
}

#[test]
fn an_unknown_restricted_to_a_list_takes_a_listed_value() {
    let thicknesses = match Domain::values([whole(12), whole(18), whole(25)]) {
        Ok(domain) => domain,
        Err(error) => panic!("{error}"),
    };
    let mut problem = Problem::new();
    problem.add_unknown(unknown("thickness", 12, thicknesses, 0, 0));
    problem.add_constraint(constraint(
        "thick enough",
        &[("thickness", 1)],
        Relation::Greater,
        15,
    ));
    let solution = solved(&problem);
    assert_eq!(solution.value("thickness"), Some(whole(18)));

    // Equally far above and below: the lower value.
    let listed = match Domain::values([whole(10), whole(30)]) {
        Ok(domain) => domain,
        Err(error) => panic!("{error}"),
    };
    let mut problem = Problem::new();
    problem.add_unknown(unknown("spacing", 20, listed, 0, 0));
    let solution = solved(&problem);
    assert_eq!(solution.value("spacing"), Some(whole(10)));
}

#[test]
fn strict_inequalities_use_the_margin() {
    let mut problem = Problem::new();
    problem.add_unknown(unknown("gap", 0, range(0, 100), 0, 0));
    problem.add_constraint(constraint("clear", &[("gap", 1)], Relation::Greater, 5));
    assert_eq!(solved(&problem).value("gap"), Some(whole(6)));
    problem.set_strict_margin(fraction(1, 4));
    assert_eq!(solved(&problem).value("gap"), Some(fraction(21, 4)));
    problem.set_strict_margin(Rational::ZERO);
    assert_eq!(
        problem.solve(),
        Err(SmtError::NonPositiveMargin(Rational::ZERO))
    );
}

#[test]
fn an_unsatisfiable_problem_names_the_conflict() {
    let mut problem = Problem::new();
    problem.add_unknown(unknown("width", 900, range(600, 1200), 0, 0));
    problem.add_unknown(unknown("depth", 500, range(400, 600), 0, 1));
    problem.add_constraint(constraint(
        "dragged",
        &[("width", 1)],
        Relation::Equal,
        1000,
    ));
    problem.add_constraint(constraint(
        "width under twice depth",
        &[("width", 1), ("depth", -2)],
        Relation::LessOrEqual,
        0,
    ));
    problem.add_constraint(constraint(
        "depth small",
        &[("depth", 1)],
        Relation::LessOrEqual,
        450,
    ));
    problem.add_constraint(constraint(
        "unrelated",
        &[("depth", 1)],
        Relation::GreaterOrEqual,
        0,
    ));
    let Ok(Outcome::Unsatisfiable(conflicts)) = problem.solve() else {
        panic!("expected a conflict");
    };
    assert_eq!(
        conflicts,
        [
            Conflict::Constraint("depth small".into()),
            Conflict::Constraint("dragged".into()),
            Conflict::Constraint("width under twice depth".into()),
        ]
    );

    // A domain can be part of the conflict.
    let mut problem = Problem::new();
    problem.add_unknown(unknown("width", 900, range(600, 1200), 0, 0));
    problem.add_constraint(constraint(
        "dragged",
        &[("width", 1)],
        Relation::Equal,
        1300,
    ));
    let Ok(Outcome::Unsatisfiable(conflicts)) = problem.solve() else {
        panic!("expected a conflict");
    };
    assert_eq!(
        conflicts,
        [
            Conflict::Domain("width".into()),
            Conflict::Constraint("dragged".into()),
        ]
    );
}

#[test]
fn malformed_problems_are_errors() {
    let mut problem = Problem::new();
    problem.add_unknown(unknown("a", 0, range(0, 1), 0, 0));
    problem.add_unknown(unknown("b", 0, range(0, 1), 0, 0));
    assert!(matches!(
        problem.solve(),
        Err(SmtError::DuplicateOrder { .. })
    ));

    let mut problem = Problem::new();
    problem.add_unknown(unknown("a", 0, range(0, 1), 0, 0));
    problem.add_constraint(constraint("c", &[("missing", 1)], Relation::Equal, 0));
    assert!(matches!(
        problem.solve(),
        Err(SmtError::UndeclaredUnknown { .. })
    ));

    assert!(Domain::range(whole(2), whole(1)).is_err());
    assert_eq!(Domain::values([]), Err(SmtError::EmptyDomain));
}

#[test]
fn a_zero_time_limit_times_out() {
    let mut problem = Problem::new();
    problem.add_unknown(unknown("a", 0, range(0, 10), 0, 0));
    problem.set_time_limit(Duration::ZERO);
    assert_eq!(problem.solve(), Ok(Outcome::TimedOut));
}

/// `items` in an order fixed by `seed` (a Fisher-Yates shuffle driven by a
/// small linear congruential generator).
fn shuffled<T: Clone>(items: &[T], seed: usize) -> Vec<T> {
    let mut items = items.to_vec();
    let mut state = seed.wrapping_mul(2_654_435_761).wrapping_add(12_345);
    for top in (1..items.len()).rev() {
        state = state.wrapping_mul(1_103_515_245).wrapping_add(12_345);
        items.swap(top, (state >> 8) % (top + 1));
    }
    items
}

/// A problem with ties at every key, built with unknowns, constraints and
/// terms in the order `permutation` gives.
fn tied_problem(permutation: usize) -> Problem {
    let unknowns = [
        unknown("p0", 100, range(0, 1000), 1, 0),
        unknown("p1", 100, range(0, 1000), 1, 1),
        unknown("p2", 100, range(0, 1000), 1, 2),
        unknown("p3", 250, range(0, 1000), 0, 3),
        unknown(
            "p4",
            30,
            match Domain::values([whole(10), whole(20), whole(40), whole(50)]) {
                Ok(domain) => domain,
                Err(error) => panic!("{error}"),
            },
            0,
            4,
        ),
        unknown("p5", 7, range(0, 1000), 2, 5),
    ];
    let constraints = [
        Constraint::new(
            "target",
            [
                ("p0", whole(1)),
                ("p1", whole(1)),
                ("p2", whole(1)),
                ("p5", whole(0)),
            ],
            Relation::Equal,
            whole(700),
        ),
        Constraint::new(
            "half",
            [
                ("p3", fraction(1, 2)),
                ("p0", fraction(1, 3)),
                ("p3", fraction(1, 2)),
            ],
            Relation::GreaterOrEqual,
            whole(300),
        ),
        Constraint::new("list moves", [("p4", whole(1))], Relation::Less, whole(30)),
        Constraint::new(
            "cap",
            [("p1", whole(2)), ("p2", whole(-1))],
            Relation::LessOrEqual,
            whole(400),
        ),
    ];
    let mut problem = Problem::new();
    for unknown in shuffled(&unknowns, permutation) {
        problem.add_unknown(unknown);
    }
    let mut ordered = shuffled(&constraints, permutation.wrapping_add(1_000));
    for constraint in &mut ordered {
        constraint.terms = shuffled(&constraint.terms, permutation);
    }
    for constraint in ordered {
        problem.add_constraint(constraint);
    }
    problem
}

#[test]
fn the_same_problem_gives_the_same_answer_every_time() {
    let first = solved(&tied_problem(0));
    for permutation in 0..40 {
        let solution = solved(&tied_problem(permutation));
        assert_eq!(
            values(&solution),
            values(&first),
            "permutation {permutation}"
        );
    }
    // The answer itself, worked by hand. `list moves` forces p4 off 30 to
    // the nearer of 20 and 40 below 30: 20. `target` needs 400 more from
    // p0, p1 or p2; one alone can do it, so exactly one more unknown moves.
    // p1 alone breaks `cap` (2 × 500 − 100 > 400); p0 and p2 tie on
    // fraction (0.4 each) and rank (1 each), so declaration order picks p0.
    // `half` then still holds with p3 where it is (250 + 500/3 >= 300).
    let answer = values(&first);
    assert_eq!(
        answer,
        [
            ("p0".to_owned(), whole(500)),
            ("p1".to_owned(), whole(100)),
            ("p2".to_owned(), whole(100)),
            ("p3".to_owned(), whole(250)),
            ("p4".to_owned(), whole(20)),
            ("p5".to_owned(), whole(7)),
        ]
    );
}

#[test]
fn the_same_conflict_every_time() {
    let mut expected = None;
    for permutation in 0..20 {
        let mut problem = tied_problem(permutation);
        problem.add_constraint(constraint(
            "too much",
            &[("p0", 1)],
            Relation::GreaterOrEqual,
            2000,
        ));
        problem.add_constraint(constraint("too little", &[("p3", 1)], Relation::Less, 0));
        let Ok(Outcome::Unsatisfiable(conflicts)) = problem.solve() else {
            panic!("expected a conflict");
        };
        match &expected {
            None => expected = Some(conflicts),
            Some(first) => assert_eq!(&conflicts, first, "permutation {permutation}"),
        }
    }
}
