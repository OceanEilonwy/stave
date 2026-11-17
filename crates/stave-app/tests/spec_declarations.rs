//! Executable spec: "Declarations and their fields". One test per field
//! rule: what a field accepts, its default, what is readable, and the
//! error when it is missing, misspelt, written twice or of the wrong type.

mod common;

use common::{
    body_line, design, diagnostics, holds, mm, only_error, open, open_files, part, the_diagnostic,
};
use stave_model::solved::{PoseValue, SceneItem, Shape};
use stave_schema::words::{Grain, OutputUnit};

/// A whole project file with its own `project` block.
fn with_project(project_fields: &str, rest: &str) -> String {
    format!(
        "stave 1\n\nproject \"Test\" {{\n{project_fields}\n}}\n\nstock pine_19 {{ species: pine, form: solid, thickness: 19mm }}\n\npart base : pine_19 {{\n  length: 600mm\n  width:  300mm\n  place {{\n    ground\n  }}\n}}\n{rest}"
    )
}

fn open_project(project_fields: &str) -> stave_app::Opened {
    open_files(&[("project.stave", &with_project(project_fields, ""))])
}

/// A part on the base with extra fields.
fn board(fields: &str) -> String {
    format!(
        "part p : pine_19 {{\n  length: 100mm\n  width:  100mm\n{fields}\n  place {{\n    against(base.top)\n    flush(base.left)\n    flush(base.front)\n  }}\n}}"
    )
}

// ---------------------------------------------------------------------------
// project
// ---------------------------------------------------------------------------

#[test]
fn project_fields_have_their_defaults() {
    let opened = open_project("  goal: \"a box\"");
    let info = &design(&opened).project;
    assert_eq!(info.title, "Test");
    assert_eq!(info.goal.as_deref(), Some("a box"));
    assert_eq!(info.units, OutputUnit::Millimetre);
    assert_eq!(info.resolution, mm("0.5"));
    assert_eq!(info.tools, None, "left out: every tool is assumed");
    assert!(info.methods.is_empty());
}

#[test]
fn project_fields_are_read() {
    let opened = open_project(
        "  units:      in\n  resolution: 1/32in\n  style:      \"shaker\"\n  tools:      [chisel, mallet]\n  methods:    [notch_by_hand]",
    );
    let info = &design(&opened).project;
    assert_eq!(info.units, OutputUnit::Inch);
    assert_eq!(info.resolution, mm("0.79375"));
    assert_eq!(info.style.as_deref(), Some("shaker"));
    let tools: Vec<&str> = info
        .tools
        .as_ref()
        .unwrap()
        .iter()
        .map(|t| t.name.as_str())
        .collect();
    assert_eq!(tools, ["chisel", "mallet"]);
    assert_eq!(info.methods, ["notch_by_hand"]);
}

#[test]
fn output_units_are_lengths_only() {
    let opened = open_project("  units: deg");
    only_error(&opened, "unknown-name", "unknown name 'deg'");
    assert_eq!(common::help(&opened), "write one of: mm, cm, m, in, ft");
}

#[test]
fn project_resolution_is_a_length() {
    only_error(
        &open_project("  resolution: 2"),
        "unit-mismatch",
        "'resolution' needs a length, found a count",
    );
}

#[test]
fn a_misspelt_tool_is_an_unknown_name() {
    let opened = open_project("  tools: [chisle]");
    only_error(&opened, "unknown-name", "unknown name 'chisle'");
    assert_eq!(common::help(&opened), "did you mean 'chisel'?");
}

#[test]
fn project_methods_are_recipes() {
    only_error(
        &open_project("  methods: [chisel]"),
        "type-mismatch",
        "'methods' needs a recipe, found a tool",
    );
}

#[test]
fn an_unknown_project_field_lists_the_fields() {
    let opened = open_project("  colour: 1");
    only_error(
        &opened,
        "unknown-field",
        "the project has no field 'colour'",
    );
    assert_eq!(
        common::help(&opened),
        "the fields are: goal, units, resolution, style, tools, methods"
    );
}

#[test]
fn a_field_written_twice_is_a_duplicate_name() {
    let opened = open_project("  units: mm\n  units: in");
    let diagnostic = the_diagnostic(&opened, "duplicate-name", "'units' is declared twice");
    assert_eq!(common::line(&opened, diagnostic), 5);
}

// ---------------------------------------------------------------------------
// library
// ---------------------------------------------------------------------------

#[test]
fn library_fields() {
    let library = |fields: &str| {
        open_files(&[(
            "library.stave",
            &format!("stave 1\n\nlibrary \"Shop\" {{\n{fields}\n}}\n"),
        )])
    };
    let opened = library("  about:   \"jigs\"\n  license: \"CC0\"");
    assert!(opened.diagnostics.is_empty(), "{:?}", diagnostics(&opened));
    assert!(opened.design.is_none(), "a library has nothing to place");
    only_error(
        &library("  goal: \"x\""),
        "unknown-field",
        "the library has no field 'goal'",
    );
}

#[test]
fn a_library_body_is_optional() {
    let opened = open_files(&[("library.stave", "stave 1\n\nlibrary \"Shop\"\n")]);
    assert!(opened.diagnostics.is_empty(), "{:?}", diagnostics(&opened));
}

// ---------------------------------------------------------------------------
// stock
// ---------------------------------------------------------------------------

#[test]
fn stock_needs_species_form_and_thickness() {
    the_diagnostic(
        &open("stock s { form: solid, thickness: 19mm }"),
        "missing-field",
        "stock 's' is missing 'species'",
    );
    the_diagnostic(
        &open("stock s { species: pine, thickness: 19mm }"),
        "missing-field",
        "stock 's' is missing 'form'",
    );
    the_diagnostic(
        &open("stock s { species: pine, form: solid }"),
        "missing-field",
        "stock 's' is missing 'thickness'",
    );
}

#[test]
fn dowel_stock_is_sized_by_diameter() {
    holds(
        "stock d { species: beech, form: dowel, diameter: 10mm }\nrequire r : d.diameter == 10mm \"readable\"",
    );
    the_diagnostic(
        &open("stock d { species: beech, form: dowel }"),
        "missing-field",
        "stock 'd' is missing 'diameter'",
    );
    the_diagnostic(
        &open("stock d { species: beech, form: solid, thickness: 19mm, diameter: 10mm }"),
        "unknown-field",
        "stock 'd' has no field 'diameter'",
    );
}

#[test]
fn species_and_forms_are_closed_word_lists() {
    let opened = open("stock s { species: piine, form: solid, thickness: 19mm }");
    only_error(&opened, "unknown-name", "unknown name 'piine'");
    assert_eq!(common::help(&opened), "did you mean 'pine'?");
    let opened = open("stock s { species: pine, form: hardboard, thickness: 19mm }");
    only_error(&opened, "unknown-name", "unknown name 'hardboard'");
    assert_eq!(
        common::help(&opened),
        "write one of: solid, plywood, mdf, dowel"
    );
}

#[test]
fn every_stock_field_is_readable() {
    holds(
        "stock s { species: walnut, form: plywood, thickness: 18mm, width: 90mm, max_width: 200mm, max_length: 2400mm }\nrequire r : s.thickness == 18mm && s.width == 90mm && s.max_width == 200mm && s.max_length == 2400mm \"readable\"",
    );
}

#[test]
fn a_part_cut_from_stock_with_a_width_takes_that_width() {
    holds(
        "stock strip { species: pine, form: solid, thickness: 19mm, width: 40mm }\npart p : strip {\n  length: 100mm\n  place {\n    against(base.top)\n    flush(base.left)\n    flush(base.front)\n  }\n}\nrequire r : p.width == 40mm \"the stock's width\"",
    );
}

#[test]
fn a_part_longer_than_its_stocks_max_length_is_a_type_mismatch() {
    let opened = open(
        "stock short { species: pine, form: solid, thickness: 19mm, max_length: 500mm }\npart p : short {\n  length: 600mm\n  width:  100mm\n  place {\n    against(base.top)\n    flush(base.left)\n    flush(base.front)\n  }\n}",
    );
    let diagnostic = the_diagnostic(
        &opened,
        "type-mismatch",
        "p: its cut length works out as 600mm, which is more than its stock's max_length of 500mm",
    );
    assert_eq!(body_line(&opened, diagnostic), 3);
    // The cut length counts: 490mm drawn, plus 5mm at each end from a joint.
    let joined = open(
        "stock short { species: pine, form: solid, thickness: 19mm, max_length: 499mm }
part side[left, right] : pine_19 {
  length: 200mm
  width:  300mm
  place {
    orient(length: up, width: back)
    against(base.top)
    spaced(ends, from: base.left, to: base.right)
    flush(base.front)
  }
}
part shelf : short {
  length: between(side[left], side[right])
  width:  300mm
  place {
    against(base.top, gap: 100mm)
    against(side[left].right)
    flush(base.front)
  }
}
joint housings : dado(shelf, into: [side[left], side[right]], depth: 5mm)",
    );
    the_diagnostic(
        &joined,
        "type-mismatch",
        "shelf: its cut length works out as 572mm, which is more than its stock's max_length of 499mm",
    );
}

#[test]
fn a_wider_part_than_max_width_is_planned_as_a_glue_up() {
    let opened = open(
        "stock narrow { species: pine, form: solid, thickness: 19mm, max_width: 50mm }\npart p : narrow {\n  length: 100mm\n  width:  300mm\n  place {\n    against(base.top)\n    flush(base.left)\n    flush(base.front)\n  }\n}",
    );
    assert!(opened.diagnostics.is_empty(), "{:?}", diagnostics(&opened));
}

// ---------------------------------------------------------------------------
// part
// ---------------------------------------------------------------------------

#[test]
fn a_part_needs_length_width_and_place() {
    the_diagnostic(
        &open(
            "part p : pine_19 {\n  width: 100mm\n  place {\n    against(base.top)\n    flush(base.left)\n    flush(base.front)\n  }\n}",
        ),
        "missing-field",
        "part 'p' is missing 'length'",
    );
    the_diagnostic(
        &open(
            "part p : pine_19 {\n  length: 100mm\n  place {\n    against(base.top)\n    flush(base.left)\n    flush(base.front)\n  }\n}",
        ),
        "missing-field",
        "part 'p' is missing 'width'",
    );
    the_diagnostic(
        &open("part p : pine_19 {\n  length: 100mm\n  width:  100mm\n}"),
        "missing-field",
        "part 'p' is missing 'place'",
    );
}

#[test]
fn a_part_is_cut_from_a_stock() {
    only_error(
        &open(&board("").replace("part p : pine_19", "part p : base")),
        "type-mismatch",
        "'stock' needs a stock, found a part",
    );
}

#[test]
fn a_part_may_be_thinner_than_its_stock_but_not_thicker() {
    holds(&format!(
        "{}\nrequire r : p.thickness == 15mm \"planed\"",
        board("  thickness: 15mm")
    ));
    let opened = open(&board("  thickness: 25mm"));
    let diagnostic = the_diagnostic(
        &opened,
        "type-mismatch",
        "p: its thickness works out as 25mm, which is more than its stock's 19mm",
    );
    assert_eq!(body_line(&opened, diagnostic), 4);
}

#[test]
fn a_size_must_be_more_than_zero() {
    the_diagnostic(
        &open(&board("").replace("length: 100mm", "length: 100mm - 100mm")),
        "type-mismatch",
        "p: its length works out as 0mm, which is not more than zero",
    );
}

#[test]
fn grain_runs_along_the_length_by_default() {
    let opened = open(&format!(
        "{}\n{}",
        board(""),
        board("  grain: width")
            .replace("part p", "part q")
            .replace("against(base.top)", "against(base.top, gap: 50mm)")
    ));
    let design = design(&opened);
    assert_eq!(part(design, "p").grain, Grain::Length);
    assert_eq!(part(design, "q").grain, Grain::Width);
    let opened = open(&board("  grain: thickness"));
    only_error(&opened, "unknown-name", "unknown name 'thickness'");
    assert_eq!(common::help(&opened), "write one of: length, width");
}

#[test]
fn grain_shape_and_reference_faces_are_not_readable() {
    let opened = open("let g = base.grain");
    only_error(&opened, "unknown-field", "a part has no field 'grain'");
}

#[test]
fn every_part_reads_its_sizes_stock_and_faces() {
    holds(
        "require r : base.length == 600mm && base.width == 300mm && base.thickness == 19mm && base.stock.thickness == 19mm && between(base.bottom, base.top) == 19mm \"readable\"",
    );
}

#[test]
fn only_round_parts_have_a_diameter() {
    holds(
        "stock d { species: beech, form: dowel, diameter: 10mm }\npart rod : d {\n  length: 100mm\n  place {\n    against(base.top)\n    flush(base.left)\n    flush(base.front)\n  }\n}\nrequire r : rod.diameter == 10mm && rod.width == 10mm \"round\"",
    );
    let opened = open("let x = base.diameter");
    only_error(&opened, "unknown-field", "a part has no field 'diameter'");
    assert_eq!(
        common::help(&opened),
        "base is not round; only dowels and turned parts have a diameter"
    );
}

#[test]
fn face_side_and_face_edge_are_face_words() {
    let opened = open(&board(
        "  face_side: bottom\n  face_edge: back\n  note:      \"mark from below\"",
    ));
    let p = part(design(&opened), "p");
    assert_eq!(p.note.as_deref(), Some("mark from below"));
    let opened = open(&board("  face_side: up"));
    only_error(&opened, "unknown-name", "unknown name 'up'");
    assert_eq!(
        common::help(&opened),
        "write one of: top, bottom, left, right, front, back"
    );
}

#[test]
fn the_reference_face_must_lie_across_the_parts_thickness() {
    // The board lies flat: its thickness runs up, so `left` is an edge or
    // an end, not a face across the thickness.
    let opened = open(&board("  face_side: left"));
    assert_eq!(
        diagnostics(&opened),
        ["type-mismatch: 'face_side' needs a face across the part's thickness, found 'left'"]
    );
}

#[test]
fn only_length_and_width_may_be_fit() {
    only_error(
        &open(&board("  thickness: fit")),
        "type-mismatch",
        "'thickness' needs a length, found 'fit'",
    );
}

// ---------------------------------------------------------------------------
// profile and turned parts
// ---------------------------------------------------------------------------

const LEG_PROFILE: &str =
    "profile leg {\n  measure: diameter\n  at 0mm:   40mm\n  at 300mm: 30mm\n}";

fn turned_leg(length: &str) -> String {
    format!(
        "{LEG_PROFILE}\npart leg_part : pine_19 {{\n  length: {length}\n  width:  40mm\n  shape:  turned(leg)\n  place {{\n    orient(length: up)\n    against(base.top)\n    flush(base.left)\n    flush(base.front)\n  }}\n}}"
    )
}

#[test]
fn a_turned_part_is_as_long_as_its_profile() {
    holds(&format!(
        "{}\nrequire r : leg_part.length == 300mm && leg.length == 300mm \"the last station\"",
        turned_leg("fit")
    ));
    let opened = open(&turned_leg("fit"));
    assert!(matches!(
        &part(design(&opened), "leg_part").shape,
        Shape::Turned { profile, stations, .. } if profile == "leg" && stations.len() == 2
    ));
}

#[test]
fn a_turned_part_with_another_length_is_a_type_mismatch() {
    let opened = open(&turned_leg("200mm"));
    let diagnostic = the_diagnostic(
        &opened,
        "type-mismatch",
        "leg_part: its length is 200mm, but its profile 'leg' is 300mm long",
    );
    assert_eq!(
        diagnostic.help.as_deref(),
        Some("a turned part is as long as its profile: write length: fit")
    );
}

#[test]
fn a_turned_parts_box_is_as_wide_as_its_largest_diameter() {
    let opened = open(&turned_leg("fit"));
    let leg = part(design(&opened), "leg_part");
    assert_eq!((leg.drawn.width, leg.drawn.thickness), (mm("40"), mm("40")));
}

#[test]
fn a_profile_needs_a_measure_and_two_stations() {
    the_diagnostic(
        &open("profile leg {\n  at 0mm:   40mm\n  at 300mm: 30mm\n}"),
        "missing-field",
        "profile 'leg' is missing 'measure'",
    );
    the_diagnostic(
        &open("profile leg {\n  measure: radius\n  at 0mm: 40mm\n}"),
        "missing-field",
        "profile 'leg' is missing 'at'",
    );
}

#[test]
fn radius_stations_are_doubled_to_diameters() {
    holds(
        "profile peg {\n  measure: radius\n  at 0mm:  10mm\n  at 50mm: 10mm\n}\npart p : pine_19 {\n  length: fit\n  width:  20mm\n  shape:  turned(peg)\n  place {\n    orient(length: up)\n    against(base.top)\n    flush(base.left)\n    flush(base.front)\n  }\n}\nrequire r : p.width == 20mm && p.length == 50mm \"radius 10 is 20 across\"",
    );
}

#[test]
fn stations_must_not_decrease_even_in_an_unused_profile() {
    let opened = open("profile leg {\n  measure: radius\n  at 10mm: 40mm\n  at 5mm:  30mm\n}");
    let diagnostic = the_diagnostic(
        &opened,
        "type-mismatch",
        "profile 'leg' has stations that go backwards",
    );
    assert_eq!(body_line(&opened, diagnostic), 4);
}

#[test]
fn two_stations_at_one_distance_make_a_shoulder() {
    let opened = open(
        "profile stepped {\n  measure: radius\n  smooth:  true\n  at 0mm:   20mm\n  at 100mm: 20mm\n  at 100mm: 15mm\n  at 200mm: 15mm\n}",
    );
    assert!(opened.diagnostics.is_empty(), "{:?}", diagnostics(&opened));
}

#[test]
fn profile_entries_are_typed() {
    only_error(
        &open("profile leg {\n  measure: radius\n  smooth:  1\n  at 0mm:  2mm\n  at 10mm: 3mm\n}"),
        "type-mismatch",
        "'smooth' needs true or false, found a count",
    );
    only_error(
        &open("profile leg {\n  measure: radius\n  colour:  1\n  at 0mm:  2mm\n  at 10mm: 3mm\n}"),
        "unknown-field",
        "profile 'leg' has no field 'colour'",
    );
}

// ---------------------------------------------------------------------------
// scene
// ---------------------------------------------------------------------------

#[test]
fn every_project_has_the_rest_scene_first() {
    let opened = open("scene a \"A\" { }\nscene b { }");
    let scenes: Vec<(&str, Option<&str>)> = design(&opened)
        .scenes
        .iter()
        .map(|s| (s.name.as_str(), s.title.as_deref()))
        .collect();
    assert_eq!(
        scenes,
        [("rest", Some("As built")), ("a", Some("A")), ("b", None)]
    );
}

#[test]
fn declaring_a_scene_called_rest_is_a_duplicate_name() {
    the_diagnostic(
        &open("scene rest \"Rest\" { }"),
        "duplicate-name",
        "'rest' is declared twice",
    );
}

#[test]
fn scene_fields_show_hide_explode_and_section() {
    let opened = open(
        "part slat[2] : pine_19 {\n  length: 50mm\n  width:  20mm\n  place {\n    against(base.top)\n    spaced(evenly, from: base.left, to: base.right)\n    flush(base.front)\n  }\n}\nscene a {\n  show:    [slat[*], base]\n  explode: true\n  section: mid(base, across)\n}\nscene b {\n  hide: [[slat[0]], [slat[1]]]\n}",
    );
    let design = design(&opened);
    let a = &design.scenes[1];
    let base = part(design, "base").id;
    let slat0 = part(design, "slat[0]").id;
    let slat1 = part(design, "slat[1]").id;
    assert_eq!(
        a.show,
        Some(vec![
            SceneItem::Part(slat0),
            SceneItem::Part(slat1),
            SceneItem::Part(base)
        ])
    );
    assert!(a.explode);
    let section = a.section.unwrap();
    assert_eq!(section.position, mm("300"));
    // Nested lists flatten.
    assert_eq!(
        design.scenes[2].hide,
        vec![SceneItem::Part(slat0), SceneItem::Part(slat1)]
    );
}

#[test]
fn scene_fields_are_typed() {
    only_error(
        &open("scene a { explode: 1 }"),
        "type-mismatch",
        "'explode' needs true or false, found a count",
    );
    only_error(
        &open("scene a { section: base }"),
        "type-mismatch",
        "'section' needs a face or mid-plane, found a part",
    );
    only_error(
        &open("scene a { colour: 1 }"),
        "unknown-field",
        "scene 'a' has no field 'colour'",
    );
}

#[test]
fn only_an_assembly_can_be_posed() {
    the_diagnostic(
        &open("scene a {\n  pose base = open\n}"),
        "type-mismatch",
        "'pose' needs an assembly, found a part",
    );
}

#[test]
fn poses_are_open_closed_an_angle_or_a_length() {
    let opened = open(
        "part wall : pine_19 {
  length: 200mm
  width:  300mm
  place {
    orient(length: up, width: back)
    against(base.top)
    flush(base.left)
    flush(base.front)
  }
}
/// A cup hinge for an inset door, with no catalogue overlay (so its
/// overlay isn't checked).
kind inset_cup_hinge : hinge {
  opening_angle: 110deg
  cup_diameter:  35mm
  cup_depth:     13mm
  cup_inset:     5mm
  plate_setback: 37mm
  plate:         box(length: 50mm, width: 36mm, thickness: 11mm)
}
component Flap {
  part leaf : pine_19 {
    length: 200mm
    width:  100mm
    place {
      orient(length: up, width: right)
      against(base.top)
      against(wall.right)
      against(base.front)
    }
  }
  hardware hinge : inset_cup_hinge(leaf, on: wall) {
    place {
      flush(leaf.bottom, inset: 20mm)
    }
  }
}
assembly flap : Flap { }
scene opened { pose flap = open }
scene shut { pose flap = closed }
scene ajar { pose flap = 30deg }",
    );
    let design = design(&opened);
    let poses: Vec<PoseValue> = design.scenes[1..]
        .iter()
        .map(|s| s.poses[0].value)
        .collect();
    assert_eq!(poses[0], PoseValue::Open);
    assert_eq!(poses[1], PoseValue::Closed);
    assert!(matches!(poses[2], PoseValue::Angle(_)));
}

// ---------------------------------------------------------------------------
// require
// ---------------------------------------------------------------------------

#[test]
fn a_require_condition_is_a_bool() {
    only_error(
        &open("require r : 1mm \"m\""),
        "type-mismatch",
        "'r' needs true or false, found a length",
    );
}

#[test]
fn a_failing_require_is_reported_on_its_line_with_its_message() {
    let opened = open("let a = 1mm\nrequire r : a > 2mm \"one is not more than two\"");
    let diagnostic = the_diagnostic(&opened, "require", "'r' fails: one is not more than two");
    assert_eq!(body_line(&opened, diagnostic), 2);
}

#[test]
fn a_require_needs_its_message() {
    only_error(
        &open("require r : 1mm < 2mm\nlet after = 1"),
        "syntax",
        "expected a message in quotes, found end of line",
    );
}

// ---------------------------------------------------------------------------
// assembly
// ---------------------------------------------------------------------------

#[test]
fn an_assembly_reads_its_components_params_and_items() {
    holds(
        "component Box {\n  param w = 50mm\n  part p : pine_19 {\n    length: w\n    width:  100mm\n    place {\n      against(base.top)\n      flush(base.left)\n      flush(base.front)\n    }\n  }\n}\nassembly b : Box { }\nrequire r : b.w == 50mm && b.p.width == 100mm \"readable\"",
    );
}

#[test]
fn an_assembly_is_an_instance_of_a_component() {
    only_error(
        &open("assembly b : base { }"),
        "type-mismatch",
        "'b' needs a component, found a part",
    );
}

#[test]
fn an_array_of_assemblies_takes_its_arguments_per_element() {
    holds(
        "component Box {\n  param w : Length\n}\nassembly box[a, b] : Box {\n  w: 1mm\n}\nrequire r : box[a].w == 1mm && box[b].w == 1mm \"each element\"",
    );
}

// ---------------------------------------------------------------------------
// Hardware members
// ---------------------------------------------------------------------------

/// A slide kind with extra entries.
fn slide_kind(entries: &str) -> String {
    format!(
        "kind small_slide : slide {{\n  length:    300mm\n  clearance: 12.7mm\n  travel:    300mm\n  envelope:  box(length: 300mm, width: 45mm, thickness: 12.7mm)\n{entries}\n}}"
    )
}

#[test]
fn hardware_members_are_named_and_documented() {
    let opened = open(&slide_kind(
        "  /// Screwed to the cabinet.\n  /// It stays put.\n  member cabinet_member\n  member drawer_member",
    ));
    assert!(
        diagnostics(&opened).is_empty(),
        "{:?}",
        diagnostics(&opened)
    );
    let model = opened.model.as_ref().unwrap();
    let kind = model
        .defs
        .kinds
        .iter()
        .find(|k| k.decl.name == "small_slide")
        .unwrap();
    let members: Vec<(&str, Vec<String>)> = kind
        .members
        .iter()
        .map(|m| (m.name.word(), m.docs.clone()))
        .collect();
    assert_eq!(
        members,
        [
            (
                "cabinet_member",
                vec![
                    "Screwed to the cabinet.".to_owned(),
                    "It stays put.".to_owned()
                ]
            ),
            ("drawer_member", Vec::new()),
        ]
    );
}

#[test]
fn the_standard_hinges_and_slides_document_their_members() {
    let opened = open("");
    let model = opened.model.as_ref().unwrap();
    let members = |name: &str| -> Vec<&'static str> {
        let kind = model
            .defs
            .kinds
            .iter()
            .find(|k| k.decl.name == name)
            .unwrap();
        assert!(kind.members.iter().all(|m| !m.docs.is_empty()), "{name}");
        kind.members.iter().map(|m| m.name.word()).collect()
    };
    assert_eq!(members("euro_hinge"), ["plate", "arm", "links", "cup"]);
    assert_eq!(members("butt_hinge"), ["leaves", "pin"]);
    assert_eq!(
        members("side_mount_slide"),
        ["cabinet_member", "drawer_member"]
    );
}

#[test]
fn a_misnamed_member_is_an_unknown_field() {
    let opened = open(&slide_kind("  member drawer_membr"));
    let diagnostic = the_diagnostic(
        &opened,
        "unknown-field",
        "a slide kind has no member 'drawer_membr'",
    );
    assert_eq!(
        diagnostic.help.as_deref(),
        Some("did you mean 'drawer_member'?")
    );
    // A cup hinge's member on a slide is not near any slide member.
    let opened = open(&slide_kind("  member cup"));
    let diagnostic = the_diagnostic(&opened, "unknown-field", "a slide kind has no member 'cup'");
    assert_eq!(
        diagnostic.help.as_deref(),
        Some("a slide kind's members are cabinet_member and drawer_member")
    );
}

#[test]
fn a_fastener_has_no_members_and_a_member_is_written_once() {
    let opened = open("kind peg : fastener {\n  diameter: 6mm\n  length:   30mm\n  member head\n}");
    let diagnostic = the_diagnostic(
        &opened,
        "unknown-field",
        "a fastener kind has no member 'head'",
    );
    assert_eq!(
        diagnostic.help.as_deref(),
        Some("only hinge, slide and pull kinds name their members")
    );
    let opened = open(&slide_kind(
        "  member drawer_member\n  member drawer_member",
    ));
    the_diagnostic(
        &opened,
        "duplicate-name",
        "'drawer_member' is declared twice",
    );
    // A pull names its knob and screw, or its bar and screws.
    let opened = open(
        "kind small_knob : pull {\n  envelope: cylinder(diameter: 20mm, length: 20mm)\n  /// Hold it.\n  member knob\n  member screw\n}",
    );
    assert!(
        diagnostics(&opened).is_empty(),
        "{:?}",
        diagnostics(&opened)
    );
}

/// A cup hinge kind written out with its members and `mechanism` (the
/// standard hinge's arm, cup, links and pins), and a door hung on it.
fn hung_door(mechanism: &str) -> String {
    format!(
        "kind test_hinge : hinge {{
  opening_angle: 110deg
  cup_diameter:  35mm
  cup_depth:     12.8mm
  cup_inset:     5mm
  plate_setback: 37mm
  plate:         box(length: 50mm, width: 36mm, thickness: 11mm)
  member plate
  member arm {{
    with:          on
    shape body:    box(along: 16mm, across: -2.9mm..15mm, depth: -55mm..0mm)
    shape knuckle: box(along: 16mm, across: 8mm..15mm, depth: 0mm..12.4mm)
  }}
  member cup {{ with: piece, shape: cylinder(diameter: 35mm, depth: 0mm..12.8mm) }}
  member inner_link {{
    shape: bar(from: point(across: 13.5mm, depth: 10.9mm), to: point(across: -8.8mm, depth: 7.33mm), thickness: 1.5mm, along: 9mm)
  }}
  member outer_link {{
    shape: bar(from: point(across: 13.28mm, depth: 3.56mm), to: point(across: -9.54mm, depth: 1.39mm), thickness: 1.5mm, along: 14mm)
  }}
  mechanism {{
{mechanism}
  }}
}}
part side : pine_19 {{
  length: 400mm
  width:  300mm
  place {{
    orient(length: up, width: back)
    against(base.top)
    flush(base.left)
    flush(base.back)
  }}
}}
part door : pine_19 {{
  length: 400mm
  width:  200mm
  place {{
    orient(length: up, width: right)
    against(base.top)
    flush(side.left, inset: 0.5mm)
    against(side.front)
  }}
}}
hardware hinge : test_hinge(door, on: side) {{
  place {{
    flush(door.bottom, inset: 80mm)
  }}
}}"
    )
}

const FOUR_PINS: &str = "    pivot arm, inner_link at point(across: 13.5mm, depth: 10.9mm)
    pivot cup, inner_link at point(across: -8.8mm, depth: 7.33mm)
    pivot arm, outer_link at point(across: 13.28mm, depth: 3.56mm)
    pivot cup, outer_link at point(across: -9.54mm, depth: 1.39mm)
    drive: turn(cup, from: 0deg, to: 110deg)";

#[test]
fn a_cup_hinge_moves_on_the_mechanism_its_kind_declares() {
    let opened = open(&hung_door(FOUR_PINS));
    assert!(
        diagnostics(&opened).is_empty(),
        "{:?}",
        diagnostics(&opened)
    );
    let model = opened.model.as_ref().unwrap();
    let kind = model
        .defs
        .kinds
        .iter()
        .find(|k| k.decl.name == "test_hinge")
        .unwrap();
    let bodies: Vec<&str> = kind.bodies.iter().map(|b| b.name.as_str()).collect();
    assert_eq!(bodies, ["arm", "cup", "inner_link", "outer_link"]);
    // The viewer's list keeps the base's words only.
    let members: Vec<&str> = kind.members.iter().map(|m| m.name.word()).collect();
    assert_eq!(members, ["plate", "arm", "cup"]);
    let mechanism = kind.mechanism.as_ref().unwrap();
    assert_eq!(mechanism.constraints.len(), 4);
    // The hinge's links are the kind's bars, its pins the kind's pivots.
    let hinge = common::hardware(design(&opened), "hinge")
        .mechanism
        .unwrap();
    assert_eq!(
        hinge.links.map(|link| (link.width, link.thickness)),
        [(mm("9"), mm("1.5")), (mm("14"), mm("1.5"))]
    );
    assert_eq!(
        hinge.linkage.opening_angle(),
        stave_lang::units::Angle::from_deg(110)
    );
}

#[test]
fn point_is_an_error_outside_a_kind() {
    let opened = open("param spot = point(across: 3mm)");
    let diagnostic = the_diagnostic(
        &opened,
        "syntax",
        "point(...) is written only inside a hardware kind's member or mechanism",
    );
    assert_eq!(body_line(&opened, diagnostic), 1);
    // So is a range anywhere but a member's shape.
    let opened = open(
        "kind small_knob : pull {\n  envelope: box(length: 10mm..20mm, width: 20mm, thickness: 20mm)\n}",
    );
    the_diagnostic(
        &opened,
        "syntax",
        "a range lo..hi is written only as an argument of a member's shape",
    );
}

#[test]
fn a_mechanism_needs_exactly_one_freedom() {
    // Leaving out the outer link's pin in the cup leaves three: the cup and
    // the two links move (nine), less two for each of three pins.
    let three_pins = FOUR_PINS.replace(
        "    pivot cup, outer_link at point(across: -9.54mm, depth: 1.39mm)\n",
        "",
    );
    let opened = open(&hung_door(&three_pins));
    the_diagnostic(
        &opened,
        "mechanism",
        "test_hinge's mechanism has 3 freedoms; it needs exactly one",
    );
    // One pin too many takes the last freedom away.
    let five_pins =
        format!("{FOUR_PINS}\n    pivot inner_link, outer_link at point(across: 2mm, depth: 5mm)");
    let opened = open(&hung_door(&five_pins));
    the_diagnostic(
        &opened,
        "mechanism",
        "test_hinge's mechanism has -1 freedoms; it needs exactly one",
    );
    // One freedom in another form is not supported yet.
    let slot = FOUR_PINS.replace(
        "    pivot cup, outer_link at point(across: -9.54mm, depth: 1.39mm)\n",
        "    slot cup, outer_link from point(across: -9.54mm, depth: 1.39mm) to point(across: -9.54mm, depth: 5mm)\n    couple outer_link to cup: 1/2\n",
    );
    let opened = open(&hung_door(&slot));
    the_diagnostic(
        &opened,
        "not-supported",
        "test_hinge's mechanism is not supported yet",
    );
}

#[test]
fn a_pivot_names_two_declared_members() {
    let misspelt = FOUR_PINS.replace("pivot arm, inner_link", "pivot ram, inner_link");
    let opened = open(&hung_door(&misspelt));
    let diagnostic = the_diagnostic(&opened, "unknown-name", "unknown name 'ram'");
    assert_eq!(diagnostic.help.as_deref(), Some("did you mean 'arm'?"));
    // A member without a body has nothing to pin.
    let bare = FOUR_PINS.replace("pivot arm, inner_link", "pivot plate, inner_link");
    let opened = open(&hung_door(&bare));
    the_diagnostic(&opened, "unknown-name", "unknown name 'plate'");
    // And the two are different members.
    let twice = FOUR_PINS.replace("pivot arm, inner_link", "pivot arm, arm");
    let opened = open(&hung_door(&twice));
    the_diagnostic(
        &opened,
        "duplicate-name",
        "'arm' is named twice in one pivot",
    );
}

#[test]
fn a_cup_hinge_kind_without_a_mechanism_gives_no_motion() {
    let opened =
        open(&hung_door(FOUR_PINS).replace(&format!("  mechanism {{\n{FOUR_PINS}\n  }}\n"), ""));
    let warnings = diagnostics(&opened);
    assert_eq!(
        warnings,
        ["not-supported: kind 'test_hinge' has no mechanism; its motion is not supported yet"]
    );
    assert!(
        common::hardware(design(&opened), "hinge")
            .mechanism
            .is_none()
    );
}
