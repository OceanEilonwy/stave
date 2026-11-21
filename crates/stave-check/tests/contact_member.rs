//! `contact(member, b)` (grammar, "Built-in functions"), end to end on a
//! small design: a board lying flat and a pull kind written in the test
//! whose one member, a 40mm x 20mm plate on the board's top, has to lie
//! wholly on the board. Where the member is named anywhere but as the
//! first argument of `contact` in a `require`, it is a type mismatch.

mod common;

use common::{Checked, check};
use stave_lang::{Code, Diagnostic};

/// A board 300 x 100 lying flat, and a `plate_pull` whose plate
/// starts `inset` in from the board's left end, with `rule` among the
/// kind's entries.
fn design(inset_mm: i64, rule: &str) -> String {
    format!(
        r#"stave 1

project "Plates" {{
  units: mm
}}

stock pine {{ species: pine, form: solid, thickness: 19mm, max_length: 2400mm }}

kind plate_pull : pull {{
  roles {{
    piece: Part
  }}
  mount {{
    face:   top
    across: open
    along:  open
  }}
  member plate {{
    with:  piece
    shape: box(along: 40mm, across: 20mm, depth: -2mm..0mm)
  }}
  {rule}
}}

part board : pine {{
  length: 300mm
  width:  100mm
  place {{ ground }}
}}

hardware pull : plate_pull(board) {{
  place {{
    center(board, deep)
    flush(board.left, inset: {inset_mm}mm)
  }}
}}
"#
    )
}

const ON_WOOD: &str = r#"require plate_on_wood : contact(plate, piece).length >= 40mm && contact(plate, piece).width >= 20mm "the plate hangs off the board""#;

fn errors(checked: &Checked) -> Vec<&Diagnostic> {
    checked
        .diagnostics
        .iter()
        .filter(|d| d.is_error())
        .collect()
}

/// The plate wholly on the board: the rule holds.
#[test]
fn a_member_wholly_on_its_part_holds() {
    check(&design(0, ON_WOOD)).clean();
    check(&design(100, ON_WOOD)).clean();
}

/// Half the plate past the board's end: the patch is as long as the part
/// still under it, measured along the plate's own length.
#[test]
fn a_member_half_off_its_part_breaks_the_rule() {
    let checked = check(&design(-20, ON_WOOD));
    let errors = errors(&checked);
    let [error] = errors.as_slice() else {
        panic!("{errors:#?}");
    };
    assert_eq!(error.code, Code::Require);
    assert_eq!(
        error.message,
        "'plate_on_wood' fails: the plate hangs off the board"
    );
    assert_eq!(
        error.help.as_deref(),
        Some(
            "board holds only 20mm of the 40mm plate; place the hardware so its plate lies wholly on board"
        )
    );
}

/// Without a rule, the same overhang is accepted: contact asks only that a
/// member meet its part somewhere.
#[test]
fn without_the_rule_a_member_may_hang_off() {
    check(&design(-20, "")).clean();
}

/// A member has no value of its own: read anywhere but as the first
/// argument of `contact`, it is a type mismatch where it is written.
#[test]
fn a_member_read_outside_contact_is_a_type_mismatch() {
    let checked = check(&design(
        20,
        r#"require bare : plate "a member is not true or false""#,
    ));
    let messages: Vec<String> = errors(&checked)
        .iter()
        .map(|d| format!("{}: {}", d.code, d.message))
        .collect();
    assert!(
        messages.iter().any(|m| m.starts_with("type-mismatch:")),
        "{messages:#?}"
    );
}

/// The second argument of `contact` is a part, never a member.
#[test]
fn a_member_as_the_second_argument_is_a_type_mismatch() {
    let checked = check(&design(
        20,
        r#"require backwards : contact(piece, plate).length >= 40mm "backwards""#,
    ));
    let messages: Vec<String> = errors(&checked)
        .iter()
        .map(|d| format!("{}: {}", d.code, d.message))
        .collect();
    assert!(
        messages.iter().any(|m| m.starts_with("type-mismatch:")),
        "{messages:#?}"
    );
}
