//! Executable spec: a cut's `wall:` ("Walls"), the backstops every kind is
//! held to ("Cuts stay in their part") and the held check ("Held"), with
//! the rendered diagnostics of every broken design they catch
//! (docs/reviews/2026-11-17-checks-gaps.md, stage 3).

mod common;

use std::path::Path;

use common::{HEADER, diagnostics, holds, open, open_files_with_every_warning};
use stave_lang::render_diagnostics;

fn workspace() -> &'static Path {
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
}

/// The rendered diagnostics of a folder, with paths relative to the
/// workspace.
fn rendered(folder: &str) -> String {
    let path = workspace().join(folder);
    let opened = stave_app::open(&path).unwrap();
    render_diagnostics(&opened.sources, &opened.diagnostics)
        .replace(&format!("{}/", workspace().display()), "")
}

/// A kind that bores one 8mm hole, 10mm deep, into `into` where `piece`
/// meets it, leaving at least `wall`; and a side standing on the left end
/// of the base, joined to it by the kind. The hole is centred under the
/// side's 19mm end, so its side is 5.5mm from the base's left end; its
/// bottom is 9mm above the base's bottom face.
fn bored(wall: &str) -> String {
    format!(
        "kind bored : joint {{
  insert: [push(piece, along: length)]
  cut hole: crossing(into, piece) {{ drill(diameter: 8mm, depth: 10mm, wall: {wall}) }}
}}
part side : pine_19 {{
  length: 400mm
  width:  300mm
  place {{
    orient(length: up, width: back)
    against(base.top)
    flush(base.left)
    flush(base.front)
  }}
}}
joint side_hole : bored(side, into: base)
require holds : 1mm == 1mm \"a rule of the project\""
    )
}

// ---------------------------------------------------------------------------
// A cut's wall:
// ---------------------------------------------------------------------------

#[test]
fn a_wall_to_a_face_is_exact() {
    holds(&bored("5.5mm"));
}

#[test]
fn a_wall_too_thin_says_what_it_measured_and_where() {
    let opened = open(&bored("5.6mm"));
    let found: Vec<_> = opened
        .diagnostics
        .iter()
        .filter(|d| d.code == stave_lang::Code::Wall)
        .collect();
    assert_eq!(found.len(), 1, "{:?}", diagnostics(&opened));
    let diagnostic = found[0];
    assert_eq!(
        diagnostic.message,
        "side_hole: its hole in base leaves 5.5mm of wood at the left end of base; it needs 5.6mm"
    );
    assert_eq!(diagnostic.notes, ["for side and base in side_hole"]);
    assert_eq!(
        diagnostic.help.as_deref(),
        Some(
            "move the parts so the hole has 5.6mm of wood round it, or change the joint's settings"
        )
    );
}

#[test]
fn the_retired_wall_function_is_an_error_that_says_what_replaces_it() {
    let opened = open(&format!(
        "{}\nrequire outside : wall(hole) >= 1mm \"not in a kind\"",
        bored("0mm")
    ));
    assert!(
        diagnostics(&opened).contains(
            &"unknown-name: 'wall(...)' was retired: a cut says the least wall it needs".to_owned()
        ),
        "{:?}",
        diagnostics(&opened)
    );
}

// ---------------------------------------------------------------------------
// Held
// ---------------------------------------------------------------------------

/// A shelf lying on the base, joined to nothing.
const SHELF: &str = "part shelf : pine_19 {
  length: 200mm
  width:  100mm
  place {
    against(base.top)
    flush(base.left)
    flush(base.front)
  }
}
";

fn with_every_warning(body: &str) -> Vec<String> {
    let mut opened =
        open_files_with_every_warning(&[("project.stave", &format!("{HEADER}\n{body}"))]);
    opened
        .diagnostics
        .retain(|d| !common::is_no_max_length_warning(d));
    diagnostics(&opened)
}

#[test]
fn a_part_nothing_joins_to_the_ground_part_is_loose() {
    assert_eq!(
        with_every_warning(SHELF),
        [
            "loose-part: shelf is loose: no joint or hardware joins it to base or to anything joined to base"
        ]
    );
}

#[test]
fn a_part_meant_to_lift_out_says_so() {
    let loose = SHELF.replace("  width:  100mm\n", "  width:  100mm\n  loose:  true\n");
    assert_eq!(with_every_warning(&loose), Vec::<String>::new());
}

#[test]
fn one_loose_part_lets_its_whole_group_lift_out() {
    // A tray: a bottom with a rim screwed to it, lifting out of the box
    // as one; only the bottom says so.
    let tray = "part bottom : pine_12 {
  length: 200mm
  width:  100mm
  loose:  true
  place {
    against(base.top)
    flush(base.left)
    flush(base.front)
  }
}
part rim : pine_19 {
  length: 200mm
  width:  40mm
  place {
    orient(length: right, width: up)
    against(bottom.top)
    flush(bottom.left)
    flush(bottom.front)
  }
}
joint rim_screws : screw(bottom, into: rim, length: 25mm)";
    assert_eq!(with_every_warning(tray), Vec::<String>::new());
    let held_by_nothing = tray.replace("  loose:  true\n", "");
    assert_eq!(
        with_every_warning(&held_by_nothing),
        [
            "loose-part: bottom and rim are joined only to each other: no joint or hardware joins them to base or to anything joined to base"
        ]
    );
}

#[test]
fn a_joint_holds_its_parts() {
    let joined = format!("{SHELF}joint shelf_screws : screw(base, into: shelf, length: 30mm)");
    assert_eq!(with_every_warning(&joined), Vec::<String>::new());
}

// ---------------------------------------------------------------------------
// The broken designs stage 3 closes
// ---------------------------------------------------------------------------

#[test]
fn a_socket_at_the_edge_of_a_side() {
    insta::assert_snapshot!(rendered("tests/broken/socket_at_the_edge"));
}

#[test]
fn a_socket_against_a_dado() {
    insta::assert_snapshot!(rendered("tests/broken/socket_against_dado"));
}

#[test]
fn a_mortise_open_at_the_top_of_its_leg() {
    insta::assert_snapshot!(rendered("tests/broken/mortise_open_at_the_top"));
}

#[test]
fn a_mortise_wall_half_a_millimetre_thick() {
    insta::assert_snapshot!(rendered("tests/broken/mortise_wall_too_thin"));
}

#[test]
fn rung_holes_too_close_in_a_turned_leg() {
    insta::assert_snapshot!(rendered("tests/broken/rung_holes_too_close"));
}

#[test]
fn a_top_held_by_nothing() {
    insta::assert_snapshot!(rendered("tests/broken/top_held_by_nothing"));
}

#[test]
fn slats_held_by_nothing() {
    insta::assert_snapshot!(rendered("tests/broken/slats_held_by_nothing"));
}

#[test]
fn a_hole_that_breaks_out_of_its_board() {
    insta::assert_snapshot!(rendered("tests/broken/hole_breaks_out"));
}

#[test]
fn two_empty_holes_that_run_together() {
    insta::assert_snapshot!(rendered("tests/broken/holes_run_together"));
}

#[test]
fn a_dowel_into_another_joints_empty_hole() {
    insta::assert_snapshot!(rendered("tests/broken/dowel_into_empty_hole"));
}

#[test]
fn a_tenon_bigger_than_its_part() {
    insta::assert_snapshot!(rendered("tests/broken/tenon_bigger_than_part"));
}

#[test]
fn a_groove_through_the_wall() {
    insta::assert_snapshot!(rendered("tests/broken/groove_through_the_wall"));
}

#[test]
fn a_kinds_own_rule_speaks_before_the_backstop() {
    // std's groove at the wall's full depth: its not_too_deep rule, not
    // the stopped-cut backstop; and std's mortise_tenon with a tenon
    // thicker than the rail: tenon_thin, not "cuts only remove wood".
    for (folder, expected) in [
        (
            "tests/broken/groove_too_deep",
            "'not_too_deep' fails: a groove deeper than half the board weakens it",
        ),
        (
            "tests/broken/tenon_too_thick",
            "'tenon_thin' fails: a tenon thicker than half the rail leaves its cheeks too thin",
        ),
    ] {
        let opened = stave_app::open(&workspace().join(folder)).unwrap();
        let messages: Vec<String> = opened
            .diagnostics
            .iter()
            .filter(|d| d.is_error())
            .map(|d| d.message.clone())
            .collect();
        assert_eq!(messages, [expected], "{folder}");
    }
}
