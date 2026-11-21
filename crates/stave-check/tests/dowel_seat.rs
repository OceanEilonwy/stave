//! What `dowel`'s `seat_wide` and `seat_long` rules asked, checked with
//! no rule: each hole's own `wall:` (grammar, "Walls"). The patch where the
//! parts touch is bounded on every side by a face of one of them, and each
//! part's holes keep `wall` to its faces and to each other, so a patch too
//! narrow for a pin, or too short for the pins, leaves a wall too thin.
//! The boundaries are exactly where the rules put them:
//!
//! - across: a pin and a wall each side, `diameter + 2 * wall`;
//! - along: every pin with a wall between and beyond,
//!   `count * (diameter + wall) + wall`.
//!
//! The design: a 19mm side standing on a base, back faces flush, joined by
//! 8mm dowels with a 4mm wall. The patch is the side's 19mm thickness by
//! the shorter of the side's and the base's depths.

mod common;

use common::check;
use stave_lang::{Code, Diagnostic};

fn design(base_deep_mm: i64, side_deep_mm: i64, count: i64) -> String {
    format!(
        r#"stave 1

project "Dowel seat" {{
  units: mm
}}

stock pine {{ species: pine, form: solid, thickness: 19mm, max_length: 2400mm }}

part base : pine {{
  length: 400mm
  width:  {base_deep_mm}mm
  place {{ ground }}
}}

part side : pine {{
  length: 300mm
  width:  {side_deep_mm}mm
  place {{
    orient(length: up, width: back)
    against(base.top)
    flush(base.left)
    flush(base.back)
  }}
}}

joint pins : dowel(side, into: base, count: {count}, diameter: 8mm, wall: 4mm, depth: 12mm)
"#
    )
}

fn errors(text: &str) -> Vec<Diagnostic> {
    check(text)
        .diagnostics
        .into_iter()
        .filter(Diagnostic::is_error)
        .collect()
}

/// `seat_wide`: one pin needs a patch 8mm + 2 * 4mm = 16mm across.
#[test]
fn a_patch_too_narrow_for_a_pin_leaves_a_wall_too_thin() {
    assert!(errors(&design(17, 200, 1)).is_empty());
    assert!(errors(&design(16, 200, 1)).is_empty());
    let errors = errors(&design(15, 200, 1));
    let [error] = errors.as_slice() else {
        panic!("{errors:#?}");
    };
    assert_eq!(error.code, Code::Wall);
    assert_eq!(
        error.message,
        "pins: its hole in base leaves 3.5mm of wood at the front edge of base; it needs 4mm"
    );
}

/// `seat_long`: three pins need a patch 3 * (8mm + 4mm) + 4mm = 40mm long.
#[test]
fn a_patch_too_short_for_its_pins_leaves_a_wall_too_thin() {
    assert!(errors(&design(100, 41, 3)).is_empty());
    assert!(errors(&design(100, 40, 3)).is_empty());
    let errors = errors(&design(100, 39, 3));
    let [error] = errors.as_slice() else {
        panic!("{errors:#?}");
    };
    assert_eq!(error.code, Code::Wall);
    assert_eq!(
        error.message,
        "pins: its hole in base leaves 3.75mm of wood at the hole that pins cuts in base; it needs 4mm"
    );
}

/// A design asks for a thinner wall with the joint's own setting, and the
/// pin that didn't fit across now does.
#[test]
fn a_joint_takes_its_wall_as_a_setting() {
    let text = design(15, 200, 1).replace("wall: 4mm", "wall: 3mm");
    assert!(errors(&text).is_empty());
}
