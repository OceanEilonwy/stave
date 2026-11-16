//! Executable spec: "Lexical grammar / Quantities", "Types, units and
//! arithmetic", `param` and `let` (grammar, "Declarations and their
//! fields"). Values are checked through `require`s at the current values,
//! which the solved design reports one by one.

mod common;

use common::{body_line, holds, mm, only_error, open, part, the_diagnostic};

// ---------------------------------------------------------------------------
// Quantities: every length carries a unit, and literals are exact
// ---------------------------------------------------------------------------

#[test]
fn a_mixed_fraction_is_one_quantity() {
    holds("require r : 1-1/2in == 38.1mm \"1-1/2in is one and a half inches\"");
}

#[test]
fn a_length_minus_a_length_written_without_spaces_is_two_quantities() {
    holds("require r : 2in-1/2in == 1-1/2in \"2in minus 1/2in\"");
}

#[test]
fn a_count_minus_a_length_is_a_unit_mismatch() {
    // `1 - 1/2in` is INT(1) - QUANTITY(1/2in), not a mixed fraction.
    only_error(
        &open("let a = 1 - 1/2in"),
        "unit-mismatch",
        "can't subtract a count and a length",
    );
}

#[test]
fn a_third_of_an_inch_is_exact() {
    holds("require r : 1/3in * 3 == 1in \"thirds add up exactly\"");
    holds("require r : 1in / 3 * 3 == 1in \"dividing by a count is exact\"");
}

#[test]
fn every_unit_converts_exactly() {
    holds("require r : 1cm == 10mm && 1m == 1000mm && 1ft == 12in && 1in == 25.4mm \"units\"");
    holds("require r : 0.1mm * 10 == 1mm && 18.5mm + 0.5mm == 19mm \"decimals are exact\"");
}

#[test]
fn digits_may_be_grouped_with_underscores() {
    holds("require r : 1_000mm == 1m \"1_000mm\"");
}

#[test]
fn angles_add_like_lengths() {
    holds("require r : 90deg + 45deg * 2 == 180deg && -(10deg) < 0deg \"angles\"");
}

// ---------------------------------------------------------------------------
// Arithmetic: the table, and everything else is a unit mismatch
// ---------------------------------------------------------------------------

#[test]
fn a_length_divided_by_a_length_is_a_ratio() {
    holds("require r : 10mm / 4mm == 2.5 \"a ratio\"");
}

#[test]
fn a_count_may_be_used_where_a_ratio_is_expected() {
    holds("require r : 3mm * 0.5 == 1.5mm && 2 * 3mm == 6mm && 6mm / 2 == 3mm \"scaling\"");
    holds("require r : 2 / 3 < 0.67 && 2 / 3 > 0.66 \"count by count is an exact ratio\"");
}

#[test]
fn unary_minus_negates_a_length() {
    holds("require r : -(3mm) + 5mm == 2mm \"negation\"");
}

#[test]
fn a_length_plus_an_angle_is_a_unit_mismatch() {
    let opened = open("let a = 1mm + 1deg");
    let diagnostic = the_diagnostic(&opened, "unit-mismatch", "can't add a length and an angle");
    assert_eq!(body_line(&opened, diagnostic), 1);
}

#[test]
fn a_count_divided_by_a_length_is_a_unit_mismatch() {
    only_error(
        &open("let a = 2 / 1mm"),
        "unit-mismatch",
        "can't divide a count and a length",
    );
}

#[test]
fn an_angle_times_a_length_is_a_unit_mismatch() {
    only_error(
        &open("let a = 2deg * 3mm"),
        "unit-mismatch",
        "can't multiply an angle and a length",
    );
}

#[test]
fn a_comparison_needs_the_same_type_on_both_sides() {
    only_error(
        &open("require r : 1mm < 2 \"m\""),
        "unit-mismatch",
        "can't compare a length and a count",
    );
}

#[test]
fn logical_operators_take_bools() {
    holds("require r : true && !false || false \"logic\"");
    only_error(
        &open("require r : 1mm && true \"m\""),
        "type-mismatch",
        "'&&' needs true or false, found a length",
    );
    only_error(
        &open("require r : !1mm \"m\""),
        "type-mismatch",
        "'!' needs true or false, found a length",
    );
}

#[test]
fn dividing_by_zero_is_a_type_mismatch() {
    the_diagnostic(
        &open("let a = 1mm / 0"),
        "type-mismatch",
        "division by zero",
    );
}

#[test]
fn comparisons_do_not_chain() {
    // `cmp_expr` takes at most one comparison operator.
    only_error(
        &open("require r : 1 < 2 < 3 \"m\""),
        "syntax",
        "expected a message in quotes, found '<'",
    );
}

#[test]
fn multiplication_binds_tighter_than_addition() {
    holds("require r : 1 + 2 * 3 == 7 && (1 + 2) * 3 == 9 && 8 - 2 - 1 == 5 \"precedence\"");
}

#[test]
fn a_bare_number_is_never_a_length() {
    let opened = open(
        "part p : pine_19 {\n  length: 3\n  width:  100mm\n  place {\n    against(base.top)\n    flush(base.left)\n    flush(base.front)\n  }\n}",
    );
    let diagnostic = the_diagnostic(
        &opened,
        "unit-mismatch",
        "'length' needs a length, found a count",
    );
    assert_eq!(body_line(&opened, diagnostic), 2);
}

// ---------------------------------------------------------------------------
// param
// ---------------------------------------------------------------------------

#[test]
fn a_param_takes_its_type_from_its_default() {
    holds("param a = 3mm\nrequire r : a == 3mm \"default\"");
}

#[test]
fn a_param_with_a_type_and_a_default_must_agree() {
    holds("param a : Length = 3mm\nrequire r : a == 3mm \"agree\"");
    only_error(
        &open("param a : Length = 3deg"),
        "unit-mismatch",
        "'a' needs a length, found an angle",
    );
}

#[test]
fn a_param_at_the_top_level_of_a_module_must_have_a_default() {
    the_diagnostic(
        &open("param a : Length"),
        "missing-field",
        "param 'a' is missing a default",
    );
}

#[test]
fn a_misspelt_type_name_is_an_unknown_name() {
    let opened = open("param a : Lenght = 3mm");
    only_error(&opened, "unknown-name", "unknown name 'Lenght'");
    assert_eq!(common::help(&opened), "did you mean 'Length'?");
}

#[test]
fn param_defaults_may_refer_to_each_other_in_any_order() {
    // Order doesn't matter (grammar, `param`): a module's files have none.
    holds("param a = b + 1mm\nparam b = 2mm\nrequire r : a == 3mm \"later param\"");
    holds(
        "component Box {\n  param a = b + 1mm\n  param b = 2mm\n  require r : a == 3mm \"in a component\"\n}\nassembly box : Box { }",
    );
}

#[test]
fn a_value_worked_out_from_itself_is_a_type_mismatch() {
    only_error(
        &open("let a = b\nlet b = a"),
        "type-mismatch",
        "'a' is worked out from itself",
    );
}

#[test]
fn within_values_must_have_the_params_type() {
    let opened = open("param a = 3mm within [1, 2]");
    assert!(
        opened
            .diagnostics
            .iter()
            .all(|d| d.message == "'within' needs a length, found a count"),
        "{:?}",
        common::diagnostics(&opened)
    );
}

#[test]
fn a_module_params_default_must_lie_within_its_range() {
    holds("param a = 2mm within 1mm..2mm\nrequire r : a == 2mm \"both ends are included\"");
    holds("param a = 2mm within [1mm, 2mm]\nrequire r : a == 2mm \"one of a list\"");
    only_error(
        &open("param a = 5mm within 1mm..2mm"),
        "out-of-range",
        "'a' must be within 1mm..2mm, found 5mm",
    );
}

#[test]
fn a_newline_after_within_continues_the_line() {
    holds("param a = 2mm within\n  1mm..3mm\nrequire r : a == 2mm \"within continues\"");
}

#[test]
fn locked_is_written_last() {
    holds("param reveal = 1.5mm within 1mm..2mm locked\nrequire r : reveal == 1.5mm \"locked\"");
}

// ---------------------------------------------------------------------------
// let
// ---------------------------------------------------------------------------

#[test]
fn a_let_is_a_named_value_used_before_or_after_it_is_written() {
    holds("let b = a * 2\nlet a = 3mm\nrequire r : b == 6mm \"lets\"");
}

#[test]
fn a_let_can_not_be_passed_into_a_component() {
    only_error(
        &open("component Box {\n  let gap = 1mm\n}\nassembly box : Box {\n  gap: 2mm\n}"),
        "unknown-field",
        "the component 'Box' has no field 'gap'",
    );
}

// ---------------------------------------------------------------------------
// Strings
// ---------------------------------------------------------------------------

#[test]
fn a_require_message_fills_in_values_in_the_projects_units() {
    let opened =
        open("param depth = 9mm\nrequire deep : depth > 10mm \"depth {depth} is too small\"");
    the_diagnostic(&opened, "require", "'deep' fails: depth 9mm is too small");
}

#[test]
fn an_interpolated_name_must_resolve() {
    only_error(
        &open("kind k : joint {\n  note: \"{nothing}\"\n}"),
        "unknown-name",
        "unknown name 'nothing'",
    );
}

#[test]
fn escapes_in_strings() {
    let opened = open(
        "part p : pine_19 {\n  length: 100mm\n  width:  100mm\n  note:   \"tab\\t \\\"quoted\\\" back\\\\ brace\\{ \\u{e9}\"\n  place {\n    against(base.top)\n    flush(base.left)\n    flush(base.front)\n  }\n}",
    );
    assert_eq!(
        part(common::design(&opened), "p").note.as_deref(),
        Some("tab\t \"quoted\" back\\ brace{ é")
    );
}

#[test]
fn the_length_unit_of_a_size_is_kept_exact_to_the_nanometre() {
    let opened = open(
        "part p : pine_19 {\n  length: 1in / 3\n  width:  100mm\n  place {\n    against(base.top)\n    flush(base.left)\n    flush(base.front)\n  }\n}",
    );
    // Stored results are whole nanometres: 25.4mm / 3 rounds once.
    assert_eq!(
        part(common::design(&opened), "p").cut.length,
        mm("8.466667")
    );
}
