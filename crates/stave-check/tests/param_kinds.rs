//! The checks on a param's kind (grammar, "Param kinds"): `locked` and
//! `solved` need a default and a quantity type, and `solved` needs a
//! `within`; one error at most for a param, the missing default first.
//! The same in a module, a component and a kind.

mod common;

use common::check;
use stave_lang::Severity;

/// A project with one part on the ground; `{more}` adds items.
fn project(more: &str) -> String {
    format!(
        r#"stave 1

project "Kinds" {{
  units: mm
}}

stock pine_19 {{ species: pine, form: solid, thickness: 19mm, max_length: 2400mm }}

part base : pine_19 {{
  length: 600mm
  width:  300mm
  place {{
    ground
  }}
}}

{more}
"#
    )
}

/// Every error, as `code message`.
fn errors(text: &str) -> Vec<String> {
    check(text)
        .diagnostics
        .iter()
        .filter(|d| d.severity == Severity::Error)
        .map(|d| format!("{} {}", d.code, d.message))
        .collect()
}

#[test]
fn every_kind_of_a_quantity_with_a_default_and_a_range_checks_clean() {
    check(&project(
        "locked param reveal = 1.5mm\n\
         solved param overall_width = 900mm within 600mm..1200mm\n\
         private solved param shelves = 3 within 1..6\n\
         locked param tilt = 5deg\n\
         locked param share = 1 / 3\n",
    ))
    .clean();
}

#[test]
fn a_component_param_with_no_default_and_no_quantity_is_one_error() {
    let text = project(
        "component Block {\n  locked param opening : Region\n}\n\
         region space = opening(left: base.left, right: base.right, bottom: base.top, top: base.top, front: base.front, back: base.back)\n\
         assembly block : Block {\n  opening: space\n}\n",
    );
    assert_eq!(errors(&text), ["missing-value 'opening' has no value"]);
    let found = check(&text);
    let diagnostic = found.diagnostics.first().unwrap();
    assert_eq!(diagnostic.notes, ["a locked param needs a default"]);
}

#[test]
fn a_solved_param_with_no_default_is_missing_its_value_not_its_range() {
    let text = project(
        "component Block {\n  solved param gap : Length\n}\nassembly block : Block {\n  gap: 1mm\n}\n",
    );
    assert_eq!(errors(&text), ["missing-value 'gap' has no value"]);
}

#[test]
fn a_mark_on_a_param_that_is_not_a_quantity_names_its_type() {
    let text = project("solved param top_stock : Stock = pine_19 within [pine_19]\n");
    assert_eq!(
        errors(&text),
        ["type-mismatch 'solved' needs a length, an angle, a ratio or a count, found a stock"]
    );
}

#[test]
fn a_module_param_with_no_default_is_reported_once() {
    // A module param must have a default anyway: that one error says it.
    let text = project("locked param gap : Length\n");
    assert_eq!(
        errors(&text),
        ["missing-field param 'gap' is missing a default"]
    );
}

#[test]
fn a_solved_param_in_a_component_needs_a_range() {
    let text =
        project("component Block {\n  solved param gap = 2mm\n}\nassembly block : Block {\n}\n");
    assert_eq!(
        errors(&text),
        ["missing-field solved param 'gap' is missing 'within'"]
    );
}

#[test]
fn a_kind_param_is_checked_the_same_way() {
    let text = project("kind spot : joint {\n  solved param depth = 2mm\n}\n");
    assert_eq!(
        errors(&text),
        ["missing-field solved param 'depth' is missing 'within'"]
    );
}
