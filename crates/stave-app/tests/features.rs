//! One small design per language feature, through the whole pipeline.

mod common;

use common::{design, diagnostics, extent, mm, only_error, open, open_files, part, ranges, solved};
use stave_lang::placement::Direction;
use stave_model::solved::{Contact, CutKind, Depth, PartOrigin};

/// A board standing on the base at its left end, for joints.
const LEFT_SIDE: &str = "
part side : pine_19 {
  length: 400mm
  width:  300mm
  place {
    orient(length: up, width: back)
    against(base.top)
    flush(base.left)
    flush(base.front)
  }
}
";

// ---------------------------------------------------------------------------
// Names
// ---------------------------------------------------------------------------

#[test]
fn a_part_may_be_called_like_a_face_word() {
    // `top` is a part; `top.top` is its top face; `up` is the axis.
    let opened = open(
        "part top : pine_19 {
  length: 600mm
  width:  300mm
  place {
    against(base.top, gap: 100mm)
    center(base, across)
    flush(base.front)
  }
}
part lid : pine_19 {
  length: 600mm
  width:  300mm
  place {
    against(top.top)
    flush(top.left)
    flush(top.front)
  }
}",
    );
    let design = design(&opened);
    assert_eq!(extent(part(design, "lid"))[1], (mm("138"), mm("157")));
}

#[test]
fn enum_words_win_where_they_are_expected() {
    // An item called `across` doesn't change `center(base, across)`.
    let opened = open(
        "let across = 10mm
part shelf : pine_19 {
  length: 200mm
  width:  across * 10
  place {
    against(base.top)
    center(base, across)
    flush(base.front)
  }
}",
    );
    let design = design(&opened);
    assert_eq!(extent(part(design, "shelf"))[0], (mm("200"), mm("400")));
}

#[test]
fn a_word_and_an_item_both_accepted_are_ambiguous() {
    let opened = open(
        "let through = 5mm
kind slot_joint : joint {
  cut crossing(into, piece) {
    notch(depth: through)
  }
}",
    );
    let found = diagnostics(&opened);
    assert!(
        found
            .iter()
            .any(|d| d == "ambiguous-name: 'through' could be a length or the word 'through' here"),
        "{found:?}"
    );
}

#[test]
fn a_builtin_and_a_param_of_the_same_name_coexist() {
    // `opening` is a param inside the component and a built-in outside it.
    let opened = open(
        "region space = opening(left: base.left, right: base.right, bottom: base.top, top: base.top, front: base.front, back: base.back)
component Holder {
  param opening : Region
  part block : pine_19 {
    length: opening.across
    width:  100mm
    place {
      flush(opening.left)
      flush(opening.bottom)
      flush(opening.front)
    }
  }
}
assembly holder : Holder {
  opening: space
}",
    );
    let found = diagnostics(&opened);
    // The region is flat (top equals bottom), which is fine to read sizes from.
    assert!(found.is_empty(), "{found:?}");
    assert_eq!(part(design(&opened), "holder.block").cut.length, mm("600"));
}

#[test]
fn unknown_names_and_duplicates_are_reported() {
    only_error(
        &open("let a = missing + 1mm"),
        "unknown-name",
        "unknown name 'missing'",
    );
    only_error(
        &open("let a = 1mm\nlet a = 2mm"),
        "duplicate-name",
        "'a' is declared twice",
    );
}

#[test]
fn names_are_shared_by_every_file_of_a_module() {
    let opened = open_files(&[
        ("a.stave", common::HEADER),
        (
            "b.stave",
            "stave 1\n\npart shelf : pine_19 {\n  length: 100mm\n  width: 100mm\n  place {\n    against(base.top)\n    flush(base.left)\n    flush(base.front)\n  }\n}\n",
        ),
    ]);
    assert_eq!(
        extent(part(design(&opened), "shelf"))[1],
        (mm("19"), mm("38"))
    );
    let twice = open_files(&[
        ("a.stave", common::HEADER),
        ("b.stave", "stave 1\n\nlet base = 1mm\n"),
    ]);
    only_error(&twice, "duplicate-name", "'base' is declared twice");
}

#[test]
fn modules_are_used_qualified_and_private_items_stay_inside() {
    let library = "stave 1\n\nlibrary \"Parts\"\n\nlet width = 120mm\nprivate let secret = 5mm\n";
    let project = format!(
        "stave 1\n\nimport \"./parts\"\n{}\nlet a = parts.width\nlet b = parts.secret\n",
        &common::HEADER["stave 1\n".len()..]
    );
    let opened = open_files(&[
        ("project.stave", &project),
        ("parts/library.stave", library),
    ]);
    only_error(&opened, "unknown-name", "unknown name 'parts.secret'");
}

#[test]
fn a_kind_hiding_the_prelude_is_a_warning() {
    let opened = open("kind dado : joint {\n  note: \"mine\"\n}");
    assert_eq!(
        diagnostics(&opened),
        ["shadows-prelude: 'dado' hides the standard kind of the same name"]
    );
    assert!(opened.design.is_some());
}

// ---------------------------------------------------------------------------
// Arrays
// ---------------------------------------------------------------------------

#[test]
fn labelled_arrays_are_indexed_by_label_and_same_is_elementwise() {
    let opened = open(
        "part post[left, right] : pine_19 {
  length: 100mm
  width:  19mm
  place {
    orient(length: up, width: back)
    against(base.top)
    spaced(ends, from: base.left, to: base.right)
    flush(base.front)
  }
}
part cap[left, right] : pine_19 {
  length: 50mm
  width:  19mm
  place {
    against(post[same].top)
    flush(post[same].left)
    flush(post[same].front)
  }
}",
    );
    let design = design(&opened);
    assert_eq!(
        extent(part(design, "post[right]"))[0],
        (mm("581"), mm("600"))
    );
    assert_eq!(
        extent(part(design, "cap[right]")),
        ranges([("581", "631"), ("119", "138"), ("0", "19")])
    );
}

#[test]
fn a_label_is_not_an_item_and_numbers_index_counted_arrays() {
    only_error(
        &open(
            "part post[left, right] : pine_19 {\n  length: 100mm\n  width: 19mm\n  place {\n    against(base.top)\n    spaced(evenly, from: base.left, to: base.right)\n    flush(base.front)\n  }\n}\nlet x = post[0].length",
        ),
        "unknown-name",
        "unknown name '0'",
    );
    let opened = open(
        "part slat[3] : pine_12 {\n  length: 100mm\n  width: 50mm\n  place {\n    against(base.top)\n    spaced(evenly, from: base.left, to: base.right)\n    flush(base.front)\n  }\n}\nlet x = slat[2].length",
    );
    let design = design(&opened);
    // evenly: gaps of (600 - 300) / 4 = 75.
    assert_eq!(extent(part(design, "slat[0]"))[0], (mm("75"), mm("175")));
    assert_eq!(extent(part(design, "slat[2]"))[0], (mm("425"), mm("525")));
}

#[test]
fn an_unindexed_array_name_needs_star() {
    let opened = open(
        "part post[left, right] : pine_19 {\n  length: 100mm\n  width: 19mm\n  place {\n    against(base.top)\n    spaced(ends, from: base.left, to: base.right)\n    flush(base.front)\n  }\n}\nlet x = post.length",
    );
    only_error(&opened, "type-mismatch", "'post' is an array of parts");
}

#[test]
fn same_needs_matching_labels() {
    let opened = open(
        "part post[left, right] : pine_19 {\n  length: 100mm\n  width: 19mm\n  place {\n    against(base.top)\n    spaced(ends, from: base.left, to: base.right)\n    flush(base.front)\n  }\n}
part cap[a, b] : pine_19 {\n  length: 50mm\n  width: 19mm\n  place {\n    against(post[same].top)\n    flush(post[same].left)\n    flush(post[same].front)\n  }\n}",
    );
    assert!(
        diagnostics(&opened)
            .iter()
            .any(|d| d.starts_with("type-mismatch: 'post[same]' needs the same labels as 'cap'")),
        "{:?}",
        diagnostics(&opened)
    );
}

// ---------------------------------------------------------------------------
// Components and kind values
// ---------------------------------------------------------------------------

#[test]
fn components_are_instantiated_per_assembly_with_names_resolved_where_written() {
    let opened = open(
        "let gap = 1mm
component Box {
  param height = 50mm
  let gap = 10mm
  part wall : pine_12 {
    length: 100mm
    width:  height
    place {
      orient(length: right, width: up)
      against(base.top, gap: gap)
      flush(base.left)
      flush(base.front)
    }
  }
}
assembly small : Box { }
assembly tall : Box {
  height: 80mm + gap
}",
    );
    let design = solved(&opened);
    // Inside the component, `gap` is the component's (10mm); the argument
    // is evaluated where the assembly is written (1mm).
    assert_eq!(extent(part(design, "small.wall"))[1], (mm("29"), mm("79")));
    assert_eq!(extent(part(design, "tall.wall"))[1], (mm("29"), mm("110")));
}

#[test]
fn a_component_param_without_a_default_must_be_given() {
    let opened = open("component Box {\n  param height : Length\n}\nassembly b : Box { }");
    only_error(&opened, "missing-field", "assembly 'b' is missing 'height'");
    let unknown =
        open("component Box {\n  param height = 1mm\n}\nassembly b : Box {\n  depth: 2mm\n}");
    only_error(
        &unknown,
        "unknown-field",
        "the component 'Box' has no field 'depth'",
    );
}

#[test]
fn kind_values_carry_settings_and_are_readable() {
    let opened = open(
        "let slide = side_mount_slide(length: 450mm)\nlet a = slide.length\nlet b = slide.clearance\nrequire long_enough : a == 450mm && b == 12.7mm \"settings are readable\"",
    );
    let design = design(&opened);
    assert!(
        design
            .requires
            .iter()
            .any(|r| r.name == "long_enough" && r.holds)
    );
    only_error(
        &open("let slide = side_mount_slide(length: 450mm, colour: 1)"),
        "unknown-field",
        "the kind 'side_mount_slide' has no field 'colour'",
    );
}

// ---------------------------------------------------------------------------
// Joints
// ---------------------------------------------------------------------------

#[test]
fn joint_kinds_are_evaluated_per_pair() {
    // The dado's default depth is into.thickness / 3, different per pair.
    let opened = open(&format!(
        "{LEFT_SIDE}
part thin : pine_12 {{
  length: 400mm
  width:  300mm
  place {{
    orient(length: up, width: back)
    against(base.top)
    flush(base.right)
    flush(base.front)
  }}
}}
part shelf : pine_19 {{
  length: fit
  width:  300mm
  place {{
    against(base.top, gap: 100mm)
    center(from: side.right, to: thin.left)
    flush(base.front)
  }}
}}
joint housings : dado(shelf, into: [side, thin])"
    ));
    let design = design(&opened);
    let shelf = part(design, "shelf");
    // Between the side (19) and thin (588): 569, plus 19/3 and 12/3.
    assert_eq!(shelf.drawn.length, mm("569"));
    assert_eq!(shelf.cut.length, mm("579.333333"));
    let side = part(design, "side");
    assert!(side.cuts.iter().any(|c| c.kind
        == CutKind::Notch {
            depth: Depth::Length(mm("6.333333"))
        }));
    let thin = part(design, "thin");
    assert!(thin.cuts.iter().any(|c| c.kind
        == CutKind::Notch {
            depth: Depth::Length(mm("4"))
        }));
}

#[test]
fn extend_follows_the_contact_and_fit_comes_from_joints() {
    let opened = open(&format!(
        "{LEFT_SIDE}
part panel : pine_12 {{
  length: 200mm
  width:  fit
  place {{
    orient(length: up, width: right)
    against(base.top)
    against(side.right)
    flush(base.front)
  }}
}}
part stop : pine_19 {{
  length: 300mm
  width:  200mm
  place {{
    orient(length: back, width: up)
    against(base.top)
    flush(base.left, inset: 300mm)
    flush(base.front)
  }}
}}
joint panel_grooves : groove(panel, into: [side, stop], depth: 5mm)"
    ));
    let design = design(&opened);
    let panel = part(design, "panel");
    // From the side's right face (19) to the stop's left face (300).
    assert_eq!(panel.drawn.width, mm("281"));
    assert_eq!(panel.cut.width, mm("291"));
    assert_eq!(panel.drawn_offset.width, mm("5"));
    let joint = &design.joints[0];
    assert!(
        joint
            .pairs
            .iter()
            .all(|p| matches!(p.contact, Contact::Butt { .. }))
    );
}

#[test]
fn fit_without_a_joined_part_on_one_side_is_undecided() {
    let opened = open(&format!(
        "{LEFT_SIDE}
part shelf : pine_19 {{
  length: fit
  width:  300mm
  place {{
    against(base.top, gap: 100mm)
    center(base, across)
    flush(base.front)
  }}
}}
joint housing : dado(shelf, into: side)"
    ));
    only_error(
        &opened,
        "fit-undecided",
        "shelf: nothing decides its length",
    );
}

#[test]
fn a_failing_kind_require_is_reported_at_the_joint() {
    let opened = open(&format!(
        "{LEFT_SIDE}
part shelf : pine_19 {{
  length: 200mm
  width:  300mm
  place {{
    against(base.top, gap: 100mm)
    against(side.right)
    flush(base.front)
  }}
}}
joint housing : dado(shelf, into: side, depth: 15mm)"
    ));
    only_error(
        &opened,
        "require",
        "'not_too_deep' fails: a dado deeper than half the board weakens it",
    );
}

// ---------------------------------------------------------------------------
// Regions
// ---------------------------------------------------------------------------

#[test]
fn regions_split_and_cover_with_meeting_edges() {
    let opened = open(
        "region space = opening(left: base.left, right: base.right, bottom: base.top, top: base.top, front: base.front, back: base.back)
region tall = opening(left: base.left, right: base.right, bottom: base.bottom, top: base.top, front: base.front, back: base.back)
region half[left, right] = split(tall, across)
region face = cover(half[left], overlay(left: 5mm, right: 5mm, meeting: -1mm), thickness: 19mm)
region inside = cover(tall, inset(reveal: 2mm), thickness: 10mm)
part door : pine_19 {
  length: face.across
  width:  face.up
  place {
    orient(length: right, width: up)
    flush(face.left)
    flush(face.bottom)
    flush(face.front)
  }
}",
    );
    let design = design(&opened);
    let region = |name: &str| {
        design
            .regions
            .iter()
            .find(|r| r.name == name)
            .unwrap()
            .bounds
    };
    assert_eq!(
        (
            region("half[left]").min().across,
            region("half[left]").max().across
        ),
        (mm("0"), mm("300"))
    );
    let face = region("face");
    // Left edge grows 5; the right edge is shared, so it gets the meeting value.
    assert_eq!(
        (
            face.min().across,
            face.max().across,
            face.min().deep,
            face.max().deep
        ),
        (mm("-5"), mm("299"), mm("-19"), mm("0"))
    );
    let inside = region("inside");
    assert_eq!(
        (inside.min().across, inside.max().across, inside.max().deep),
        (mm("2"), mm("598"), mm("10"))
    );
    assert_eq!(part(design, "door").cut.length, mm("304"));
}

#[test]
fn split_at_a_part_ends_at_its_faces() {
    let opened = open(
        "part shelf : pine_19 {\n  length: 600mm\n  width: 300mm\n  place {\n    against(base.top, gap: 100mm)\n    flush(base.left)\n    flush(base.front)\n  }\n}
part lid : pine_19 {\n  length: 600mm\n  width: 300mm\n  place {\n    against(base.top, gap: 300mm)\n    flush(base.left)\n    flush(base.front)\n  }\n}
region space = opening(left: base.left, right: base.right, bottom: base.top, top: lid.bottom, front: base.front, back: base.back)
region bay[low, high] = split(space, up, at: shelf)",
    );
    let design = design(&opened);
    let bay = |name: &str| {
        design
            .regions
            .iter()
            .find(|r| r.name == name)
            .unwrap()
            .bounds
    };
    assert_eq!(
        (bay("bay[low]").min().up, bay("bay[low]").max().up),
        (mm("19"), mm("119"))
    );
    assert_eq!(
        (bay("bay[high]").min().up, bay("bay[high]").max().up),
        (mm("138"), mm("319"))
    );
}

// ---------------------------------------------------------------------------
// Mirroring
// ---------------------------------------------------------------------------

#[test]
fn mirrored_assemblies_rebind_outside_parts() {
    let opened = open(
        "part wall[left, right] : pine_19 {\n  length: 200mm\n  width: 300mm\n  place {\n    orient(length: up, width: back)\n    against(base.top)\n    spaced(ends, from: base.left, to: base.right)\n    flush(base.front)\n  }\n}
component Flap {
  param hinge_side : Part
  part leaf : pine_12 {
    length: 200mm
    width:  100mm
    place {
      orient(length: up, width: right)
      against(base.top)
      against(hinge_side.right)
      against(base.front)
    }
  }
  hardware hinge : euro_hinge(leaf, on: hinge_side) {
    place {
      flush(leaf.bottom, inset: 20mm)
    }
  }
}
assembly flap_l : Flap {
  hinge_side: wall[left]
}
assembly flap_r = mirror(flap_l, about: mid(base, across))",
    );
    let design = design(&opened);
    let leaf = part(design, "flap_r.leaf");
    assert_eq!(extent(leaf)[0], (mm("481"), mm("581")));
    assert_eq!(leaf.pose.orientation.width(), Direction::Left);
    assert!(matches!(leaf.origin, PartOrigin::MirroredAssembly { .. }));
    let hinge = design
        .hardware
        .iter()
        .find(|h| h.name == "flap_r.hinge")
        .unwrap();
    let wall_right = part(design, "wall[right]").id;
    assert!(
        matches!(hinge.data, stave_model::solved::HardwareData::Hinge { on, .. } if on == wall_right)
    );
}

#[test]
fn a_mirror_without_a_counterpart_is_an_error() {
    let opened = open(
        "part wall : pine_19 {\n  length: 200mm\n  width: 300mm\n  place {\n    orient(length: up, width: back)\n    against(base.top)\n    flush(base.left)\n    flush(base.front)\n  }\n}
component Flap {
  param hinge_side : Part
  part leaf : pine_12 {
    length: 200mm
    width:  100mm
    place {
      orient(length: up, width: right)
      against(base.top)
      against(hinge_side.right)
      against(base.front)
    }
  }
  hardware hinge : euro_hinge(leaf, on: hinge_side) {
    place {
      flush(leaf.bottom, inset: 20mm)
    }
  }
}
assembly flap_l : Flap {
  hinge_side: wall
}
assembly flap_r = mirror(flap_l, about: mid(base, across))",
    );
    only_error(
        &opened,
        "mirror",
        "wall has no counterpart on the other side of the mirror",
    );
}

// ---------------------------------------------------------------------------
// Placement terms
// ---------------------------------------------------------------------------

fn block(place: &str) -> String {
    format!(
        "part block : pine_19 {{\n  length: 100mm\n  width:  50mm\n  place {{\n{place}\n  }}\n}}"
    )
}

#[test]
fn against_and_flush_with_gaps_and_insets() {
    let opened = open(&block(
        "    against(base.top, gap: 10mm)\n    flush(base.right, inset: 5mm)\n    flush(base.back)",
    ));
    assert_eq!(
        extent(part(design(&opened), "block")),
        ranges([("495", "595"), ("29", "48"), ("250", "300")])
    );
    let opened = open(&block(
        "    against(base.bottom)\n    against(base.left, gap: 1mm)\n    against(base.front)",
    ));
    assert_eq!(
        extent(part(design(&opened), "block")),
        ranges([("-101", "-1"), ("-19", "0"), ("-50", "0")])
    );
}

#[test]
fn center_on_an_item_and_between_planes() {
    let opened = open(&block(
        "    against(base.top)\n    center(base, across)\n    center(from: base.front, to: mid(base, deep))",
    ));
    assert_eq!(
        extent(part(design(&opened), "block")),
        ranges([("250", "350"), ("19", "38"), ("50", "100")])
    );
}

#[test]
fn spaced_pitch_and_gap() {
    let element = |spacing: &str| {
        format!(
            "part peg[3] : pine_12 {{\n  length: 20mm\n  width: 20mm\n  place {{\n    against(base.top)\n    spaced({spacing}, from: base.left, to: base.right)\n    flush(base.front)\n  }}\n}}"
        )
    };
    let pitch = open(&element("pitch(100mm)"));
    assert_eq!(
        extent(part(design(&pitch), "peg[2]"))[0],
        (mm("200"), mm("220"))
    );
    let gap = open(&element("gap(5mm)"));
    assert_eq!(
        extent(part(design(&gap), "peg[2]"))[0],
        (mm("50"), mm("70"))
    );
    let inset = open(&element("ends(inset: 10mm)"));
    assert_eq!(
        extent(part(design(&inset), "peg[1]"))[0],
        (mm("290"), mm("310"))
    );
    // From the right to the left: elements run leftward.
    let reversed = open(&element("ends").replace(
        "from: base.left, to: base.right",
        "from: base.right, to: base.left",
    ));
    assert_eq!(
        extent(part(design(&reversed), "peg[0]"))[0],
        (mm("580"), mm("600"))
    );
}

#[test]
fn corners_place_the_four_labels() {
    let opened = open(
        "part leg[front_left, front_right, back_left, back_right] : pine_19 {\n  length: 100mm\n  width: 19mm\n  place {\n    orient(length: down, width: right)\n    corners(base, inset: 10mm)\n    against(base.bottom)\n  }\n}",
    );
    let design = design(&opened);
    assert_eq!(
        extent(part(design, "leg[back_right]")),
        ranges([("571", "590"), ("-100", "0"), ("271", "290")])
    );
    let bad = open(
        "part leg[a, b] : pine_19 {\n  length: 100mm\n  width: 19mm\n  place {\n    orient(length: down, width: right)\n    corners(base, inset: 10mm)\n    against(base.bottom)\n  }\n}",
    );
    assert!(
        diagnostics(&bad)
            .iter()
            .any(|d| d.starts_with("type-mismatch: 'corners' needs the labels"))
    );
}

#[test]
fn orient_turns_a_part_and_needs_perpendicular_directions() {
    let opened = open(&block(
        "    orient(length: back, width: up)\n    against(base.top)\n    flush(base.left)\n    flush(base.front)",
    ));
    let block_part = part(design(&opened), "block");
    assert_eq!(
        extent(block_part),
        ranges([("0", "19"), ("19", "69"), ("0", "100")])
    );
    assert_eq!(block_part.pose.orientation.thickness(), Direction::Right);
    let bad = open(&block(
        "    orient(length: up, width: down)\n    against(base.top)\n    flush(base.left)\n    flush(base.front)",
    ));
    only_error(
        &bad,
        "type-mismatch",
        "'orient' needs a length and width along different axes, found up and down",
    );
}

// ---------------------------------------------------------------------------
// Placement errors
// ---------------------------------------------------------------------------

#[test]
fn every_axis_is_set_exactly_once() {
    only_error(
        &open(&block("    against(base.top)\n    flush(base.left)")),
        "axis-not-set",
        "block: position along 'deep' is not set",
    );
    only_error(
        &open(&block(
            "    against(base.top)\n    flush(base.left)\n    flush(base.right)\n    flush(base.front)",
        )),
        "axis-set-twice",
        "block: position along 'across' is set twice",
    );
}

#[test]
fn exactly_one_part_is_ground() {
    only_error(
        &open(&block("    ground")),
        "ground",
        "'base' and 'block' are both placed with 'ground'",
    );
    let none = open_files(&[(
        "project.stave",
        "stave 1\n\nproject \"T\" {\n  units: mm\n}\n\nstock s { species: pine, form: solid, thickness: 19mm }\n\npart a : s {\n  length: 1mm\n  width: 1mm\n  place {\n    against(b.top)\n    flush(b.left)\n    flush(b.front)\n  }\n}\npart b : s {\n  length: 1mm\n  width: 1mm\n  place {\n    against(a.top)\n    flush(a.left)\n    flush(a.front)\n  }\n}\n",
    )]);
    only_error(&none, "ground", "no part is placed with 'ground'");
}

#[test]
fn parts_placed_against_each_other_are_a_cycle() {
    let opened = open(
        "part a : pine_19 {\n  length: 100mm\n  width: 100mm\n  place {\n    against(b.top)\n    flush(base.left)\n    flush(base.front)\n  }\n}
part b : pine_19 {\n  length: 100mm\n  width: 100mm\n  place {\n    against(a.top)\n    flush(base.left)\n    flush(base.front)\n  }\n}",
    );
    let found = diagnostics(&opened);
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(
        found[0] == "placement-cycle: b and a are placed against each other"
            || found[0] == "placement-cycle: a and b are placed against each other",
        "{found:?}"
    );
}

#[test]
fn hardware_on_its_mount_face() {
    let opened = open(&format!(
        "{}\nhardware handle : knob(block) {{\n  place {{\n    center(block, across)\n    center(block, up)\n  }}\n}}",
        block(
            "    orient(length: right, width: up)\n    against(base.top)\n    flush(base.left)\n    flush(base.front)"
        )
    ));
    let design = design(&opened);
    let knob = &design.hardware[0];
    // A 30mm knob standing 28mm off the front face (deep 0).
    assert_eq!(
        (knob.bounds.min().deep, knob.bounds.max().deep),
        (mm("-28"), mm("0"))
    );
    assert_eq!(
        (knob.bounds.min().across, knob.bounds.max().across),
        (mm("35"), mm("65"))
    );
    let twice = open(&format!(
        "{}\nhardware handle : knob(block) {{\n  place {{\n    center(block, across)\n    center(block, up)\n    center(block, deep)\n  }}\n}}",
        block(
            "    orient(length: right, width: up)\n    against(base.top)\n    flush(base.left)\n    flush(base.front)"
        )
    ));
    only_error(
        &twice,
        "axis-set-twice",
        "handle: position along 'deep' is set twice",
    );
}

#[test]
fn values_outside_within_are_out_of_range() {
    let opened = open(
        "component Box {\n  param height = 50mm within 40mm..60mm\n}\nassembly b : Box {\n  height: 70mm\n}",
    );
    only_error(
        &opened,
        "out-of-range",
        "'height' must be within 40mm..60mm, found 70mm",
    );
    let kind = open(&format!(
        "{LEFT_SIDE}\npart drawer_side : pine_12 {{\n  length: 300mm\n  width: 100mm\n  place {{\n    orient(length: back, width: up)\n    against(base.top)\n    against(side.right, gap: 12.7mm)\n    flush(base.front)\n  }}\n}}\nregion space = opening(left: side.right, right: base.right, bottom: base.top, top: side.top, front: base.front, back: base.back)\nhardware runner : side_mount_slide(drawer_side, in: space, length: 320mm)"
    ));
    only_error(
        &kind,
        "out-of-range",
        "'length' must be within [250mm, 300mm, 350mm, 400mm, 450mm, 500mm, 550mm, 600mm], found 320mm",
    );
}

// ---------------------------------------------------------------------------
// Checking per item
// ---------------------------------------------------------------------------

#[test]
fn a_name_error_poisons_only_its_item() {
    // `typo` has an unknown stock; `leaning` depends on it and is skipped
    // silently; `floating` is unrelated and still reports its own error.
    let opened = open(
        "part typo : pine_18 {
  length: 100mm
  width:  100mm
  place {
    against(base.top)
    flush(base.left)
    flush(base.front)
  }
}
part leaning : pine_19 {
  length: 100mm
  width:  100mm
  place {
    against(typo.top)
    flush(typo.left)
    flush(typo.front)
  }
}
part floating : pine_19 {
  length: 100mm
  width:  100mm
  place {
    against(base.top, gap: 200mm)
    flush(base.left)
  }
}",
    );
    assert_eq!(
        diagnostics(&opened),
        [
            "unknown-name: unknown name 'pine_18'",
            "axis-not-set: floating: position along 'deep' is not set",
        ]
    );
    assert!(opened.design.is_none());
}

#[test]
fn a_syntax_error_leaves_out_only_its_item() {
    let opened = open(
        "part broken : pine_19 {
  length:
  width:  100mm
  place {
    against(base.top)
    flush(base.left)
    flush(base.front)
  }
}
part uses_broken : pine_19 {
  length: broken.length
  width:  100mm
  place {
    against(broken.top)
    flush(base.left)
    flush(base.front)
  }
}
part floating : pine_19 {
  length: 100mm
  width:  100mm
  place {
    against(base.top, gap: 200mm)
    flush(base.left)
  }
}",
    );
    assert_eq!(
        diagnostics(&opened),
        [
            "missing-value: 'length' has no value",
            "axis-not-set: floating: position along 'deep' is not set",
        ]
    );
}

#[test]
fn a_misspelt_field_is_one_error_not_two() {
    // The unknown field's help names the field; it is not also missing.
    let part = open(
        "part shelf : pine_19 {\n  length: 100mm\n  widht:  100mm\n  place {\n    against(base.top)\n    flush(base.left)\n    flush(base.front)\n  }\n}",
    );
    only_error(&part, "unknown-field", "part 'shelf' has no field 'widht'");
    assert_eq!(
        part.diagnostics[0].help.as_deref(),
        Some("did you mean 'width'?")
    );
    let required = open(
        "part shelf : pine_19 {\n  lenght: 100mm\n  width:  100mm\n  place {\n    against(base.top)\n    flush(base.left)\n    flush(base.front)\n  }\n}",
    );
    only_error(
        &required,
        "unknown-field",
        "part 'shelf' has no field 'lenght'",
    );
    let stock = open("stock ash_20 { species: ash, form: solid, thicknes: 20mm }");
    only_error(
        &stock,
        "unknown-field",
        "stock 'ash_20' has no field 'thicknes'",
    );
    let param =
        open("component Box {\n  param height : Length\n}\nassembly b : Box {\n  hieght: 2mm\n}");
    only_error(
        &param,
        "unknown-field",
        "the component 'Box' has no field 'hieght'",
    );
    let role = open(&format!(
        "{LEFT_SIDE}\npart shelf : pine_19 {{\n  length: 200mm\n  width: 300mm\n  place {{\n    against(base.top, gap: 100mm)\n    against(side.right)\n    flush(base.front)\n  }}\n}}\njoint housing : dado(shelf, int: side)"
    ));
    only_error(&role, "unknown-field", "the kind 'dado' has no field 'int'");
    let entry = open(
        "kind runner : slide {\n  lenght: 500mm\n  clearance: 12.7mm\n  travel: 500mm\n  envelope: box(length: 500mm, width: 45mm, thickness: 12.7mm)\n}",
    );
    only_error(
        &entry,
        "unknown-field",
        "a slide kind has no field 'lenght'",
    );
}

// ---------------------------------------------------------------------------
// Butt hinges
// ---------------------------------------------------------------------------

/// A box back standing at the back of the base, and a lid on it hinged with
/// two butt hinges housed in the lid's underside and the back's top edge.
const BUTT_HINGED_LID: &str = "
part back : pine_12 {
  length: 600mm
  width:  100mm
  place {
    orient(length: right, width: up)
    against(base.top)
    flush(base.left)
    flush(base.back)
  }
}
component Lid {
  param on    : Part
  param under : Part
  part panel : pine_12 {
    length: under.length
    width:  under.width
    place {
      against(on.top)
      flush(under.left)
      flush(under.front)
    }
  }
  hardware hinge[left, right] : butt_hinge(panel, on: on, face: bottom) {
    place {
      spaced(ends(inset: 50mm), from: panel.left, to: panel.right)
    }
  }
}
assembly lid : Lid {
  on:    back
  under: base
}
";

#[test]
fn butt_hinge_leaves_are_housed_in_both_parts_at_the_flush_edge() {
    let opened = open(BUTT_HINGED_LID);
    let design = design(&opened);
    // The back is 12 thick at deep 288..300; the lid sits on it at 119.
    // A 40mm hinge has 12mm leaves, 1.5mm thick; the lid's back edge is
    // flush with the back's back face, so the pin is there.
    let hinge = design
        .hardware
        .iter()
        .find(|h| h.name == "lid.hinge[left]")
        .unwrap();
    assert!(hinge.is_butt_hinge());
    let bounds = |b: &stave_model::solved::WorldBox| {
        [
            (b.min().across, b.max().across),
            (b.min().up, b.max().up),
            (b.min().deep, b.max().deep),
        ]
    };
    assert_eq!(
        bounds(&hinge.bounds),
        ranges([("50", "90"), ("119", "120.5"), ("288", "300")])
    );
    let fixed = hinge.fixed.as_ref().unwrap();
    assert_eq!(
        bounds(&fixed.bounds),
        ranges([("50", "90"), ("117.5", "119"), ("288", "300")])
    );
    let back = part(design, "back");
    assert!(
        matches!(hinge.data, stave_model::solved::HardwareData::ButtHinge { on, .. } if on == back.id)
    );
    // A mortise as deep as a leaf in each part, where the leaf lies.
    let recesses = |name: &str| -> Vec<(stave_model::solved::LocalBox, Depth)> {
        part(design, name)
            .cuts
            .iter()
            .filter_map(|cut| match (&cut.place, &cut.kind) {
                (stave_model::solved::CutPlace::Recess { footprint }, CutKind::Notch { depth }) => {
                    Some((*footprint, *depth))
                }
                _ => None,
            })
            .collect()
    };
    let in_back = recesses("back");
    assert_eq!(in_back.len(), 2);
    let (footprint, depth) = in_back[0];
    assert_eq!(depth, Depth::Length(mm("1.5")));
    // The back's frame: length right, width up from 19, thickness back.
    assert_eq!(
        (footprint.min.length, footprint.max.length),
        (mm("50"), mm("90"))
    );
    assert_eq!(
        (footprint.min.width, footprint.max.width),
        (mm("98.5"), mm("100"))
    );
    assert_eq!(recesses("lid.panel").len(), 2);
    // The lid turns about its pin edge, lifting its front.
    let lid = design.assemblies.iter().find(|a| a.name == "lid").unwrap();
    let Some(stave_model::solved::Motion::Turn { pivot, .. }) = lid.motion else {
        panic!("the lid has no turn: {:?}", lid.motion);
    };
    assert_eq!(
        (pivot.through.up, pivot.through.deep),
        (mm("119"), mm("300"))
    );
    assert_eq!(pivot.free_side, Direction::Front);
    assert_eq!(pivot.swing_toward, Direction::Up);
}

#[test]
fn a_mirrored_butt_hinged_door_cuts_its_mortise_in_the_other_side() {
    let opened = open(
        "part wall[left, right] : pine_19 {\n  length: 200mm\n  width: 300mm\n  place {\n    orient(length: up, width: back)\n    against(base.top)\n    spaced(ends, from: base.left, to: base.right)\n    flush(base.front)\n  }\n}
component Flap {
  param hinge_side : Part
  part leaf : pine_12 {
    length: 200mm
    width:  100mm
    place {
      orient(length: up, width: right)
      against(base.top)
      against(hinge_side.right)
      flush(base.front)
    }
  }
  hardware hinge : butt_hinge(leaf, on: hinge_side, face: left) {
    place {
      flush(leaf.bottom, inset: 20mm)
    }
  }
}
assembly flap_l : Flap {
  hinge_side: wall[left]
}
assembly flap_r = mirror(flap_l, about: mid(base, across))",
    );
    let design = design(&opened);
    let recess = |name: &str| {
        part(design, name)
            .cuts
            .iter()
            .find_map(|cut| match cut.place {
                stave_model::solved::CutPlace::Recess { footprint } => Some(footprint),
                _ => None,
            })
            .unwrap_or_else(|| panic!("{name} has no recess"))
    };
    // The door's edge is flush with the wall's front, so the pin is at the
    // front and the leaves reach 12mm back. The wall's thickness runs right,
    // so the left wall's mortise is at its right face and the right wall's
    // at its left face.
    let left = recess("wall[left]");
    assert_eq!(
        (left.min.thickness, left.max.thickness),
        (mm("17.5"), mm("19"))
    );
    assert_eq!((left.min.width, left.max.width), (mm("0"), mm("12")));
    let right = recess("wall[right]");
    assert_eq!(
        (right.min.thickness, right.max.thickness),
        (mm("0"), mm("1.5"))
    );
    assert_eq!((right.min.length, right.max.length), (mm("20"), mm("60")));
    let mirrored = design
        .hardware
        .iter()
        .find(|h| h.name == "flap_r.hinge")
        .unwrap();
    let wall_right = part(design, "wall[right]").id;
    assert!(
        matches!(mirrored.data, stave_model::solved::HardwareData::ButtHinge { on, .. } if on == wall_right)
    );
    recess("flap_r.leaf");
}

#[test]
fn a_hinge_kind_is_a_cup_hinge_or_a_butt_hinge() {
    let neither = open("kind flap_hinge : hinge {\n  opening_angle: 90deg\n}");
    only_error(
        &neither,
        "missing-field",
        "kind 'flap_hinge' is missing 'leaf'",
    );
    let both = open(
        "kind odd_hinge : hinge {\n  opening_angle: 90deg\n  cup_diameter:  35mm\n  leaf:          box(length: 40mm, width: 12mm, thickness: 1.5mm)\n}",
    );
    only_error(
        &both,
        "type-mismatch",
        "kind 'odd_hinge' has both a leaf and a cup; a hinge is a cup hinge or a butt hinge",
    );
    let half_cup = open(
        "kind odd_hinge : hinge {\n  opening_angle: 90deg\n  cup_diameter:  35mm\n  cup_depth:     13mm\n  cup_inset:     5mm\n  plate_setback: 37mm\n}",
    );
    only_error(
        &half_cup,
        "missing-field",
        "kind 'odd_hinge' is missing 'plate'",
    );
}

#[test]
fn a_butt_hinge_needs_its_parts_to_meet_at_the_mount_face() {
    // face: top houses the leaf in the lid's top, which touches nothing.
    let opened = open(&BUTT_HINGED_LID.replace("face: bottom", "face: top"));
    assert_eq!(
        diagnostics(&opened),
        [
            "no-contact: lid.hinge[left]: lid.panel meets none of the parts it should join",
            "no-contact: lid.hinge[right]: lid.panel meets none of the parts it should join",
        ]
    );
}

// ---------------------------------------------------------------------------
// Built-in functions
// ---------------------------------------------------------------------------

#[test]
fn extent_along_another_parts_axis() {
    // A shelf screwed down into the top edge of a 120mm bracket: the screw
    // bites into the bracket's width, not its thickness.
    let opened = open(
        "part bracket : pine_19 {
  length: 300mm
  width:  120mm
  place {
    orient(length: back, width: up)
    against(base.bottom)
    flush(base.left)
    flush(base.front)
  }
}
let bite = extent(bracket, thickness, of: base)
require deep_enough : bite == 120mm \"the bracket is 120mm tall under the base\"
joint screws : screw(base, into: bracket, length: 50mm)",
    );
    let design = design(&opened);
    assert!(
        design.requires.iter().all(|r| r.holds),
        "{:?}",
        design.requires
    );
    // Into a face, the same screw would come out the other side.
    let flat = open(
        "part cleat : pine_19 {
  length: 300mm
  width:  60mm
  place {
    against(base.bottom)
    flush(base.left)
    flush(base.front)
  }
}
joint screws : screw(base, into: cleat, length: 50mm)",
    );
    assert_eq!(
        diagnostics(&flat),
        ["require: 'stays_inside' fails: the screw would come out the other side"]
    );
}

#[test]
fn between_parts_that_overlap_says_so() {
    let opened = open(
        "part block : pine_19 {
  length: 100mm
  width:  50mm
  place {
    center(base, up)
    flush(base.left)
    flush(base.front)
  }
}
let gap = between(base, block)",
    );
    only_error(
        &opened,
        "type-mismatch",
        "'between' needs parts apart along exactly one axis, but base and block touch or overlap",
    );
}

#[test]
fn hardware_named_after_its_kind_is_explained() {
    let opened = open(&format!(
        "{}\nhardware knob : knob(block) {{\n  place {{\n    center(block, across)\n    center(block, up)\n  }}\n}}",
        block(
            "    orient(length: right, width: up)\n    against(base.top)\n    flush(base.left)\n    flush(base.front)"
        )
    ));
    only_error(
        &opened,
        "type-mismatch",
        "'knob' here is this hardware itself, not a kind",
    );
}
