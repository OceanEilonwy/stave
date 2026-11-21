//! `wall:` on a cut (grammar, "Cut operations" and "Walls"): a closed cut
//! must say the least wood it leaves round it, the old `wall(cut)` rule is
//! retired with a help that says what replaces it, and a wall is measured
//! to the part's faces and its other cuts, but not to the other cuts its
//! own pair makes with it.

mod common;

use common::{Checked, check};
use stave_lang::{Code, Diagnostic};

/// A base 300 x 200 x 19 lying flat, a peg standing on it, and a joint
/// kind `peg_hole` whose cut is `cut`, with `rule` among its entries.
fn design(cut: &str, rule: &str) -> String {
    format!(
        r#"stave 1

project "Walls" {{
  units: mm
}}

stock pine {{ species: pine, form: solid, thickness: 19mm, max_length: 2400mm }}

kind peg_hole : joint {{
  roles {{
    piece: Part
    into:  Part
  }}
  insert: [push(piece, along: length)]
  cut hole: crossing(into, piece) {{ {cut} }}
  {rule}
}}

part base : pine {{
  length: 300mm
  width:  200mm
  place {{ ground }}
}}

part peg : pine {{
  length: 60mm
  width:  19mm
  place {{
    orient(length: up, width: back)
    against(base.top)
    flush(base.left, inset: 100mm)
    flush(base.front, inset: 100mm)
  }}
}}

joint pegged : peg_hole(peg, into: base)
"#
    )
}

fn errors(checked: &Checked) -> Vec<&Diagnostic> {
    checked
        .diagnostics
        .iter()
        .filter(|d| d.is_error())
        .collect()
}

fn only_error(text: &str) -> Diagnostic {
    let checked = check(text);
    let errors = errors(&checked);
    let [error] = errors.as_slice() else {
        panic!("{errors:#?}");
    };
    (*error).clone()
}

/// A drill and a pocket are closed cuts: each must say its wall.
#[test]
fn a_closed_cut_without_a_wall_is_missing_a_field() {
    for (cut, word) in [
        ("drill(diameter: 8mm, depth: 10mm)", "drill"),
        ("pocket(width: 10mm, thickness: 8mm, depth: 10mm)", "pocket"),
    ] {
        let error = only_error(&design(cut, ""));
        assert_eq!(error.code, Code::MissingField, "{error:#?}");
        assert_eq!(error.message, format!("'{word}' is missing 'wall'"));
    }
}

/// With its wall, the same cut checks clean, and a notch may leave it out.
#[test]
fn a_cut_with_its_wall_checks() {
    check(&design("drill(diameter: 8mm, depth: 10mm, wall: 3mm)", "")).clean();
    let notched = check(&design("notch(depth: 5mm)", ""));
    assert!(
        !notched
            .diagnostics
            .iter()
            .any(|d| d.code == Code::MissingField),
        "{:#?}",
        notched.messages()
    );
}

/// A wall less than 0mm is out of range.
#[test]
fn a_wall_less_than_nothing_is_out_of_range() {
    let error = only_error(&design("drill(diameter: 8mm, depth: 10mm, wall: -1mm)", ""));
    assert_eq!(error.code, Code::OutOfRange, "{error:#?}");
    assert_eq!(error.message, "'wall' must be 0mm or more, found -1mm");
}

/// A hole 10mm deep in a 19mm base leaves 9mm behind it: enough for a
/// 6mm wall, not for a 10mm one.
#[test]
fn a_wall_is_measured_to_the_face_behind_the_hole() {
    check(&design("drill(diameter: 8mm, depth: 10mm, wall: 6mm)", "")).clean();
    let error = only_error(&design("drill(diameter: 8mm, depth: 10mm, wall: 10mm)", ""));
    assert_eq!(error.code, Code::Wall, "{error:#?}");
    assert_eq!(
        error.message,
        "pegged: its hole in base leaves 9mm of wood at the bottom face of base; it needs 10mm"
    );
}

/// `wall(cut)` was retired: written in a rule, it says what replaces it.
#[test]
fn the_retired_wall_function_says_what_replaces_it() {
    let error = only_error(&design(
        "drill(diameter: 8mm, depth: 10mm, wall: 3mm)",
        r#"require hole_wall : wall(hole) >= 3mm "too close""#,
    ));
    assert_eq!(error.code, Code::UnknownName, "{error:#?}");
    assert_eq!(
        error.message,
        "'wall(...)' was retired: a cut says the least wall it needs"
    );
    assert_eq!(
        error.help.as_deref(),
        Some(
            "give the cut wall:, as in drill(diameter: 8mm, depth: 20mm, wall: 3mm), and take out the rule"
        )
    );
}

/// A counterbore and the hole through its floor are one pair's cuts, made
/// together: they are no walls to each other, though each keeps its wall
/// to the faces.
#[test]
fn a_pairs_own_cuts_are_no_walls_to_each_other() {
    let stepped = r#"cut counterbore: crossing(into, piece) { drill(diameter: 12mm, depth: 4mm, wall: 3mm) }"#;
    check(&design(
        "drill(diameter: 6mm, depth: 12mm, wall: 3mm)",
        stepped,
    ))
    .clean();
}
