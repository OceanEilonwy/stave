//! Grain and wood movement (grammar, "Grain and wood movement"), end to
//! end on small designs: glue on end grain, glue across the grain, a tenon
//! with the grain across it, and a solid part held fast too far apart
//! across its grain; and the designs that look like them but are sound.

mod common;

use common::{Checked, check};
use stave_lang::{Code, Severity};

/// A top 600 x 400 x 19 of solid oak lying flat, its grain left to right
/// (so it moves front to back), and whatever `more` adds.
fn design(more: &str) -> String {
    format!(
        r#"stave 1

project "Grain" {{
  units: mm
}}

stock oak    {{ species: white_oak, form: solid, thickness: 19mm, max_width: 500mm, max_length: 2400mm }}
stock oak_25 {{ species: white_oak, form: solid, thickness: 25mm, max_width: 200mm, max_length: 2400mm }}
stock oak_45 {{ species: white_oak, form: solid, thickness: 45mm, width: 45mm, max_length: 2400mm }}
stock ply    {{ species: birch, form: plywood, thickness: 18mm, max_width: 1220mm, max_length: 2440mm }}

part top : oak {{
  length: 600mm
  width:  400mm
  place {{
    ground
  }}
}}
{more}
"#
    )
}

/// The grain warnings, as `message` then `help`.
fn grain(checked: &Checked) -> Vec<(String, String)> {
    let errors: Vec<String> = checked
        .diagnostics
        .iter()
        .filter(|d| d.is_error())
        .map(|d| format!("{}: {}", d.code, d.message))
        .collect();
    assert!(errors.is_empty(), "{errors:#?}");
    checked
        .diagnostics
        .iter()
        .filter(|d| d.code == Code::Grain)
        .map(|d| {
            assert_eq!(d.severity, Severity::Warning);
            (d.message.clone(), d.help.clone().unwrap_or_default())
        })
        .collect()
}

/// The grain warnings' messages only.
fn messages(checked: &Checked) -> Vec<String> {
    grain(checked).into_iter().map(|(m, _)| m).collect()
}

/// A batten under the top, 50mm in from its left end, running front to
/// back: its grain runs the way the top moves.
const BATTEN: &str = r#"
part batten : oak_25 {
  length: 400mm
  width:  50mm
  place {
    orient(length: back, width: right)
    against(top.bottom)
    flush(top.left, inset: 50mm)
    flush(top.front)
  }
}
"#;

#[test]
fn glue_across_the_grain_is_warned_at_the_joint() {
    let checked = check(&design(&format!(
        "{BATTEN}\njoint glued : glued_face(batten, into: top)"
    )));
    assert_eq!(
        grain(&checked),
        [(
            "glued: top is glued across its grain to batten over 400mm, and wood held that wide splits as it moves with the seasons".to_owned(),
            "glue only a strip no wider than 80mm across the grain of top, or hold it with fasteners that let it move".to_owned()
        )]
    );
}

#[test]
fn a_strip_glued_along_the_grain_is_sound() {
    // A cleat running left to right, the way the top's grain runs: the
    // two move together, so the glue is sound however wide.
    let checked = check(&design(
        r#"
part cleat : oak {
  length: 500mm
  width:  100mm
  place {
    against(top.bottom)
    center(top, across)
    flush(top.front)
  }
}
joint glued : glued_face(cleat, into: top)"#,
    ));
    assert_eq!(messages(&checked), Vec::<String>::new());
}

#[test]
fn glue_across_the_grain_no_wider_than_80mm_is_sound() {
    // The same batten across a top only 80mm wide.
    let checked = check(&format!(
        "{}\njoint glued : glued_face(strip, into: top)",
        design(
            r#"
part strip : oak_25 {
  length: 80mm
  width:  50mm
  place {
    orient(length: back, width: right)
    against(top.bottom)
    flush(top.left, inset: 50mm)
    flush(top.front)
  }
}"#
        )
        .replace("width:  400mm", "width:  80mm")
    ));
    assert_eq!(messages(&checked), Vec::<String>::new());
}

#[test]
fn a_solid_part_glued_to_plywood_is_held_still_by_it() {
    // Plywood doesn't move, so it holds the top still whichever way.
    let checked = check(&design(
        r#"
part panel : ply {
  length: 600mm
  width:  400mm
  place {
    against(top.bottom)
    flush(top.left)
    flush(top.front)
  }
}
joint glued : glued_face(top, into: panel)"#,
    ));
    assert_eq!(
        messages(&checked),
        [
            "glued: top is glued across its grain to panel over 400mm, and wood held that wide splits as it moves with the seasons"
        ]
    );
}

#[test]
fn glue_on_end_grain_is_warned_at_the_joint() {
    let checked = check(&design(
        r#"
part leg : oak_45 {
  length: 700mm
  place {
    orient(length: down, width: right)
    against(top.bottom)
    flush(top.left)
    flush(top.front)
  }
}
joint stuck : glued_face(leg, into: top)"#,
    ));
    assert_eq!(
        grain(&checked),
        [(
            "stuck: leg meets top with its end grain, and glue on end grain holds almost nothing".to_owned(),
            "join them with a kind that cuts the parts or places fasteners in them, so the glue holds on long grain, or turn leg so it meets top with a face or an edge".to_owned()
        )]
    );
}

#[test]
fn an_end_glued_and_doweled_holds_on_its_dowels() {
    // The glue goes in the dowels' holes: not measured.
    let checked = check(&design(
        r#"
part leg : oak_45 {
  length: 700mm
  place {
    orient(length: down, width: right)
    against(top.bottom)
    flush(top.left)
    flush(top.front)
  }
}
joint pins : dowel(leg, into: top, count: 1)"#,
    ));
    assert_eq!(messages(&checked), Vec::<String>::new());
}

/// An apron tenoned into a leg standing under the top, its grain `grain`;
/// the apron is screwed up into the top, and holds the leg.
fn tenoned(grain: &str) -> String {
    design(&format!(
        r#"
part leg : oak_45 {{
  length: 700mm
  place {{
    orient(length: down, width: right)
    against(top.bottom)
    flush(top.left)
    flush(top.front)
  }}
}}
part apron : oak {{
  length: 300mm
  width:  90mm
  grain:  {grain}
  place {{
    orient(length: right, width: up)
    against(leg.right)
    against(top.bottom)
    center(leg, deep)
  }}
}}
joint tenons : mortise_tenon(apron, into: leg, thickness: 8mm, length: 20mm)
joint aprons : screw(top, into: apron, count: 1, length: 40mm, glue: false)"#
    ))
}

#[test]
fn a_tenon_with_the_grain_across_it_is_short_grain() {
    let checked = check(&tenoned("width"));
    assert_eq!(
        grain(&checked),
        [(
            "tenons: the grain of apron runs across its tenon, so the tenon can snap off along the grain".to_owned(),
            "make apron with its grain along its length (grain: length), so the grain runs along the tenon".to_owned()
        )]
    );
}

#[test]
fn a_tenon_with_the_grain_along_it_is_sound() {
    let checked = check(&tenoned("length"));
    assert_eq!(messages(&checked), Vec::<String>::new());
}

/// The batten screwed to the top by `kind`, three screws, no glue.
fn screwed(kind: &str) -> String {
    design(&format!(
        "{BATTEN}\njoint screws : {kind}(batten, into: top, length: 35mm, count: 3, glue: false)"
    ))
}

#[test]
fn screws_in_round_holes_across_the_grain_hold_the_top_fast() {
    let checked = check(&screwed("screw"));
    let found: Vec<_> = checked
        .diagnostics
        .iter()
        .filter(|d| d.code == Code::Grain)
        .collect();
    let [only] = found.as_slice() else {
        panic!("{:#?}", checked.messages());
    };
    assert_eq!(
        only.message,
        "top is held fast at places 201.5mm apart across its grain, so it can't swell and shrink with the seasons"
    );
    assert_eq!(only.notes, ["held to batten by screws"]);
    assert_eq!(
        only.help.as_deref(),
        Some(
            "hold top fast along one line only and let it slide everywhere else: fasten it through slots, use fittings made to let it move, or keep its fixings within 80mm of each other across its grain"
        )
    );
}

#[test]
fn screws_through_slots_let_the_top_move() {
    let checked = check(&screwed("slotted_screw"));
    assert_eq!(messages(&checked), Vec::<String>::new());
}

/// Two cleats under the top, at its front and back edges, running left to
/// right, screwed up into it through round holes; `join` joins them to
/// each other or not.
fn two_cleats(join: &str) -> String {
    design(&format!(
        r#"
part cleat_front : oak {{
  length: 500mm
  width:  30mm
  place {{
    against(top.bottom)
    center(top, across)
    flush(top.front)
  }}
}}
part cleat_back : oak {{
  length: 500mm
  width:  30mm
  place {{
    against(top.bottom)
    center(top, across)
    flush(top.back)
  }}
}}
joint screws : screw([cleat_front, cleat_back], into: top, count: 3, glue: false)
{join}"#
    ))
}

#[test]
fn two_parts_joined_to_each_other_hold_the_top_fast_between_them() {
    let checked = check(&two_cleats(
        r#"
part tie : oak {
  length: 400mm
  width:  60mm
  place {
    orient(length: back, width: right)
    against(cleat_front.bottom)
    center(top, across)
    flush(top.front)
  }
}
joint ties : glued_face(tie, into: [cleat_front, cleat_back])"#,
    ));
    let found: Vec<_> = checked
        .diagnostics
        .iter()
        .filter(|d| d.code == Code::Grain)
        .collect();
    let [only] = found.as_slice() else {
        panic!("{:#?}", checked.messages());
    };
    assert_eq!(
        only.message,
        "top is held fast at places 370mm apart across its grain, so it can't swell and shrink with the seasons"
    );
    assert_eq!(
        only.notes,
        [
            "held to cleat_front by screws",
            "held to cleat_back by screws"
        ]
    );
}

#[test]
fn two_parts_joined_only_through_the_top_move_with_it() {
    let checked = check(&two_cleats(""));
    assert_eq!(messages(&checked), Vec::<String>::new());
}

#[test]
fn a_part_of_unglued_boards_moves_board_by_board() {
    // Three boards of about 133mm, left dry: each screw holds one board.
    let text = screwed("screw").replace(
        "  width:  400mm\n",
        "  width:  400mm\n  boards: joined(by: tongue_and_groove(glue: false))\n",
    );
    let text = text.replace(
        "stock oak    { species: white_oak, form: solid, thickness: 19mm, max_width: 500mm,",
        "stock oak    { species: white_oak, form: solid, thickness: 19mm, max_width: 150mm,",
    );
    let checked = check(&text);
    assert_eq!(messages(&checked), Vec::<String>::new());
}

/// A panel lying on a plywood base between two strips, in their grooves; the
/// strips are screwed to the base, so they are joined other than through
/// the panel.
fn panel(glue: &str) -> String {
    format!(
        r#"stave 1

project "Panel" {{
  units: mm
}}

stock oak    {{ species: white_oak, form: solid, thickness: 19mm, max_width: 500mm, max_length: 2400mm }}
stock oak_12 {{ species: white_oak, form: solid, thickness: 12mm, max_width: 500mm, max_length: 2400mm }}
stock ply    {{ species: birch, form: plywood, thickness: 18mm, max_width: 1220mm, max_length: 2440mm }}

part base : ply {{
  length: 600mm
  width:  300mm
  place {{
    ground
  }}
}}
part strip_front : oak {{
  length: 600mm
  width:  30mm
  place {{
    against(base.top)
    flush(base.left)
    flush(base.front)
  }}
}}
part strip_back : oak {{
  length: 600mm
  width:  30mm
  place {{
    against(base.top)
    flush(base.left)
    flush(base.back)
  }}
}}
part panel : oak_12 {{
  length: 600mm
  width:  fit
  place {{
    against(base.top)
    flush(base.left)
    against(strip_front.back)
  }}
}}
joint strips : screw([strip_front, strip_back], into: base, count: 3, glue: false)
joint housed : groove(panel, into: [strip_front, strip_back]{glue})
joint rests  : glued_face(panel, into: base, glue: false)"#
    )
}

#[test]
fn a_solid_panel_glued_into_its_grooves_is_held_fast() {
    let checked = check(&panel(""));
    assert_eq!(
        messages(&checked),
        [
            "panel is held fast at places 240mm apart across its grain, so it can't swell and shrink with the seasons"
        ]
    );
}

#[test]
fn a_solid_panel_left_dry_in_its_grooves_is_sound() {
    let checked = check(&panel(", glue: false"));
    assert_eq!(messages(&checked), Vec::<String>::new());
}
