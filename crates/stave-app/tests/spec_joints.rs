//! Executable spec: "Joints and hardware" (calls, kind values, roles and
//! pairs), "Defining joints" (params, roles, require, insert, cut_first,
//! adds, uses, makes, cut selectors and operations), "Hardware" (kinds,
//! bases and members) and "Work cards and recipes" (as far as names and
//! types: choosing recipes and ordering work are later steps).

mod common;

use common::{
    body_line, design, diagnostics, extent, holds, joint, mm, only_error, open, part, solved,
    the_diagnostic,
};
use stave_lang::placement::PartAxis;
use stave_model::solved::{Contact, CutKind, CutPlace, Depth, PartOrigin, Slope, StepKind};

/// A side standing at the left end of the base and a shelf against it.
const SIDE_AND_SHELF: &str = "part side : pine_19 {
  length: 400mm
  width:  300mm
  place {
    orient(length: up, width: back)
    against(base.top)
    flush(base.left)
    flush(base.front)
  }
}
part shelf : pine_19 {
  length: 200mm
  width:  300mm
  place {
    against(base.top, gap: 100mm)
    against(side.right)
    flush(base.front)
  }
}
";

fn with_parts(rest: &str) -> stave_app::Opened {
    open(&format!("{SIDE_AND_SHELF}{rest}"))
}

// ---------------------------------------------------------------------------
// Calls: roles and settings
// ---------------------------------------------------------------------------

#[test]
fn the_positional_argument_binds_the_first_role() {
    let opened = with_parts("joint housing : dado(shelf, into: side)");
    let design = design(&opened);
    let pair = &joint(design, "housing").pairs[0];
    let roles: Vec<(&str, &str)> = pair
        .roles
        .iter()
        .map(|r| (r.role.as_str(), design.part(r.part).unwrap().name.as_str()))
        .collect();
    assert_eq!(roles, [("piece", "shelf"), ("into", "side")]);
}

#[test]
fn a_kind_call_takes_at_most_one_positional_argument() {
    // One mistake, one error: the extra argument isn't also a missing role.
    let opened = with_parts("joint housing : dado(shelf, side)");
    let diagnostic = the_diagnostic(
        &opened,
        "type-mismatch",
        "a kind call takes at most one argument without a name",
    );
    assert_eq!(
        diagnostic.help.as_deref(),
        Some("name every other role and setting: into: ...")
    );
}

#[test]
fn a_role_that_is_not_given_is_a_missing_field() {
    the_diagnostic(
        &with_parts("joint housing : dado(shelf)"),
        "missing-field",
        "joint 'housing' is missing 'into'",
    );
}

#[test]
fn a_role_or_setting_given_twice_is_a_duplicate_name() {
    let opened = with_parts("joint housing : dado(shelf, piece: shelf, into: side)");
    the_diagnostic(&opened, "duplicate-name", "'piece' is declared twice");
    let opened = with_parts("joint housing : dado(shelf, into: side, depth: 3mm, depth: 4mm)");
    let diagnostic = the_diagnostic(&opened, "duplicate-name", "'depth' is declared twice");
    assert_eq!(
        diagnostic.help.as_deref(),
        Some("give 'depth' once; remove one of them")
    );
}

#[test]
fn a_part_role_takes_one_part_and_a_parts_role_one_or_a_list() {
    let opened = with_parts(
        "kind pair_joint : joint {\n  roles {\n    a: Part\n    b: Part\n  }\n}\njoint j : pair_joint([shelf, base], b: side)",
    );
    only_error(&opened, "type-mismatch", "'a' needs a part, found parts");
    let opened = with_parts(
        "region bay = opening(left: base.left, right: base.right, bottom: base.top, top: side.top, front: base.front, back: base.back)\njoint j : dado(bay, into: side)",
    );
    only_error(
        &opened,
        "type-mismatch",
        "'piece' needs parts, found a region",
    );
}

#[test]
fn a_joint_needs_a_joint_kind_and_hardware_a_hardware_kind() {
    only_error(
        &with_parts("joint j : knob(shelf)"),
        "type-mismatch",
        "'joint' needs a joint kind, found a pull kind",
    );
    only_error(
        &with_parts("hardware h : dado(shelf, into: side)"),
        "type-mismatch",
        "'hardware' needs a hardware kind, found a joint kind",
    );
    only_error(
        &with_parts("hardware h : dowel_pin(shelf)"),
        "type-mismatch",
        "'hardware' needs a hardware kind, found a fastener kind",
    );
    only_error(
        &with_parts("hardware h : chisel(shelf)"),
        "type-mismatch",
        "'hardware' needs a hardware kind, found a tool",
    );
    let opened = with_parts("joint j : dadoo(shelf, into: side)");
    only_error(&opened, "unknown-name", "unknown name 'dadoo'");
    assert_eq!(common::help(&opened), "did you mean 'dado'?");
}

#[test]
fn every_joint_accepts_glue_and_methods() {
    let opened = with_parts(
        "joint a : dado(shelf, into: side)\njoint b : dado(shelf, into: side, glue: false, methods: [notch_by_hand])",
    );
    let design = solved(&opened);
    assert!(joint(design, "a").glue, "glue defaults to true");
    assert!(joint(design, "a").methods.is_empty());
    assert!(!joint(design, "b").glue);
    assert_eq!(joint(design, "b").methods, ["notch_by_hand"]);
    only_error(
        &with_parts("joint j : dado(shelf, into: side, glue: 1)"),
        "type-mismatch",
        "'glue' needs true or false, found a count",
    );
    only_error(
        &with_parts("joint j : dado(shelf, into: side, methods: [chisel])"),
        "type-mismatch",
        "'methods' needs a recipe, found a tool",
    );
}

#[test]
fn a_kind_can_not_declare_glue_or_methods() {
    the_diagnostic(
        &open("kind k : joint {\n  param glue = true\n}"),
        "duplicate-name",
        "'glue' is declared twice",
    );
    the_diagnostic(
        &open("kind k : joint {\n  roles {\n    methods: Part\n    b:       Part\n  }\n}"),
        "duplicate-name",
        "'methods' is declared twice",
    );
}

#[test]
fn a_kind_value_supplies_settings_and_the_call_adds_roles() {
    let opened =
        with_parts("let shallow = dado(depth: 3mm)\njoint housing : shallow(shelf, into: side)");
    // The shelf is housed at its left end only.
    assert_eq!(part(design(&opened), "shelf").cut.length, mm("203"));
    the_diagnostic(
        &with_parts(
            "let shallow = dado(depth: 3mm)\njoint housing : shallow(shelf, into: side, depth: 4mm)",
        ),
        "duplicate-name",
        "'depth' is declared twice",
    );
}

#[test]
fn a_kind_param_without_a_default_must_be_given() {
    the_diagnostic(
        &with_parts("kind k : joint {\n  param depth : Length\n}\njoint j : k(shelf, into: side)"),
        "missing-field",
        "joint 'j' is missing 'depth'",
    );
}

#[test]
fn a_setting_outside_the_kinds_within_is_out_of_range() {
    let opened = with_parts(
        "kind k : joint {\n  param depth = 3mm within 1mm..5mm\n}\njoint j : k(shelf, into: side, depth: 6mm)",
    );
    let diagnostic = the_diagnostic(
        &opened,
        "out-of-range",
        "'depth' must be within 1mm..5mm, found 6mm",
    );
    assert_eq!(body_line(&opened, diagnostic), 23);
}

// ---------------------------------------------------------------------------
// A kind chosen through a param that doesn't take what the call hands it
// ---------------------------------------------------------------------------

/// A component that houses a board in a part a set depth, taking its joint
/// kind as a setting, as std's Shelf does; then `rest`.
const HOUSED: &str = "component Housed {
  param support     : Part
  param board_stock : Stock
  param joinery     : JointKind = dado
  part board : board_stock {
    length: 200mm
    width:  300mm
    place {
      against(support.top, gap: 100mm)
      flush(support.left)
      flush(support.front)
    }
  }
  joint housing : joinery(board, into: support, depth: 9mm)
}
";

#[test]
fn a_kind_written_in_the_joint_with_a_setting_it_lacks_keeps_its_message() {
    // The user wrote `depth:` for this kind: that is the thing to say.
    let opened = with_parts("joint housing : glued_edge(shelf, into: side, depth: 9mm)");
    let diagnostic = the_diagnostic(
        &opened,
        "unknown-field",
        "the kind 'glued_edge' has no field 'depth'",
    );
    assert!(diagnostic.cause.is_none());
    // And a role it lacks.
    let opened = with_parts(
        "kind battens : joint {\n  roles {\n    boards: Parts\n  }\n}\njoint j : battens(shelf, into: side)",
    );
    only_error(
        &opened,
        "unknown-field",
        "the kind 'battens' has no field 'into'",
    );
    // And several parts where it takes one.
    let opened = with_parts("joint j : half_lap([shelf, base], into: side)");
    only_error(
        &opened,
        "type-mismatch",
        "'piece' needs a part, found parts",
    );
}

#[test]
fn a_kind_chosen_for_a_component_that_lacks_a_setting_it_hands_is_one_error_at_the_choice() {
    let opened = open(&format!(
        "{HOUSED}assembly housed : Housed {{\n  support:     base\n  board_stock: pine_19\n  joinery:     glued_edge\n}}\n"
    ));
    let diagnostic = the_diagnostic(
        &opened,
        "type-mismatch",
        "'glued_edge' can't be the joinery of Housed",
    );
    // At the choice, with the component's line as the second place.
    assert_eq!(body_line(&opened, diagnostic), 19);
    let second = diagnostic.labels.get(1).expect("the component's line");
    assert_eq!(second.message, "Housed uses joinery here");
    assert_eq!(
        diagnostic.reason(),
        Some("Housed gives its joint a depth, and glued_edge has no depth")
    );
    assert_eq!(
        diagnostic.notes,
        ["Housed gives its joint a depth, and glued_edge has no depth"]
    );
    let help = diagnostic.help.as_deref().unwrap();
    assert!(
        help.starts_with("choose a kind that takes what Housed hands its joint: "),
        "{help}"
    );
    assert!(
        help.contains("dado") && !help.contains("glued_edge"),
        "{help}"
    );
}

#[test]
fn a_default_that_cant_be_used_is_reported_at_the_default() {
    let housed = HOUSED.replace("JointKind = dado", "JointKind = screw");
    let opened = open(&format!(
        "{housed}assembly housed : Housed {{\n  support:     base\n  board_stock: pine_19\n}}\n"
    ));
    let diagnostic = the_diagnostic(
        &opened,
        "type-mismatch",
        "'screw' can't be the joinery of Housed",
    );
    assert_eq!(body_line(&opened, diagnostic), 4);
}

#[test]
fn a_projects_own_param_names_the_project_and_its_joint() {
    let opened = with_parts(
        "param joinery : JointKind = glued_edge\njoint housing : joinery(shelf, into: side, depth: 9mm)",
    );
    let diagnostic = the_diagnostic(
        &opened,
        "type-mismatch",
        "'glued_edge' can't be the project's joinery",
    );
    assert_eq!(
        diagnostic.reason(),
        Some("the project gives housing a depth, and glued_edge has no depth")
    );
}

#[test]
fn a_kind_that_needs_a_role_the_component_does_not_hand_it_is_one_error() {
    let opened = open(&format!(
        "kind three_way : joint {{\n  roles {{\n    piece: Part\n    into:  Part\n    brace: Part\n  }}\n  param depth = 5mm\n}}\n{HOUSED}assembly housed : Housed {{\n  support:     base\n  board_stock: pine_19\n  joinery:     three_way\n}}\n"
    ));
    let diagnostic = the_diagnostic(
        &opened,
        "type-mismatch",
        "'three_way' can't be the joinery of Housed",
    );
    assert_eq!(
        diagnostic.reason(),
        Some("Housed hands its joint nothing as brace, and three_way needs one part there")
    );
}

#[test]
fn a_component_passing_its_kind_on_is_reported_where_the_kind_was_chosen() {
    let opened = open(&format!(
        "{HOUSED}component Outer {{\n  param base_part : Part\n  param wood      : Stock\n  param housing   : JointKind = dado\n  assembly inner : Housed {{\n    support:     base_part\n    board_stock: wood\n    joinery:     housing\n  }}\n}}\nassembly outer : Outer {{\n  base_part: base\n  wood:      pine_19\n  housing:   glued_edge\n}}\n"
    ));
    let diagnostic = the_diagnostic(
        &opened,
        "type-mismatch",
        "'glued_edge' can't be the housing of Outer",
    );
    assert_eq!(body_line(&opened, diagnostic), 29);
    assert_eq!(
        diagnostic.reason(),
        Some("Housed gives its joint a depth, and glued_edge has no depth")
    );
}

// ---------------------------------------------------------------------------
// Roles and pairs
// ---------------------------------------------------------------------------

#[test]
fn roles_default_to_piece_and_into() {
    let opened = with_parts(
        "kind plain : joint {\n  note: \"no roles\"\n}\njoint j : plain(shelf, into: side)",
    );
    let pair = &joint(design(&opened), "j").pairs[0];
    assert_eq!(
        pair.roles
            .iter()
            .map(|r| r.role.as_str())
            .collect::<Vec<_>>(),
        ["piece", "into"]
    );
}

#[test]
fn a_kind_with_a_parts_role_has_one_role_or_two() {
    the_diagnostic(
        &open("kind k : joint {\n  roles {\n    a: Parts\n    b: Part\n    c: Part\n  }\n}"),
        "type-mismatch",
        "kind 'k' has a Parts role, so it has one role or two",
    );
    the_diagnostic(
        &open("kind k : joint {\n  roles {\n    a: Region\n    b: Part\n  }\n}"),
        "type-mismatch",
        "'a' needs Part or Parts, found 'Region'",
    );
}

#[test]
fn a_kind_with_three_part_roles_is_one_connection() {
    let opened = with_parts(
        "kind tri : joint {\n  roles {\n    a: Part\n    b: Part\n    c: Part\n  }\n}\njoint j : tri(shelf, b: side, c: base)",
    );
    let j = joint(design(&opened), "j");
    assert_eq!(j.pairs.len(), 1);
    assert_eq!(j.pairs[0].contact, Contact::Connection);
    assert_eq!(j.pairs[0].roles.len(), 3);
}

/// Three boards side by side on the base, edge to edge.
const THREE_BOARDS: &str = "part plank[a, b, c] : pine_19 {
  length: 300mm
  width:  100mm
  place {
    orient(length: back, width: right)
    against(base.top)
    spaced(gap(0mm), from: base.left, to: base.right)
    flush(base.front)
  }
}
";

#[test]
fn a_kind_with_one_parts_role_is_one_connection_across_its_parts() {
    let opened = open(&format!(
        "{THREE_BOARDS}kind clamped : joint {{\n  roles {{\n    boards: Parts\n  }}\n}}\njoint glue_up : clamped([plank[a], plank[b], plank[c]])"
    ));
    let design = design(&opened);
    let j = joint(design, "glue_up");
    assert_eq!(j.pairs.len(), 1);
    assert_eq!(j.pairs[0].contact, Contact::Connection);
    let bound: Vec<(&str, &str)> = j.pairs[0]
        .roles
        .iter()
        .map(|r| (r.role.as_str(), design.part(r.part).unwrap().name.as_str()))
        .collect();
    assert_eq!(
        bound,
        [
            ("boards", "plank[a]"),
            ("boards", "plank[b]"),
            ("boards", "plank[c]")
        ]
    );
    // Every part must meet at least one of the others.
    let opened = open(&format!(
        "{THREE_BOARDS}kind clamped : joint {{\n  roles {{\n    boards: Parts\n  }}\n}}\njoint glue_up : clamped([plank[a], plank[c]])"
    ));
    the_diagnostic(
        &opened,
        "no-contact",
        "glue_up: plank[a] meets none of the parts it should join",
    );
    assert!(
        opened.diagnostics[0]
            .notes
            .iter()
            .any(|n| n.contains("plank[c]"))
    );
}

#[test]
fn a_tongue_is_a_tenon_on_an_edge() {
    // Two boards written as parts, joined by the same kind a part's boards
    // use: the tongue fills the groove exactly, so the fit checks pass.
    let opened = open(&format!(
        "{THREE_BOARDS}joint seam : tongue_and_groove(plank[a], into: plank[b], depth: 6mm, thickness: 6mm)"
    ));
    let design = design(&opened);
    assert!(opened.diagnostics.is_empty(), "{:?}", diagnostics(&opened));
    let tongue = part(design, "plank[a]");
    assert_eq!(tongue.cut.width, mm("106"));
    let cut = &tongue.cuts[0];
    assert_eq!(cut.face.axis, PartAxis::Width);
    assert!(matches!(cut.place, CutPlace::Crossing { .. }));
    assert_eq!(
        cut.kind,
        CutKind::Tenon {
            length: mm("6"),
            width: mm("300"),
            thickness: mm("6")
        }
    );
    // Where the other part meets this one on its end, it is a type
    // mismatch.
    let opened = with_parts(
        "kind lap : joint {\n  cut tongue: crossing(piece, into) {\n    tenon(length: 5mm, width: 50mm, thickness: 5mm)\n  }\n}\njoint j : lap(shelf, into: side)",
    );
    the_diagnostic(
        &opened,
        "type-mismatch",
        "a tenon on crossing is a tongue along an edge, but side meets this part on its end",
    );
}

#[test]
fn a_joint_applies_to_the_pairs_that_meet() {
    // Two shelves into two sides and a cleat: four pairs meet.
    let opened = open(
        "part side[left, right] : pine_19 {
  length: 400mm
  width:  300mm
  place {
    orient(length: up, width: back)
    against(base.top)
    spaced(ends, from: base.left, to: base.right)
    flush(base.front)
  }
}
part shelf[low, high] : pine_19 {
  length: between(side[left], side[right])
  width:  300mm
  place {
    spaced(evenly, from: base.top, to: side[left].top)
    against(side[left].right)
    flush(base.front)
  }
}
part cleat : pine_19 {
  length: 100mm
  width:  50mm
  place {
    against(base.top)
    center(base, across)
    flush(base.front)
  }
}
joint housings : dado(shelf[*], into: [side[*], cleat])",
    );
    let design = solved(&opened);
    let housings = joint(design, "housings");
    assert_eq!(housings.pairs.len(), 4);
    assert!(
        housings
            .pairs
            .iter()
            .all(|p| matches!(p.contact, Contact::Butt { .. }))
    );
    // The cleat meets no shelf: it is recorded, and reported once layer 4
    // (fit and contact) is built.
    assert_eq!(housings.unmatched, [part(design, "cleat").id]);
}

#[test]
fn a_kind_require_is_checked_per_pair_and_reported_at_the_joint() {
    let opened = with_parts(
        "kind k : joint {\n  require thin : piece.thickness < 10mm \"piece {piece} is too thick\"\n}\njoint j : k(shelf, into: side)",
    );
    let diagnostic = the_diagnostic(&opened, "require", "'thin' fails: piece shelf is too thick");
    assert_eq!(body_line(&opened, diagnostic), 23, "the joint's line");
}

/// A pin whose diameter must be half the 19mm shelf's thickness, from two
/// lists; `call` is the joint's settings after the roles.
fn pin_rule(params: &str, condition: &str, call: &str) -> stave_app::Opened {
    with_parts(&format!(
        "kind k : joint {{\n{params}\n  require half : {condition} \"the pin is half the board\"\n}}\njoint j : k(shelf, into: side{call})"
    ))
}

const PIN_LISTS: &str =
    "  param pin   = 6mm within [6mm, 8mm, 9.5mm, 10mm]\n  param depth = 5mm within [5mm, 10mm]";

#[test]
fn a_failed_equality_rule_offers_the_values_from_the_kinds_lists_that_fit() {
    let opened = pin_rule(PIN_LISTS, "pin * 2 == piece.thickness", "");
    the_diagnostic(
        &opened,
        "require",
        "'half' fails: the pin is half the board",
    );
    assert_eq!(
        common::help(&opened),
        "use pin: 9.5mm, from the kind's list"
    );
}

#[test]
fn the_search_tries_one_param_at_a_time_with_the_others_as_given() {
    let lists = "  param pin   = 6mm within [6mm, 8mm, 9.5mm, 10mm]\n  param depth = 5mm within [5mm, 7mm, 9mm]";
    let rule = "pin + depth == piece.thickness - 4mm";
    // 8mm + 9mm is 17mm, not 15mm: pin 6mm fits with depth as given, and
    // depth 7mm fits with pin as given. Both are offered, in the order the
    // params and their lists are written.
    let opened = pin_rule(lists, rule, ", pin: 8mm, depth: 9mm");
    the_diagnostic(
        &opened,
        "require",
        "'half' fails: the pin is half the board",
    );
    assert_eq!(
        common::help(&opened),
        "use pin: 6mm or depth: 7mm, from the kind's lists"
    );
    // 9.5mm + the default 5mm is 14.5mm: pin 10mm fits; depth 5.5mm would
    // too, but it isn't in the list, so it isn't offered.
    let opened = pin_rule(lists, rule, ", pin: 9.5mm");
    assert_eq!(common::help(&opened), "use pin: 10mm, from the kind's list");
}

#[test]
fn a_failed_equality_rule_says_when_no_listed_value_fits() {
    let opened = pin_rule(PIN_LISTS, "pin * 3 == piece.thickness", "");
    the_diagnostic(
        &opened,
        "require",
        "'half' fails: the pin is half the board",
    );
    assert_eq!(
        common::help(&opened),
        "no value in the kind's lists makes this hold (pin within [6mm, 8mm, 9.5mm, 10mm], depth within [5mm, 10mm]); change the parts' sizes"
    );
}

#[test]
fn the_search_only_offers_listed_values_and_only_for_equality() {
    // A range is never searched: 9.5mm is inside it, but only lists are
    // offered.
    let opened = pin_rule(
        "  param pin = 6mm within 4mm..12mm",
        "pin * 2 == piece.thickness",
        "",
    );
    assert_eq!(
        common::help(&opened),
        "change the joint's pin, or the parts' sizes, so the rule holds"
    );
    // A rule that compares for order, or joins an equality with `||`,
    // keeps the general help, which names the params the rule reads.
    for (condition, help) in [
        (
            "pin * 2 >= piece.thickness",
            "change the joint's pin, or the parts' sizes, so the rule holds",
        ),
        (
            "pin * 2 == piece.thickness || depth == pin",
            "change the joint's pin or depth, or the parts' sizes, so the rule holds",
        ),
    ] {
        let opened = pin_rule(PIN_LISTS, condition, "");
        assert_eq!(common::help(&opened), help, "{condition}");
    }
    // A rule that reads no param keeps the general help as it is.
    let opened = pin_rule(PIN_LISTS, "piece.thickness >= 30mm", "");
    assert_eq!(
        common::help(&opened),
        "change the joint's settings or the parts' sizes so the rule holds"
    );
}

#[test]
fn a_kind_note_fills_in_its_roles() {
    let opened = with_parts("joint housing : dado(shelf, into: side)");
    assert_eq!(
        joint(design(&opened), "housing").note.as_deref(),
        Some(
            "Test the fit dry: shelf should push into side by hand and sit tight at the shoulders"
        )
    );
}

// ---------------------------------------------------------------------------
// adds, uses, cut_first, insert
// ---------------------------------------------------------------------------

#[test]
fn extensions_of_two_joints_at_the_same_end_add_up() {
    let opened = with_parts(
        "kind k : joint {\n  adds: extend(piece, by: 3mm)\n}\njoint one : k(shelf, into: side)\njoint two : k(shelf, into: side)",
    );
    let shelf = part(solved(&opened), "shelf");
    assert_eq!(shelf.cut.length, mm("206"));
    assert_eq!(shelf.drawn_offset.length, mm("6"));
}

#[test]
fn adds_takes_a_list_of_extends() {
    let opened = with_parts(
        "kind k : joint {\n  adds: [extend(piece, by: 3mm), extend(into, by: 2mm)]\n}\njoint j : k(shelf, into: side)",
    );
    let design = solved(&opened);
    assert_eq!(part(design, "shelf").cut.length, mm("203"));
    // The side butts against the shelf's end with its face: it grows in
    // its thickness, toward the shelf.
    assert_eq!(part(design, "side").cut.thickness, mm("21"));
}

#[test]
fn adds_is_extend_of_a_role_by_a_length() {
    the_diagnostic(
        &open("kind k : joint {\n  adds: 3mm\n}"),
        "type-mismatch",
        "'adds' needs extend(role, by: d)",
    );
    only_error(
        &open("kind k : joint {\n  adds: extend(wood, by: 3mm)\n}"),
        "unknown-name",
        "unknown name 'wood'",
    );
    only_error(
        &open("kind k : joint {\n  adds: extend(piece, by: 3deg)\n}"),
        "unit-mismatch",
        "'by' needs a length, found an angle",
    );
}

#[test]
fn extend_on_parts_that_cross_is_a_type_mismatch() {
    let opened = with_parts(
        "part rail : pine_19 {\n  length: 100mm\n  width:  50mm\n  place {\n    against(base.top, gap: 100mm)\n    flush(side.left)\n    flush(base.front)\n  }\n}\nkind k : joint {\n  adds: extend(piece, by: 3mm)\n}\njoint j : k(rail, into: side)",
    );
    the_diagnostic(
        &opened,
        "type-mismatch",
        "'extend' needs parts that butt, but rail and side cross",
    );
}

#[test]
fn uses_lists_fasteners_per_pair_with_a_count() {
    let opened = with_parts(
        "kind pinned : joint {\n  uses: dowel_pin(diameter: 6mm, length: 30mm, count: 3)\n}\nkind screwed : joint {\n  uses: wood_screw\n}\njoint a : pinned(shelf, into: side)\njoint b : screwed(shelf, into: side)",
    );
    let design = design(&opened);
    let uses: Vec<(&str, u32)> = design
        .fasteners
        .iter()
        .map(|f| (f.kind.as_str(), f.count))
        .collect();
    assert_eq!(uses, [("dowel_pin", 3), ("wood_screw", 1)]);
    assert_eq!(design.fasteners[0].diameter, mm("6"));
    assert!(design.hardware.is_empty(), "fasteners are never placed");
    only_error(
        &open("kind k : joint {\n  uses: knob\n}"),
        "type-mismatch",
        "'uses' needs a fastener kind, found a pull kind",
    );
}

#[test]
fn cut_first_orders_the_roles_of_each_pair() {
    let opened = with_parts(
        "kind k : joint {\n  cut_first: [into, piece]\n}\nkind dado_first : joint {\n  cut_first: into\n}\njoint a : k(shelf, into: side)\njoint b : dado_first(shelf, into: side)",
    );
    let design = design(&opened);
    let (side, shelf) = (part(design, "side").id, part(design, "shelf").id);
    assert_eq!(joint(design, "a").pairs[0].cut_order, [side, shelf]);
    assert_eq!(joint(design, "b").pairs[0].cut_order, [side, shelf]);
    the_diagnostic(
        &with_parts("kind k : joint {\n  cut_first: shelf\n}"),
        "type-mismatch",
        "'cut_first' needs a role, found 'shelf'",
    );
}

#[test]
fn insert_steps_move_a_part_along_its_own_axes() {
    let opened = with_parts(
        "kind k : joint {\n  insert: [push(piece, along: width), push(piece, along: length, by: 5mm), push(piece, along: width, angle: 45deg), rotate(piece, by: 90deg, about: length)]\n}\njoint j : k(shelf, into: side)",
    );
    let steps = &joint(design(&opened), "j").pairs[0].insert;
    let summary: Vec<(StepKind, PartAxis)> = steps.iter().map(|s| (s.kind, s.axis)).collect();
    assert_eq!(
        summary,
        [
            (StepKind::Push, PartAxis::Width),
            (StepKind::Push, PartAxis::Length),
            (StepKind::Push, PartAxis::Width),
            (StepKind::Rotate, PartAxis::Length),
        ]
    );
    assert!(steps[1].by.is_some() && steps[2].angle.is_some());
}

#[test]
fn insert_steps_name_part_axes_and_a_fixed_vocabulary() {
    let opened = open("kind k : joint {\n  insert: [push(piece, along: up)]\n}");
    only_error(&opened, "unknown-name", "unknown name 'up'");
    let opened = open("kind k : joint {\n  insert: [shove(piece, along: width)]\n}");
    only_error(&opened, "unknown-name", "unknown name 'shove'");
    assert_eq!(
        common::help(&opened),
        "an assembly step is push, rotate or drive"
    );
}

// ---------------------------------------------------------------------------
// makes part
// ---------------------------------------------------------------------------

#[test]
fn a_made_part_is_named_after_its_joint_and_made_once_per_pair() {
    let opened = open(
        "stock key_stock { species: pine, form: solid, thickness: 6mm }
part rail[low, high] : pine_19 {
  length: 600mm
  width:  50mm
  place {
    against(base.top)
    flush(base.left)
    spaced(ends, from: base.front, to: base.back)
  }
}
part post : pine_19 {
  length: 300mm
  width:  50mm
  place {
    orient(length: back, width: up)
    against(base.top)
    center(base, across)
    flush(base.front)
  }
}
kind keyed : joint {
  makes part key : key_stock {
    length: 30mm
    width:  10mm
    place {
      orient(length: thickness, width: width, of: piece)
      center(crossing(piece, into))
    }
  }
  insert: [drive(key, along: length)]
}
joint keys : keyed(rail[*], into: post)",
    );
    let design = solved(&opened);
    let made: Vec<&str> = design.made_parts().map(|p| p.name.as_str()).collect();
    assert_eq!(made.len(), 2, "{made:?}");
    assert!(
        made.iter().all(|name| name.starts_with("keys.key")),
        "{made:?}"
    );
    let key = design.made_parts().next().unwrap();
    assert!(matches!(key.origin, PartOrigin::Made { pair: 0, .. }));
    assert_eq!(key.cut.thickness, mm("6"));
}

// ---------------------------------------------------------------------------
// Cut selectors and operations
// ---------------------------------------------------------------------------

#[test]
fn a_cut_selector_is_end_side_or_crossing() {
    let opened = open("kind k : joint {\n  cut face(into) {\n    notch(depth: 3mm)\n  }\n}");
    only_error(&opened, "unknown-name", "unknown name 'face'");
    assert_eq!(
        common::help(&opened),
        "a face selector is end, side or crossing"
    );
}

#[test]
fn side_names_the_face_across_the_width_or_thickness() {
    the_diagnostic(
        &open(
            "kind k : joint {\n  cut recess: side(piece, normal: length, from: into) {\n    pocket(width: 10mm, thickness: 5mm, depth: 5mm)\n  }\n}",
        ),
        "type-mismatch",
        "'normal' needs width or thickness, found length",
    );
    the_diagnostic(
        &open(
            "kind k : joint {\n  cut recess: side(piece, from: into) {\n    pocket(width: 10mm, thickness: 5mm, depth: 5mm)\n  }\n}",
        ),
        "missing-field",
        "'side' is missing 'normal'",
    );
}

#[test]
fn each_operation_is_cut_on_its_own_selectors() {
    for (selector, operation, message, help) in [
        (
            "side(piece, normal: width, from: into)",
            "tenon(length: 3mm, width: 3mm, thickness: 3mm)",
            "'tenon' can't be cut on side",
            "'tenon' is cut on end or crossing",
        ),
        (
            "end(piece, facing: into)",
            "notch(depth: 3mm)",
            "'notch' can't be cut on end",
            "'notch' is cut on crossing",
        ),
        (
            "end(piece, facing: into)",
            "round(diameter: 5mm)",
            "'round' can't be cut on end",
            "'round' is cut on crossing",
        ),
    ] {
        let opened = open(&format!(
            "kind k : joint {{\n  cut shape: {selector} {{\n    {operation}\n  }}\n}}"
        ));
        only_error(&opened, "type-mismatch", message);
        assert_eq!(common::help(&opened), help);
    }
}

#[test]
fn a_required_operation_argument_is_a_missing_field() {
    the_diagnostic(
        &open("kind k : joint {\n  cut housing: crossing(into, piece) {\n    notch()\n  }\n}"),
        "missing-field",
        "'notch' is missing 'depth'",
    );
}

#[test]
fn an_unknown_operation_lists_the_operations() {
    let opened = open(
        "kind k : joint {\n  cut housing: crossing(into, piece) {\n    chop(depth: 3mm)\n  }\n}",
    );
    only_error(&opened, "unknown-name", "unknown name 'chop'");
    assert_eq!(
        common::help(&opened),
        "a cut operation is one of notch, slot, pocket, tenon, spigot, round, drill, dovetail_socket, dovetail_tail, outline"
    );
}

#[test]
fn through_cuts_all_the_way() {
    let opened = with_parts(
        "kind k : joint {\n  cut housing: crossing(into, piece) {\n    notch(depth: through)\n  }\n}\njoint j : k(shelf, into: side)",
    );
    // The notch goes right across the side, so the side is in two: the
    // design still solves, with that one error.
    let side = part(opened.design.as_ref().unwrap(), "side");
    assert!(side.cuts.iter().any(|c| c.kind
        == CutKind::Notch {
            depth: Depth::Through
        }));
    the_diagnostic(
        &opened,
        "wall",
        "j: its housing for shelf goes right across side and cuts it in two",
    );
}

#[test]
fn operations_record_their_measurements() {
    let opened = with_parts(
        "kind k : joint {
  cut housing: crossing(into, piece) {
    slot(width: 5mm, depth: 6mm, angle: 10deg, from_centre: true)
    drill(diameter: 5mm, depth: 8mm)
  }
  cut recess: side(piece, normal: thickness, from: into, opposite: true, flip: true) {
    pocket(width: 10mm, thickness: 5mm, depth: 5mm)
  }
  cut tenon: end(piece, facing: into) {
    tenon(length: 10mm, width: 100mm, thickness: 6mm)
  }
}
joint j : k(shelf, into: side)",
    );
    let design = design(&opened);
    let side_cuts: Vec<&CutKind> = part(design, "side").cuts.iter().map(|c| &c.kind).collect();
    assert!(side_cuts.iter().any(|k| matches!(
        k,
        CutKind::Slot {
            from_centre: true,
            ..
        }
    )));
    assert!(side_cuts.iter().any(|k| **k
        == CutKind::Drill {
            diameter: mm("5"),
            depth: Depth::Length(mm("8")),
        }));
    let shelf = part(design, "shelf");
    assert!(
        shelf
            .cuts
            .iter()
            .any(|c| matches!(c.place, CutPlace::Side { .. }))
    );
    assert!(shelf.cuts.iter().any(|c| matches!(
        (&c.place, &c.kind),
        (CutPlace::End { .. }, CutKind::Tenon { .. })
    )));
}

#[test]
fn a_dovetail_slope_is_written_1_in_n() {
    let opened = with_parts(
        "kind k : joint {
  cut tenon: end(piece, facing: into) {
    dovetail_tail(neck: 100mm, depth: 10mm, slope: 1 in 8)
  }
}
joint j : k(shelf, into: side)",
    );
    let design = design(&opened);
    let slopes: Vec<Slope> = part(design, "shelf")
        .cuts
        .iter()
        .filter_map(|c| match c.kind {
            CutKind::DovetailTail { slope, .. } => Some(slope),
            _ => None,
        })
        .collect();
    assert_eq!(
        slopes,
        [Slope::Ratio(
            stave_lang::units::Rational::new(1, 8).unwrap()
        )]
    );
    assert_eq!(slopes[0].to_string(), "slope 1 in 8");
}

#[test]
fn a_dovetail_takes_an_angle_or_a_slope() {
    let joint = |arguments: &str| {
        with_parts(&format!(
            "kind k : joint {{\n  cut tail: end(piece, facing: into) {{\n    dovetail_tail(neck: 100mm, depth: 10mm{arguments})\n  }}\n}}\njoint j : k(shelf, into: side)"
        ))
    };
    only_error(
        &joint(", angle: 10deg, slope: 1 in 8"),
        "type-mismatch",
        "a dovetail takes 'angle' or 'slope', not both",
    );
    only_error(
        &joint(""),
        "missing-field",
        "dovetail_tail is missing 'angle' or 'slope'",
    );
    only_error(
        &joint(", slope: 0 in 8"),
        "type-mismatch",
        "a dovetail's slope must be more than 0",
    );
}

/// A drill makes one hole: `count:` spread several before joint members
/// placed them, and now says what to write instead.
#[test]
fn a_drill_has_no_count_and_points_to_joint_members() {
    let opened = open(
        "kind k : joint {\n  cut hole: crossing(into, piece) {\n    drill(diameter: 5mm, depth: 5mm, count: 2)\n  }\n}",
    );
    only_error(&opened, "unknown-field", "'drill' has no field 'count'");
    assert_eq!(
        common::help(&opened),
        "a drill makes one hole; for a row of holes, give the joint kind a member for what goes in them, with count: and spaced:, and drill crossing(role, member): one hole for every copy, lined up in both parts"
    );
}

// ---------------------------------------------------------------------------
// Hardware kinds
// ---------------------------------------------------------------------------

/// What a pull kind needs: the part it is mounted on and where it sits.
const PULL_HEAD: &str = "  roles {\n    piece: Part\n  }\n  mount {\n    face:   front\n    across: open\n    along:  open\n  }\n";

/// A slide, hinge or pull kind declares the parts it joins, the first the
/// part it is mounted on, and where it sits; a fastener still has its fixed
/// entries. (Before, a base fixed the roles and required entries such as a
/// slide's travel and a pull's envelope.)
#[test]
fn a_hardware_kind_declares_its_roles() {
    let no_roles = open(
        "kind thing : slide {\n  mount {\n    face:   left\n    across: open\n    along:  open\n  }\n}",
    );
    the_diagnostic(
        &no_roles,
        "missing-field",
        "kind 'thing' is missing 'roles'",
    );
    let region_first = open(&format!(
        "kind thing : slide {{\n  roles {{\n    in:    Region\n    piece: Part\n  }}\n{}\n}}",
        "  mount {\n    face:   left\n    across: open\n    along:  open\n  }"
    ));
    let diagnostic = the_diagnostic(
        &region_first,
        "missing-field",
        "kind 'thing' is missing 'roles'",
    );
    assert_eq!(
        diagnostic.help.as_deref(),
        Some("make the first role the part it is mounted on: piece: Part")
    );
    let several = open(&format!(
        "kind thing : pull {{\n  roles {{\n    piece: Parts\n  }}\n{}\n}}",
        "  mount {\n    face:   front\n    across: open\n    along:  open\n  }"
    ));
    assert!(
        diagnostics(&several)
            .contains(&"type-mismatch: 'piece' needs Part or Region, found 'Parts'".to_owned()),
        "{:?}",
        diagnostics(&several)
    );
    the_diagnostic(
        &open("kind thing : pull {\n  roles {\n    piece: Part\n  }\n}"),
        "missing-field",
        "kind 'thing' is missing 'mount'",
    );
    the_diagnostic(
        &open("kind thing : fastener {\n  diameter: 3mm\n}"),
        "missing-field",
        "kind 'thing' is missing 'length'",
    );
    // The roles are bound by the call like a joint's.
    let opened = open(
        "part front : pine_19 {\n  length: 300mm\n  width:  150mm\n  place {\n    orient(length: right, width: up)\n    against(base.top)\n    flush(base.left)\n    flush(base.front)\n  }\n}\nhardware a : knob(front) {\n  place {\n    center(front, across)\n    center(front, up)\n  }\n}",
    );
    let design = design(&opened);
    let knob = common::hardware(design, "a");
    assert_eq!(
        knob.roles,
        [(
            stave_model::solved::RoleName::new("piece").unwrap(),
            stave_model::solved::RoleValue::Part(part(design, "front").id)
        )]
    );
    assert_eq!(
        knob.entries
            .get(&stave_model::solved::EntryName::new("diameter").unwrap()),
        Some(&stave_model::solved::EntryValue::Length(mm("30")))
    );
}

#[test]
fn a_required_entry_may_be_a_param_and_every_entry_is_readable() {
    holds(
        "kind runner : slide {\n  roles {\n    piece: Part\n    in:    Region\n  }\n  param length = 400mm\n  clearance: 12.7mm\n  travel:    length\n  mount {\n    face:   face(piece, nearer: in)\n    across: center(piece)\n    along:  from(end(piece, toward: front))\n  }\n}\nlet short = runner(length: 300mm)\nrequire r : short.travel == 300mm && short.clearance == 12.7mm \"readable\"",
    );
}

/// A pull's screw is the size its kind says. (Before, a pull's
/// `screw_diameter` was 4mm when the kind didn't say: a base default.)
#[test]
fn a_pulls_screw_diameter_is_its_kinds_own() {
    holds("require r : knob.screw_diameter == 4mm \"the standard knob's\"");
    only_error(
        &open(&format!(
            "kind p : pull {{\n{PULL_HEAD}}}\nrequire r : p.screw_diameter == 4mm \"none\""
        )),
        "unknown-field",
        "a pull kind has no field 'screw_diameter'",
    );
}

#[test]
fn hardware_kinds_have_no_joint_fields_and_cut_where_a_member_is() {
    for (entry, code, message) in [
        (
            "roles {\n    a: Part\n  }",
            "unknown-field",
            "a fastener kind has no field 'roles'",
        ),
        (
            "mount {\n    face: front\n  }",
            "unknown-field",
            "a fastener kind has no field 'mount'",
        ),
    ] {
        only_error(
            &open(&format!(
                "kind f : fastener {{\n  diameter: 3mm\n  length:   20mm\n  {entry}\n}}"
            )),
            code,
            message,
        );
    }
    for (entry, code, message) in [
        (
            "cut hole: end(piece, facing: piece) {\n    drill(diameter: 3mm, depth: 3mm)\n  }",
            "type-mismatch",
            "a hardware kind cuts on crossing, found 'end'",
        ),
        (
            "cut hole: crossing(piece, knob) {\n    drill(diameter: 3mm, depth: 3mm)\n  }",
            "unknown-name",
            "'crossing' needs a member of the kind, found 'knob'",
        ),
        (
            "adds: extend(piece, by: 3mm)",
            "unknown-field",
            "a pull kind has no field 'adds'",
        ),
    ] {
        only_error(
            &open(&format!("kind p : pull {{\n{PULL_HEAD}  {entry}\n}}")),
            code,
            message,
        );
    }
}

#[test]
fn an_envelope_is_a_box_or_a_cylinder() {
    only_error(
        &open("kind f : fastener {\n  diameter: 3mm\n  length:   20mm\n  envelope: 3mm\n}"),
        "type-mismatch",
        "'envelope' needs an envelope, found a length",
    );
}

#[test]
fn a_model_is_for_looks_only() {
    // A missing model file is drawn as its members: no diagnostic. `align:`
    // and `sha256:` are both optional (tests/models.rs checks a model that
    // is there against the text).
    let opened = open(&format!(
        "kind p : pull {{\n{PULL_HEAD}  model:    model(\"./models/knob.glb\", align: node(\"mount\"))\n  license:  \"CC0\"\n  moving:   \"knob\"\n}}\nkind q : pull {{\n{PULL_HEAD}  model:    model(\"./models/knob.glb\", sha256: \"e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855\")\n}}\nkind r : pull {{\n{PULL_HEAD}  model:    model(\"./models/knob.glb\")\n}}"
    ));
    assert!(opened.diagnostics.is_empty(), "{:?}", diagnostics(&opened));
    // A model is no longer fitted to its bounds: the text is checked
    // against its named nodes instead.
    only_error(
        &open(&format!(
            "kind q : pull {{\n{PULL_HEAD}  model:    model(\"./models/knob.glb\", align: bounds)\n}}"
        )),
        "unknown-name",
        "unknown name 'bounds'",
    );
}

#[test]
fn a_tool_kind_has_a_title_and_powered() {
    let opened = open("kind sumitsubo : tool { title: \"sumitsubo (ink line)\" }");
    assert!(opened.diagnostics.is_empty(), "{:?}", diagnostics(&opened));
    let opened = open("kind t : tool { colour: 1 }");
    only_error(
        &opened,
        "unknown-field",
        "a tool kind has no field 'colour'",
    );
    assert_eq!(common::help(&opened), "its entries are: title, powered");
}

#[test]
fn a_pull_bores_one_hole_or_two_holes_centres_apart() {
    let front = "part front : pine_19 {\n  length: 300mm\n  width:  150mm\n  place {\n    orient(length: right, width: up)\n    against(base.top)\n    flush(base.left)\n    flush(base.front)\n  }\n}";
    let place = "{\n  place {\n    center(front, across)\n    center(front, up)\n  }\n}";
    let opened = open(&format!(
        "{front}\nhardware a : knob(front) {place}\nhardware b : bar_pull(front, centres: 96mm) {place}"
    ));
    let design = solved(&opened);
    let cuts = &part(design, "front").cuts;
    // Each hole: the piece of hardware that drills it and its centre along
    // the front's length.
    let holes: Vec<(String, stave_lang::units::Length)> = cuts
        .iter()
        .filter_map(|c| match (&c.place, &c.kind) {
            (
                CutPlace::Member {
                    owner: stave_model::solved::BodyOwner::Hardware { hardware },
                    footprint,
                    ..
                },
                CutKind::Drill {
                    diameter,
                    depth: Depth::Through,
                },
            ) if *diameter == mm("4") => Some((
                design.hardware[hardware.index()].name.clone(),
                stave_lang::units::Length::from_nm(
                    (footprint.min.length.nm() + footprint.max.length.nm()) / 2,
                ),
            )),
            _ => None,
        })
        .collect();
    // One hole in the middle for the knob; two 96mm apart for the bar.
    assert_eq!(
        holes,
        [
            ("a".to_owned(), mm("150")),
            ("b".to_owned(), mm("102")),
            ("b".to_owned(), mm("198")),
        ]
    );
}

#[test]
fn a_cup_hinge_bores_its_cup_in_the_mount_face() {
    let opened = open(
        "/// A cup hinge for an inset door, whose plate has no adjustment: it\n/// is drawn where this design puts it.\nkind inset_cup_hinge : hinge {\n  roles {\n    piece: Part\n    on:    Part\n  }\n  opening_angle: 110deg\n  cup_diameter:  35mm\n  cup_depth:     13mm\n  cup_inset:     5mm\n  mount {\n    face:   back\n    across: from(edge(piece, nearer: on), inset: 22.5mm)\n    along:  open\n  }\n  member plate {\n    with:  on\n    shape: box(along: 50mm, across: -22.5mm..-11.5mm, depth: -55mm..-19mm)\n    mount {\n      across: from(face(on, toward: piece), inset: -22.5mm)\n      depth:  from(face(on, toward: piece))\n    }\n  }\n  member cup {\n    with:  piece\n    shape: cylinder(diameter: 35mm, depth: 0mm..13mm)\n  }\n  cut cup_bore: crossing(piece, cup) { drill(diameter: 35mm, depth: 13mm) }\n}\npart side : pine_19 {\n  length: 400mm\n  width:  300mm\n  place {\n    orient(length: up, width: back)\n    against(base.top)\n    flush(base.left)\n    flush(base.front)\n  }\n}\npart door : pine_19 {\n  length: 300mm\n  width:  200mm\n  place {\n    orient(length: up, width: right)\n    against(base.top)\n    against(side.right)\n    against(base.front)\n  }\n}\nhardware h : inset_cup_hinge(door, on: side) {\n  place {\n    flush(door.bottom, inset: 50mm)\n  }\n}",
    );
    let design = design(&opened);
    let door = part(design, "door");
    assert!(door.cuts.iter().any(|c| c.kind
        == CutKind::Drill {
            diameter: mm("35"),
            depth: Depth::Length(mm("13")),
        }));
    // The plate stays on the side: it never reaches into it.
    let plate = common::member_bounds(common::hardware(design, "h"), "plate");
    assert!(plate.min().across >= extent(part(design, "side"))[0].1);
}

// ---------------------------------------------------------------------------
// Recipes (names and types; choosing them and ordering work come later)
// ---------------------------------------------------------------------------

/// A recipe for a notch with the given entries.
fn recipe(entries: &str) -> stave_app::Opened {
    open(&format!("recipe r : notch {{\n{entries}\n}}"))
}

#[test]
fn a_recipe_uses_every_step_feature_and_need_of_its_operation() {
    let opened = recipe(
        "  tools: [chisel, mallet, marking_knife]\n  rank:  5\n  needs: [flat_face, square_stock, before(drill), after(tenon)]\n  mark {\n    knife(walls)\n    gauge(depth, on: open_edges)\n    pencil(floor, with: chisel, note: \"x\")\n    square_around(walls)\n    centre_mark(floor)\n    transfer(walls)\n    hatch(waste)\n    template(walls)\n    line(walls, with: marking_knife)\n  }\n  cut {\n    saw(walls, to: depth)\n    chop(waste, passes: 2, guide: walls, with: chisel)\n    pare(floor, to: depth)\n    plane(floor)\n    rout(waste)\n    bore(floor)\n    turn(floor)\n    shave(floor)\n  }\n  check: \"the shelf pushes in by hand\"",
    );
    assert!(opened.diagnostics.is_empty(), "{:?}", diagnostics(&opened));
}

#[test]
fn a_recipe_needs_tools_and_cutting_steps() {
    the_diagnostic(
        &recipe("  cut {\n    chop(waste)\n  }"),
        "missing-field",
        "recipe 'r' is missing 'tools'",
    );
    the_diagnostic(
        &recipe("  tools: [chisel]"),
        "missing-field",
        "recipe 'r' is missing 'cut'",
    );
}

#[test]
fn a_recipe_is_for_one_of_the_eleven_operations() {
    let opened = open("recipe r : chop {\n  tools: [chisel]\n  cut {\n    chop(waste)\n  }\n}");
    only_error(&opened, "unknown-name", "unknown name 'chop'");
    let opened = open(
        "recipe r : turning {\n  tools: [lathe]\n  cut {\n    turn(shape, to: largest)\n  }\n}",
    );
    assert!(opened.diagnostics.is_empty(), "{:?}", diagnostics(&opened));
}

#[test]
fn a_steps_feature_and_measurement_belong_to_the_operation() {
    let opened = recipe("  tools: [chisel]\n  cut {\n    chop(cheeks)\n  }");
    only_error(&opened, "unknown-name", "unknown name 'cheeks'");
    assert_eq!(
        common::help(&opened),
        "here 'chop' is one of: walls, floor, waste, open_edges"
    );
    // A measurement is not a feature, and a feature is not a measurement.
    only_error(
        &recipe("  tools: [chisel]\n  cut {\n    chop(depth)\n  }"),
        "unknown-name",
        "unknown name 'depth'",
    );
    only_error(
        &recipe(
            "  tools: [chisel]\n  mark {\n    gauge(walls, on: walls)\n  }\n  cut {\n    chop(waste)\n  }",
        ),
        "unknown-name",
        "unknown name 'walls'",
    );
}

#[test]
fn marking_steps_go_in_mark_and_cutting_steps_in_cut() {
    let opened = recipe("  tools: [chisel]\n  cut {\n    knife(walls)\n  }");
    only_error(&opened, "unknown-name", "unknown name 'knife'");
    assert_eq!(
        common::help(&opened),
        "a cutting step is saw, chop, pare, plane, rout, bore, turn or shave"
    );
}

#[test]
fn with_names_one_of_the_recipes_tools() {
    the_diagnostic(
        &recipe("  tools: [chisel]\n  cut {\n    chop(waste, with: mallet)\n  }"),
        "type-mismatch",
        "'with' needs one of the recipe's tools",
    );
}

#[test]
fn diameter_is_for_bore_only() {
    let opened = open(
        "recipe r : drill {\n  tools: [drill]\n  cut {\n    bore(centre, diameter: diameter)\n  }\n}",
    );
    assert!(opened.diagnostics.is_empty(), "{:?}", diagnostics(&opened));
    only_error(
        &open(
            "recipe r : drill {\n  tools: [drill]\n  cut {\n    saw(centre, diameter: diameter)\n  }\n}",
        ),
        "unknown-field",
        "'saw' has no field 'diameter'",
    );
}

#[test]
fn recipe_fields_are_typed() {
    only_error(
        &recipe("  tools: [chisel]\n  rank: 1mm\n  cut {\n    chop(waste)\n  }"),
        "unit-mismatch",
        "'rank' needs a count, found a length",
    );
    only_error(
        &recipe("  tools: [knob]\n  cut {\n    chop(waste)\n  }"),
        "type-mismatch",
        "'tools' needs a tool, found a pull kind",
    );
    only_error(
        &recipe("  tools: [chisel]\n  cut {\n    chop(waste, passes: 2mm)\n  }"),
        "unit-mismatch",
        "'passes' needs a count, found a length",
    );
    let opened = recipe("  tools: [chisel]\n  needs: [sharp]\n  cut {\n    chop(waste)\n  }");
    only_error(&opened, "unknown-name", "unknown name 'sharp'");
    assert_eq!(
        common::help(&opened),
        "a need is flat_face, square_stock, before(operation) or after(operation)"
    );
    only_error(
        &recipe("  tools: [chisel]\n  needs: [before(chop)]\n  cut {\n    chop(waste)\n  }"),
        "unknown-name",
        "unknown name 'chop'",
    );
    only_error(
        &recipe("  tools: [chisel]\n  colour: 1\n  cut {\n    chop(waste)\n  }"),
        "unknown-field",
        "recipe 'r' has no field 'colour'",
    );
}

#[test]
fn roles_params_and_made_parts_of_a_kind_share_one_namespace() {
    let opened = open(
        "kind k : joint {\n  roles {\n    piece: Part\n    into:  Part\n  }\n  param piece = 1mm\n}",
    );
    let diagnostic = the_diagnostic(&opened, "duplicate-name", "'piece' is declared twice");
    assert_eq!(
        diagnostic.help.as_deref(),
        Some("roles, params and made parts of a kind share one namespace")
    );
}

// ---------------------------------------------------------------------------
// Hardware mounts, selectors and cuts
// ---------------------------------------------------------------------------

fn plus(a: stave_lang::units::Length, b: stave_lang::units::Length) -> stave_lang::units::Length {
    a.checked_add(b).unwrap()
}

fn minus(a: stave_lang::units::Length, b: stave_lang::units::Length) -> stave_lang::units::Length {
    a.checked_sub(b).unwrap()
}

/// A front standing on the base, its length running right.
const FRONT: &str = "part front : pine_19 {\n  length: 300mm\n  width:  150mm\n  place {\n    orient(length: right, width: up)\n    against(base.top)\n    flush(base.left)\n    flush(base.front)\n  }\n}\n";

/// A knob whose kind centres it up and down and leaves the axis along the
/// front to the design.
const CENTRED_KNOB: &str = "kind centred_knob : pull {\n  roles {\n    piece: Part\n  }\n  mount {\n    face:   front\n    across: center(piece)\n    along:  open\n  }\n  member knob {\n    with:  piece\n    shape: cylinder(diameter: 20mm, depth: -20mm..0mm)\n  }\n}\n";

#[test]
fn mount_axes_left_open_are_set_by_place() {
    let knob = |place: &str| {
        open(&format!(
            "{FRONT}{CENTRED_KNOB}hardware a : centred_knob(front){place}"
        ))
    };
    let opened = knob(" {\n  place {\n    flush(front.left, inset: 30mm)\n  }\n}");
    let design = design(&opened);
    let bounds = common::hardware(design, "a").bounds;
    let [across, up, _] = extent(part(design, "front"));
    // Along the front: where the design puts it.
    assert_eq!(
        (bounds.min().across, bounds.max().across),
        (plus(across.0, mm("30")), plus(across.0, mm("50")))
    );
    // Up and down: where the kind puts it, centred on the front.
    let middle = stave_lang::units::Length::from_nm((up.0.nm() + up.1.nm()) / 2);
    assert_eq!(
        (bounds.min().up, bounds.max().up),
        (minus(middle, mm("10")), plus(middle, mm("10")))
    );
    // An axis the kind fixes can't be set again; the open one must be.
    only_error(
        &knob(" {\n  place {\n    flush(front.left, inset: 30mm)\n    center(front, up)\n  }\n}"),
        "axis-set-twice",
        "a: position along 'up' is set twice",
    );
    only_error(
        &knob(""),
        "axis-not-set",
        "a: position along 'across' is not set",
    );
}

/// A side standing on the base and a flap against its right face, the
/// flap's front edge flush with the side's front; the side's middle is
/// nearer the flap's back edge.
const FLAP: &str = "part side : pine_19 {\n  length: 300mm\n  width:  300mm\n  place {\n    orient(length: up, width: back)\n    against(base.top)\n    flush(base.left)\n    flush(base.front)\n  }\n}\npart flap : pine_19 {\n  length: 200mm\n  width:  100mm\n  place {\n    orient(length: up, width: back)\n    against(base.top)\n    against(side.right)\n    flush(side.front)\n  }\n}\n";

#[test]
fn edge_flush_with_picks_the_flush_edge() {
    let hung = |kind: &str| {
        open(&format!(
            "{FLAP}{kind}hardware h : {}(flap, on: side, face: left) {{\n  place {{\n    flush(flap.bottom, inset: 20mm)\n  }}\n}}",
            if kind.is_empty() {
                "butt_hinge"
            } else {
                "nearer_hinge"
            }
        ))
    };
    // The standard butt hinge's pin is on the flush edge: the front.
    let opened = hung("");
    let design = design(&opened);
    let leaf = common::hardware(design, "h").bounds;
    let [_, _, deep] = extent(part(design, "flap"));
    assert_eq!(
        (leaf.min().deep, leaf.max().deep),
        (deep.0, plus(deep.0, mm("12")))
    );
    // The same hinge measured from the edge nearer the side's middle sits
    // at the back.
    let nearer = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../std/hardware/hinges.stave"),
    )
    .unwrap();
    let start = nearer.find("kind butt_hinge : hinge {").unwrap();
    let kind = nearer[start..]
        .replacen("kind butt_hinge", "kind nearer_hinge", 1)
        .replacen("edge(piece, flush_with: on)", "edge(piece, nearer: on)", 1);
    let opened = hung(&format!("{kind}\n"));
    let design = common::design(&opened);
    let leaf = common::hardware(design, "h").bounds;
    assert_eq!(
        (leaf.min().deep, leaf.max().deep),
        (minus(deep.1, mm("12")), deep.1)
    );
}

/// A slide whose drawer pulls out to the right, so its drawer side's
/// thickness runs front to back.
const SIDEWAYS_SLIDE: &str = "kind sideways_slide : slide {\n  roles {\n    piece: Part\n    in:    Region\n  }\n  length:    250mm\n  clearance: 12.7mm\n  travel:    250mm\n  mount {\n    face:   face(piece, nearer: in)\n    across: center(piece)\n    along:  from(end(piece, toward: right))\n  }\n  member cabinet_member {\n    with:  in\n    shape: box(along: 0mm..250mm, across: 45mm, depth: -12.7mm..-6.35mm)\n  }\n  member drawer_member {\n    with:  piece\n    shape: box(along: 0mm..250mm, across: 45mm, depth: -6.35mm..0mm)\n  }\n}\n";

#[test]
fn face_nearer_works_on_any_axis() {
    let opened = open(&format!(
        "{SIDEWAYS_SLIDE}part back_wall : pine_19 {{\n  length: 600mm\n  width:  200mm\n  place {{\n    orient(length: right, width: up)\n    against(base.top)\n    flush(base.left)\n    flush(base.back)\n  }}\n}}\nregion bay = opening(left: base.left, right: base.right, bottom: base.top, top: back_wall.top, front: base.front, back: back_wall.front)\npart drawer_side : pine_12 {{\n  length: 300mm\n  width:  100mm\n  place {{\n    orient(length: right, width: up)\n    against(base.top)\n    against(back_wall.front, gap: 12.7mm)\n    flush(base.right)\n  }}\n}}\nhardware runner : sideways_slide(drawer_side, in: bay)"
    ));
    let design = design(&opened);
    let runner = common::hardware(design, "runner");
    let drawer_member = common::member_bounds(runner, "drawer_member");
    // The drawer side's back face is the one nearer a side of the bay.
    assert_eq!(runner.mount_face, stave_lang::placement::Face::Back);
    let [across, _, deep] = extent(part(design, "drawer_side"));
    // From the side's right end, 250mm along it, standing off its back.
    assert_eq!(
        (drawer_member.min().across, drawer_member.max().across),
        (minus(across.1, mm("250")), across.1)
    );
    assert_eq!(
        (drawer_member.min().deep, drawer_member.max().deep),
        (deep.1, plus(deep.1, mm("6.35")))
    );
    // A drawer side whose length doesn't run that way can't be measured
    // from such an end.
    let opened = open(&format!(
        "{SIDEWAYS_SLIDE}part drawer_side : pine_12 {{\n  length: 300mm\n  width:  100mm\n  place {{\n    orient(length: back, width: up)\n    against(base.top)\n    flush(base.left)\n    flush(base.front)\n  }}\n}}\nregion bay = opening(left: base.left, right: base.right, bottom: base.top, top: drawer_side.top, front: base.front, back: base.back)\nhardware runner : sideways_slide(drawer_side, in: bay)"
    ));
    let diagnostic = the_diagnostic(
        &opened,
        "type-mismatch",
        "end(drawer_side, toward: right) needs drawer_side's length to run right, but it runs along 'deep'",
    );
    assert!(diagnostic.help.is_some());
}

#[test]
fn a_named_cut_is_called_by_its_name() {
    let opened = with_parts(
        "kind trench : joint {\n  adds: extend(piece, by: 6mm)\n  cut trench: crossing(into, piece) {\n    notch(depth: 3mm)\n  }\n}\njoint housing : trench(shelf, into: side)",
    );
    let solved = common::solved(&opened);
    let cut = &part(solved, "side").cuts[0];
    assert_eq!(cut.name.as_ref().map(|n| n.as_str()), Some("trench"));
    // Messages call the cut by its name.
    let overlap = opened
        .diagnostics
        .iter()
        .find(|d| d.code == stave_lang::Code::Overlap)
        .expect("the shelf goes past the bottom of its trench");
    let help = overlap.help.as_deref().unwrap_or("");
    assert!(help.contains("its trench in side"), "{help}");
    // A standard dado's cut is called a dado; a hardware kind's cut keeps
    // its name too.
    let opened = with_parts("joint housing : dado(shelf, into: side)");
    assert_eq!(
        part(design(&opened), "side").cuts[0]
            .name
            .as_ref()
            .map(|n| n.as_str()),
        Some("dado")
    );
    let opened = open(&format!(
        "{FRONT}hardware a : knob(front) {{\n  place {{\n    center(front, across)\n    center(front, up)\n  }}\n}}"
    ));
    assert_eq!(
        part(design(&opened), "front").cuts[0]
            .name
            .as_ref()
            .map(|n| n.as_str()),
        Some("screw_hole")
    );
}

/// The full text of `warning[unnamed-cut]`: a kind used for two sides is
/// warned once, at its cut.
#[test]
fn an_unnamed_cut_is_a_warning_once_at_its_kind() {
    let root = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
    let opened = stave_app::open(&root.join("tests/broken/unnamed_cut")).unwrap();
    let rendered = stave_lang::render_diagnostics(&opened.sources, &opened.diagnostics)
        .replace(&format!("{}/", root.display()), "");
    insta::assert_snapshot!(rendered);
}

/// The full text of `error[duplicate-name]` for two cuts of one kind with
/// the same name: both places are named, and the help asks for a name per
/// cut rather than removing one.
#[test]
fn two_cuts_of_one_kind_with_one_name_are_an_error() {
    let root = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
    let opened = stave_app::open(&root.join("tests/broken/duplicate_cut_name")).unwrap();
    let rendered = stave_lang::render_diagnostics(&opened.sources, &opened.diagnostics)
        .replace(&format!("{}/", root.display()), "");
    insta::assert_snapshot!(rendered);
}

/// Two different kinds may give their cuts the same names.
#[test]
fn two_kinds_may_share_a_cut_name() {
    let opened = open(
        "kind trench : joint {\n  cut groove: crossing(into, piece) {\n    notch(depth: 3mm)\n  }\n}\nkind channel : joint {\n  cut groove: crossing(into, piece) {\n    notch(depth: 3mm)\n  }\n}",
    );
    assert!(
        diagnostics(&opened).is_empty(),
        "{:?}",
        diagnostics(&opened)
    );
}

/// Every kind is resolved where it is declared, so a kind nobody uses is
/// still warned; the quoted word is the operation's, or `cut` when a cut's
/// operations have different words. A hardware kind's cut warns the same.
#[test]
fn a_kind_nobody_uses_is_still_warned() {
    let opened = open(
        "kind trench : joint {\n  cut crossing(into, piece) {\n    notch(depth: 3mm)\n  }\n  cut end(piece, facing: into) {\n    tenon(length: 3mm, width: 10mm, thickness: 3mm)\n  }\n  cut crossing(piece, into) {\n    drill(diameter: 3mm, depth: 3mm)\n    slot(width: 3mm, depth: 3mm)\n  }\n}",
    );
    assert_eq!(
        diagnostics(&opened),
        [
            "unnamed-cut: this cut has no name, so messages will call it \"a notch\"",
            "unnamed-cut: this cut has no name, so messages will call it \"a cut\"",
            "unnamed-cut: this cut has no name, so messages will call it \"a cut\"",
        ]
    );
    assert!(
        opened
            .diagnostics
            .iter()
            .all(|d| d.severity == stave_lang::Severity::Warning)
    );
    assert_eq!(
        opened.diagnostics[1].help.as_deref(),
        Some("name it after what it is, as in cut trench: end(...) { ... }")
    );
    let hardware = open(&format!(
        "kind screwed_knob : pull {{\n{PULL_HEAD}  member knob {{\n    with:  piece\n    shape: cylinder(diameter: 20mm, depth: -20mm..0mm)\n  }}\n  cut crossing(piece, knob) {{ drill(diameter: 4mm, depth: through) }}\n}}"
    ));
    assert_eq!(
        diagnostics(&hardware),
        ["unnamed-cut: this cut has no name, so messages will call it \"a hole\""]
    );
    // Named, the same kinds are quiet.
    let named = open(&format!(
        "kind screwed_knob : pull {{\n{PULL_HEAD}  member knob {{\n    with:  piece\n    shape: cylinder(diameter: 20mm, depth: -20mm..0mm)\n  }}\n  cut screw_hole: crossing(piece, knob) {{ drill(diameter: 4mm, depth: through) }}\n}}"
    ));
    assert_eq!(diagnostics(&named), Vec::<String>::new());
}

/// A recipe's `cut { }` block lists cutting steps, not a kind's cuts: it
/// never warns.
#[test]
fn a_recipes_cut_block_never_warns() {
    let opened = open(
        "recipe notch_by_saw : notch {\n  tools: [tenon_saw, chisel]\n  cut {\n    saw(walls, to: depth)\n    chop(waste, with: chisel)\n  }\n}",
    );
    assert_eq!(diagnostics(&opened), Vec::<String>::new());
}

/// A named tenon is called by its name in a message about its fit, and
/// so is a named dovetail tail and socket (`broken/dovetail_too_wide`).
#[test]
fn a_named_tenon_or_dovetail_uses_its_name_in_a_message() {
    let root = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
    let read = |folder: &str| {
        std::fs::read_to_string(root.join("tests/broken").join(folder).join("project.stave"))
            .unwrap()
    };
    let help_of = |text: &str| {
        let opened = common::open_files(&[("project.stave", text)]);
        let overlap = opened
            .diagnostics
            .iter()
            .find(|d| d.code == stave_lang::Code::Overlap)
            .expect("an overlap");
        overlap.help.clone().unwrap_or_default()
    };
    let tenon = read("tenon_too_long");
    assert!(tenon.contains("cut tenon: end(piece, facing: into)"));
    let stub = tenon.replace("cut tenon: end(", "cut stub_tenon: end(");
    let help = help_of(&stub);
    assert!(
        help.starts_with("the stub tenon on rail goes ")
            && help.contains("its mortise in leg")
            && help.contains("or the stub tenon "),
        "{help}"
    );
    let dovetail = read("dovetail_too_wide")
        .replace("cut socket: end(", "cut housing: end(")
        .replace("cut tail: end(", "cut key: end(");
    let help = help_of(&dovetail);
    assert!(
        help.starts_with("the sloped faces of the key on beam_b ")
            && help.contains("its housing in beam_a; cut the key and the housing "),
        "{help}"
    );
}

#[test]
fn a_hardware_kind_counts_the_fasteners_it_uses() {
    let opened = open(&format!(
        "{FRONT}kind screwed_knob : pull {{\n{PULL_HEAD}  uses: wood_screw(length: 25mm, count: 2)\n  member knob {{\n    with:  piece\n    shape: cylinder(diameter: 20mm, depth: -20mm..0mm)\n  }}\n}}\nhardware a : screwed_knob(front) {{\n  place {{\n    center(front, across)\n    center(front, up)\n  }}\n}}"
    ));
    let fasteners = &common::hardware(design(&opened), "a").fasteners;
    assert_eq!(
        fasteners,
        &[stave_model::solved::HardwareFastener {
            kind: "wood_screw".to_owned(),
            diameter: mm("4"),
            length: mm("25"),
            count: 2,
        }]
    );
}
