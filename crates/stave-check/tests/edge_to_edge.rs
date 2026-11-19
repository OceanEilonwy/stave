//! `edge_to_edge(a, b)` (grammar, "Built-in functions"), end to end on
//! small designs: two parts lying side by side in one plane, an edge of
//! each against the other, and every way of meeting that is not that. And
//! the standard edge joints' rule that asks it, which holds at every seam
//! of a part made of boards and refuses a corner on its own.

mod common;

use common::{Checked, check};
use stave_lang::{Code, Diagnostic};

/// A base 600 x 300 x 19 lying flat on the floor, its length across and
/// its width deep, and whatever `more` adds.
fn design(more: &str) -> String {
    format!(
        r#"stave 1

project "Meetings" {{
  units: mm
}}

stock pine_19 {{ species: pine, form: solid, thickness: 19mm, max_length: 2400mm }}
stock pine_12 {{ species: pine, form: solid, thickness: 12mm, max_length: 2400mm }}

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

/// The errors, as `code: message`; the parts here are mostly not joined,
/// so warnings (a loose part) are left out.
fn errors(checked: &Checked) -> Vec<String> {
    checked
        .diagnostics
        .iter()
        .filter(|d| d.is_error())
        .map(|d: &Diagnostic| format!("{}: {}", d.code, d.message))
        .collect()
}

/// `edge_to_edge(a, b)` in a design with `more`, read through a project
/// rule that holds only when it is `true`, and its mirror that holds only
/// when it is `false`: exactly one of them fails, and nothing else does.
fn edge_to_edge(more: &str, a: &str, b: &str) -> bool {
    let text = design(&format!(
        "{more}
require yes : edge_to_edge({a}, {b}) \"edge to edge\"
require no  : !edge_to_edge({a}, {b}) \"not edge to edge\""
    ));
    let checked = check(&text);
    let errors = errors(&checked);
    let failed_yes = errors.contains(&"require: 'yes' fails: edge to edge".to_owned());
    let failed_no = errors.contains(&"require: 'no' fails: not edge to edge".to_owned());
    assert_eq!(errors.len(), 1, "{errors:#?}");
    assert_ne!(failed_yes, failed_no, "{errors:#?}");
    failed_no
}

/// A second board lying behind the base, its front edge on the base's
/// back edge.
const BEHIND: &str = "part next : pine_19 {
  length: 600mm
  width:  200mm
  place {
    against(base.back)
    flush(base.left)
    flush(base.bottom)
  }
}";

#[test]
fn two_boards_side_by_side_are_edge_to_edge_either_way_round() {
    assert!(edge_to_edge(BEHIND, "base", "next"));
    assert!(edge_to_edge(BEHIND, "next", "base"));
}

#[test]
fn a_board_s_end_against_another_s_face_is_not_edge_to_edge() {
    // A side standing on the base's left end: its end meets the base's
    // top face, at a corner.
    let side = "part side : pine_19 {
  length: 400mm
  width:  300mm
  place {
    orient(length: up, width: back)
    against(base.top)
    flush(base.left)
    flush(base.front)
  }
}";
    assert!(!edge_to_edge(side, "side", "base"));
    assert!(!edge_to_edge(side, "base", "side"));
}

#[test]
fn an_edge_against_an_end_in_one_plane_is_not_edge_to_edge() {
    // A breadboard end: a cleat whose edge runs along the base's right
    // end, in the base's plane.
    let cleat = "part cleat : pine_19 {
  length: 300mm
  width:  60mm
  place {
    orient(length: back, width: right)
    against(base.right)
    flush(base.front)
    flush(base.bottom)
  }
}";
    assert!(!edge_to_edge(cleat, "cleat", "base"));
    assert!(!edge_to_edge(cleat, "base", "cleat"));
}

#[test]
fn two_edges_meeting_at_right_angles_are_not_edge_to_edge() {
    // A board standing behind the base, its thickness across: its front
    // edge meets the base's back edge, but the two don't lie in one plane.
    let upright = "part upright : pine_19 {
  length: 400mm
  width:  100mm
  place {
    orient(length: up, width: back)
    against(base.back)
    flush(base.left)
    flush(base.bottom)
  }
}";
    assert!(!edge_to_edge(upright, "upright", "base"));
}

#[test]
fn two_boards_apart_are_not_edge_to_edge_and_that_is_no_error() {
    let apart = "part away : pine_19 {
  length: 600mm
  width:  200mm
  place {
    against(base.back, gap: 10mm)
    flush(base.left)
    flush(base.bottom)
  }
}";
    assert!(!edge_to_edge(apart, "away", "base"));
}

#[test]
fn boards_of_different_thickness_side_by_side_are_edge_to_edge() {
    let thin = "part thin : pine_12 {
  length: 600mm
  width:  200mm
  place {
    against(base.back)
    flush(base.left)
    flush(base.bottom)
  }
}";
    assert!(edge_to_edge(thin, "thin", "base"));
}

#[test]
fn edge_to_edge_needs_one_part_on_each_side() {
    let checked = check(&design(&format!(
        "{BEHIND}\nrequire both : edge_to_edge([base, next], next) \"one part each\""
    )));
    assert!(
        checked
            .diagnostics
            .iter()
            .any(|d| d.code == Code::TypeMismatch
                && d.message == "'edge_to_edge' needs one part, found parts"),
        "{:#?}",
        errors(&checked)
    );
}

/// A top 450mm wide of 150mm stock: three boards, joined by the standard
/// edge joint unless `boards` says otherwise.
fn wide_top(boards: &str, grain: &str) -> String {
    format!(
        r#"stave 1

project "Wide top" {{
  units: mm
}}

stock oak_19 {{ species: white_oak, form: solid, thickness: 19mm, max_width: 150mm, max_length: 2400mm }}

part top : oak_19 {{
  length: 450mm
  width:  450mm
{grain}{boards}  place {{
    ground
  }}
}}
"#
    )
}

#[test]
fn the_boards_of_a_wide_part_are_edge_to_edge_at_every_seam() {
    for boards in [
        "",
        "  boards: joined(by: tongue_and_groove)\n",
        "  boards: joined(by: splined_edge)\n",
        "  boards: joined(by: chigiri(glue: false))\n",
    ] {
        for grain in ["", "  grain: width\n"] {
            // On a part whose grain runs across its length, a tongue is
            // refused and a spline or a key is laid the wrong way, before
            // this rule and without it (docs/STATUS.md, "Boards"): only
            // the plain glued edge is made there.
            if grain.contains("width") && !boards.is_empty() {
                continue;
            }
            let checked = check(&wide_top(boards, grain));
            let design = checked.clean();
            let boards_of = design
                .part_named("top")
                .and_then(|p| p.boards.as_ref())
                .unwrap_or_else(|| panic!("top is made of boards ({boards}{grain})"));
            let side_by_side: Vec<_> = boards_of
                .requires
                .iter()
                .filter(|r| r.name.ends_with(".side_by_side"))
                .collect();
            assert_eq!(side_by_side.len(), 2, "two seams ({boards}{grain})");
            assert!(side_by_side.iter().all(|r| r.holds), "{boards}{grain}");
            assert!(checked.diagnostics.is_empty(), "{boards}{grain}");
        }
    }
}

#[test]
fn an_edge_joint_between_two_boards_side_by_side_checks_clean() {
    for kind in ["glued_edge", "tongue_and_groove", "splined_edge"] {
        let checked = check(&design(&format!(
            "{BEHIND}\njoint seam : {kind}(next, into: base)"
        )));
        assert_eq!(errors(&checked), Vec::<String>::new(), "{kind}");
    }
}

#[test]
fn an_edge_joint_at_a_corner_is_its_rule_alone() {
    let side = "part side : pine_19 {
  length: 400mm
  width:  300mm
  place {
    orient(length: up, width: back)
    against(base.top)
    flush(base.left)
    flush(base.front)
  }
}";
    for (kind, message) in [
        (
            "chigiri",
            "a butterfly key joins two boards lying side by side, edge to edge; these meet another way",
        ),
        (
            "glued_edge",
            "an edge joint glues two boards lying side by side, edge to edge; these meet another way",
        ),
        (
            "splined_edge",
            "a spline joins two boards lying side by side, edge to edge; these meet another way",
        ),
        (
            "tongue_and_groove",
            "a tongue and groove joins two boards lying side by side, edge to edge; these meet another way",
        ),
    ] {
        let checked = check(&design(&format!(
            "{side}\njoint corner : {kind}(side, into: base)"
        )));
        assert_eq!(
            errors(&checked),
            vec![format!("require: 'side_by_side' fails: {message}")],
            "{kind}"
        );
        let only = checked.diagnostics.iter().find(|d| d.is_error()).unwrap();
        assert_eq!(
            only.notes,
            [
                "for side and base in corner",
                "side's end meets base's face at right angles"
            ],
            "{kind}"
        );
    }
}
