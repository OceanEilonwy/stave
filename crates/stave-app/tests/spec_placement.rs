//! Executable spec: "Placement" (terms, rules, hardware placement,
//! mirroring), "Sizes" and "Regions". Every position below was worked out
//! by hand from the grammar before the test was written. The base is
//! 600 (across) x 19 (up) x 300 (deep) at the origin.

mod common;

use common::{
    bounds, design, diagnostics, extent, hardware, holds, mm, only_error, open, part, ranges,
    region, solved, the_diagnostic,
};
use stave_lang::placement::{Direction, Face};
use stave_model::solved::PartOrigin;

/// A 100 x 50 board (19 thick) with the given placement terms.
fn block(place: &str) -> String {
    format!(
        "part block : pine_19 {{\n  length: 100mm\n  width:  50mm\n  place {{\n{place}\n  }}\n}}"
    )
}

/// A side standing at the left end of the base, 400 tall.
const SIDE: &str = "part side : pine_19 {
  length: 400mm
  width:  300mm
  place {
    orient(length: up, width: back)
    against(base.top)
    flush(base.left)
    flush(base.front)
  }
}";

/// The whole base as a region, from its bottom to its top.
const SPACE: &str = "region space = opening(left: base.left, right: base.right, bottom: base.bottom, top: base.top, front: base.front, back: base.back)";

// ---------------------------------------------------------------------------
// Terms
// ---------------------------------------------------------------------------

#[test]
fn without_orient_a_part_lies_flat() {
    let opened = open(&block(
        "    against(base.top)\n    flush(base.left)\n    flush(base.front)",
    ));
    let orientation = part(design(&opened), "block").pose.orientation;
    assert_eq!(
        (
            orientation.length(),
            orientation.width(),
            orientation.thickness()
        ),
        (Direction::Right, Direction::Back, Direction::Up)
    );
}

#[test]
fn against_puts_the_opposite_face_on_the_anchor() {
    // Under the base: the block's top face on the base's bottom face.
    let opened = open(&block(
        "    against(base.bottom)\n    flush(base.left)\n    flush(base.front)",
    ));
    assert_eq!(
        extent(part(design(&opened), "block"))[1],
        (mm("-19"), mm("0"))
    );
}

#[test]
fn against_with_gap_is_measured_outward_from_the_anchor_face() {
    let opened = open(&block(
        "    against(base.bottom, gap: 5mm)\n    flush(base.left)\n    against(base.back, gap: 2mm)",
    ));
    let block_extent = extent(part(design(&opened), "block"));
    assert_eq!(block_extent[1], (mm("-24"), mm("-5")), "below the bottom");
    assert_eq!(block_extent[2], (mm("302"), mm("352")), "behind the back");
}

#[test]
fn flush_with_inset_moves_into_the_anchors_part() {
    let opened = open(&block(
        "    flush(base.top, inset: 4mm)\n    flush(base.left, inset: 10mm)\n    flush(base.back, inset: 20mm)",
    ));
    assert_eq!(
        extent(part(solved(&opened), "block")),
        ranges([("10", "110"), ("-4", "15"), ("230", "280")])
    );
}

#[test]
fn against_and_flush_take_a_face_not_a_mid_plane() {
    only_error(
        &open(&block(
            "    against(mid(base, up))\n    flush(base.left)\n    flush(base.front)",
        )),
        "type-mismatch",
        "'against' needs a face, found a mid-plane",
    );
    only_error(
        &open(&block(
            "    against(base.top)\n    flush(mid(base, across))\n    flush(base.front)",
        )),
        "type-mismatch",
        "'flush' needs a face, found a mid-plane",
    );
}

#[test]
fn center_on_a_region_along_a_world_axis() {
    let opened = open(&format!(
        "{SPACE}\n{}",
        block("    against(base.top)\n    center(space, across)\n    center(space, deep)")
    ));
    assert_eq!(
        extent(part(design(&opened), "block")),
        ranges([("250", "350"), ("19", "38"), ("125", "175")])
    );
}

#[test]
fn center_between_needs_two_parallel_anchors() {
    only_error(
        &open(&block(
            "    against(base.top)\n    center(from: base.left, to: base.front)\n    flush(base.front)",
        )),
        "type-mismatch",
        "'center' needs two parallel planes, found one across 'across' and one across 'deep'",
    );
}

#[test]
fn spaced_ends_puts_the_first_and_last_element_flush_with_the_anchors() {
    let opened = open(
        "part peg[3] : pine_12 {\n  length: 20mm\n  width:  20mm\n  place {\n    against(base.top)\n    spaced(ends, from: base.left, to: base.right)\n    flush(base.front)\n  }\n}",
    );
    let design = design(&opened);
    assert_eq!(extent(part(design, "peg[0]"))[0], (mm("0"), mm("20")));
    assert_eq!(extent(part(design, "peg[1]"))[0], (mm("290"), mm("310")));
    assert_eq!(extent(part(design, "peg[2]"))[0], (mm("580"), mm("600")));
}

#[test]
fn spaced_runs_from_a_mid_plane_too() {
    let opened = open(
        "part peg[2] : pine_12 {\n  length: 20mm\n  width:  20mm\n  place {\n    against(base.top)\n    spaced(ends, from: mid(base, across), to: base.right)\n    flush(base.front)\n  }\n}",
    );
    assert_eq!(
        extent(part(design(&opened), "peg[0]"))[0],
        (mm("300"), mm("320"))
    );
}

#[test]
fn spaced_needs_an_array_and_two_parallel_anchors() {
    only_error(
        &open(&block(
            "    against(base.top)\n    spaced(evenly, from: base.left, to: base.right)\n    flush(base.front)",
        )),
        "type-mismatch",
        "'spaced' needs an array, but 'block' is a single item",
    );
    only_error(
        &open(
            "part peg[2] : pine_12 {\n  length: 20mm\n  width:  20mm\n  place {\n    against(base.top)\n    spaced(ends, from: base.left, to: base.back)\n    flush(base.front)\n  }\n}",
        ),
        "type-mismatch",
        "'spaced' needs two parallel planes",
    );
}

#[test]
fn corners_needs_the_four_corner_labels() {
    only_error(
        &open(&block(
            "    corners(base, inset: 0mm)\n    against(base.top)",
        )),
        "type-mismatch",
        "'corners' needs the labels front_left, front_right, back_left and back_right, found 'block'",
    );
}

#[test]
fn orient_names_where_the_length_and_width_run() {
    let opened = open(&block(
        "    orient(length: up, width: front)\n    against(base.top)\n    flush(base.left)\n    flush(base.front)",
    ));
    let block_part = part(design(&opened), "block");
    assert_eq!(block_part.pose.orientation.thickness(), Direction::Right);
    assert_eq!(
        extent(block_part),
        ranges([("0", "19"), ("19", "119"), ("0", "50")])
    );
}

#[test]
fn orient_is_written_once() {
    only_error(
        &open(&block(
            "    orient(length: right, width: back)\n    orient(length: right, width: back)\n    against(base.top)\n    flush(base.left)\n    flush(base.front)",
        )),
        "axis-set-twice",
        "block: 'orient' is written twice",
    );
}

#[test]
fn orient_needs_a_width_unless_the_part_is_round() {
    let opened = open(&block(
        "    orient(length: up)\n    against(base.top)\n    flush(base.left)\n    flush(base.front)",
    ));
    the_diagnostic(&opened, "missing-field", "'orient' is missing 'width'");
    let dowel = |length: &str| {
        open(&format!(
            "stock d {{ species: beech, form: dowel, diameter: 10mm }}\npart rod : d {{\n  length: 100mm\n  place {{\n    orient(length: {length})\n    against(base.top)\n    flush(base.left)\n    flush(base.front)\n  }}\n}}"
        ))
    };
    let opened = dowel("up");
    assert_eq!(
        part(design(&opened), "rod").pose.orientation.width(),
        Direction::Right
    );
    let opened = dowel("left");
    assert_eq!(
        part(design(&opened), "rod").pose.orientation.width(),
        Direction::Back
    );
}

#[test]
fn the_ground_part_may_be_turned_with_orient() {
    let opened = common::open_files(&[(
        "project.stave",
        "stave 1\n\nproject \"T\" {\n  units: mm\n}\n\nstock s { species: pine, form: solid, thickness: 19mm }\n\npart post : s {\n  length: 700mm\n  width:  40mm\n  place {\n    ground\n    orient(length: up, width: right)\n  }\n}\n",
    )]);
    assert_eq!(
        extent(part(design(&opened), "post")),
        ranges([("0", "40"), ("0", "700"), ("0", "19")])
    );
}

#[test]
fn a_placement_term_is_one_of_a_fixed_vocabulary() {
    let opened = open(&block(
        "    on(base.top)\n    flush(base.left)\n    flush(base.front)",
    ));
    only_error(&opened, "unknown-name", "unknown name 'on'");
    assert_eq!(
        common::help(&opened),
        "a placement term is one of ground, against, flush, center, spaced, corners, orient"
    );
}

#[test]
fn an_anchor_is_a_face_of_a_part_or_region() {
    only_error(
        &open(&block(
            "    against(base)\n    flush(base.left)\n    flush(base.front)",
        )),
        "type-mismatch",
        "'face' needs a face or mid-plane, found a part",
    );
}

// ---------------------------------------------------------------------------
// Rules
// ---------------------------------------------------------------------------

#[test]
fn ground_is_never_inside_a_component() {
    the_diagnostic(
        &open(
            "component C {\n  part p : pine_19 {\n    length: 100mm\n    width:  50mm\n    place {\n      ground\n    }\n  }\n}\nassembly c : C { }",
        ),
        "ground",
        "'c.p' is placed with 'ground' inside a component",
    );
}

#[test]
fn parts_are_placed_in_dependency_order_not_text_order() {
    let opened = open(&format!(
        "{}\n{SIDE}",
        block("    against(side.top)\n    flush(side.left)\n    flush(side.front)")
    ));
    assert_eq!(
        extent(part(design(&opened), "block"))[1],
        (mm("419"), mm("438"))
    );
}

#[test]
fn a_longer_placement_cycle_lists_every_part() {
    let chain = |name: &str, on: &str| {
        format!(
            "part {name} : pine_19 {{\n  length: 100mm\n  width:  100mm\n  place {{\n    against({on}.top)\n    flush(base.left)\n    flush(base.front)\n  }}\n}}"
        )
    };
    let opened = open(&format!(
        "{}\n{}\n{}",
        chain("a", "c"),
        chain("b", "a"),
        chain("c", "b")
    ));
    let [only] = opened.diagnostics.as_slice() else {
        panic!("{:?}", diagnostics(&opened));
    };
    assert_eq!(only.code.as_str(), "placement-cycle");
    assert_eq!(only.notes.len(), 1, "{only:?}");
    assert!(only.notes[0].starts_with("the cycle is "), "{only:?}");
}

#[test]
fn joints_never_position_parts() {
    // The shelf stops 20mm short of the side; the dado doesn't pull it in.
    let opened = open(&format!(
        "{SIDE}\n{}\njoint housing : dado(block, into: side)",
        block(
            "    against(base.top, gap: 100mm)\n    against(side.right, gap: 20mm)\n    flush(base.front)"
        )
    ));
    let design = solved(&opened);
    assert_eq!(extent(part(design, "block"))[0], (mm("39"), mm("139")));
    let block_id = part(design, "block").id;
    assert!(
        common::joint(design, "housing")
            .unmatched
            .contains(&block_id)
    );
}

// ---------------------------------------------------------------------------
// Hardware placement
// ---------------------------------------------------------------------------

/// A drawer front standing on the base at its front.
const FRONT: &str = "part front : pine_19 {
  length: 300mm
  width:  150mm
  place {
    orient(length: right, width: up)
    against(base.top)
    flush(base.left)
    flush(base.front)
  }
}";

fn knob(face: &str, place: &str) -> String {
    format!("{FRONT}\nhardware handle : knob(front{face}) {{\n  place {{\n{place}\n  }}\n}}")
}

#[test]
fn a_pull_is_mounted_on_the_front_face_of_its_piece_by_default() {
    let opened = open(&knob(
        "",
        "    center(front, across)\n    center(front, up)",
    ));
    let handle = hardware(design(&opened), "handle");
    assert_eq!(handle.mount_face, Face::Front);
    assert_eq!(
        bounds(&handle.bounds),
        ranges([("135", "165"), ("79", "109"), ("-28", "0")])
    );
}

#[test]
fn face_names_another_mount_face() {
    let opened = open(&knob(
        ", face: top",
        "    center(front, across)\n    center(front, deep)",
    ));
    let handle = hardware(design(&opened), "handle");
    assert_eq!(handle.mount_face, Face::Top);
    assert_eq!(
        (handle.bounds.min().up, handle.bounds.max().up),
        (mm("169"), mm("197"))
    );
    let opened = open(&knob(
        ", face: up",
        "    center(front, across)\n    center(front, deep)",
    ));
    only_error(&opened, "unknown-name", "unknown name 'up'");
}

#[test]
fn the_mount_face_fixes_the_axis_it_is_normal_to() {
    only_error(
        &open(&knob(
            "",
            "    center(front, across)\n    center(front, up)\n    center(front, deep)",
        )),
        "axis-set-twice",
        "handle: position along 'deep' is set twice",
    );
    only_error(
        &open(&knob("", "    center(front, across)")),
        "axis-not-set",
        "handle: position along 'up' is not set",
    );
}

#[test]
fn orient_ground_and_corners_do_not_apply_to_hardware() {
    only_error(
        &open(&knob(
            "",
            "    orient(length: up, width: right)\n    center(front, across)\n    center(front, up)",
        )),
        "type-mismatch",
        "'orient' doesn't apply to hardware",
    );
    only_error(
        &open(&knob("", "    ground\n    center(front, across)")),
        "type-mismatch",
        "'ground' doesn't apply to hardware",
    );
}

/// A door hanging on the side's right face, flush with the base's front.
const DOOR: &str = "part door : pine_19 {
  length: 300mm
  width:  200mm
  place {
    orient(length: up, width: right)
    against(base.top)
    against(side.right)
    against(base.front)
  }
}";

#[test]
fn a_hinge_is_placed_along_its_pieces_length_only() {
    let hinge = |place: &str| {
        open(&format!(
            "{SIDE}\n{DOOR}\nhardware h : euro_hinge(door, on: side){place}"
        ))
    };
    let opened = hinge(" {\n  place {\n    flush(door.bottom, inset: 50mm)\n  }\n}");
    let h = hardware(design(&opened), "h");
    assert_eq!(h.mount_face, Face::Back);
    assert_eq!(h.bounds.min().up, mm("69"));
    only_error(
        &hinge(""),
        "axis-not-set",
        "h: position along 'up' is not set",
    );
    only_error(
        &hinge(
            " {\n  place {\n    flush(door.bottom, inset: 50mm)\n    center(door, across)\n  }\n}",
        ),
        "axis-set-twice",
        "h: position along 'across' is set twice",
    );
}

#[test]
fn a_slide_is_fixed_entirely_by_its_base() {
    let opened = open(&format!(
        "{SIDE}\npart drawer_side : pine_12 {{\n  length: 300mm\n  width:  100mm\n  place {{\n    orient(length: back, width: up)\n    against(base.top)\n    against(side.right, gap: 12.7mm)\n    flush(base.front)\n  }}\n}}\nregion bay = opening(left: side.right, right: base.right, bottom: base.top, top: side.top, front: base.front, back: base.back)\nhardware runner : side_mount_slide(drawer_side, in: bay, length: 300mm) {{\n  place {{\n    flush(drawer_side.bottom)\n  }}\n}}"
    ));
    only_error(
        &opened,
        "axis-set-twice",
        "runner: position along 'up' is set twice",
    );
}

#[test]
fn a_slide_mounts_on_the_side_nearer_its_openings_side() {
    let opened = open(&format!(
        "{SIDE}\npart drawer_side : pine_12 {{\n  length: 300mm\n  width:  100mm\n  place {{\n    orient(length: back, width: up)\n    against(base.top)\n    against(side.right, gap: 12.7mm)\n    flush(base.front)\n  }}\n}}\nregion bay = opening(left: side.right, right: base.right, bottom: base.top, top: side.top, front: base.front, back: base.back)\nhardware runner : side_mount_slide(drawer_side, in: bay, length: 300mm)"
    ));
    let runner = hardware(design(&opened), "runner");
    assert_eq!(runner.mount_face, Face::Left);
    // Front ends level, centred on the drawer side's width (19..119).
    assert_eq!(
        (runner.bounds.min().deep, runner.bounds.max().deep),
        (mm("0"), mm("300"))
    );
    assert_eq!(
        (runner.bounds.min().up, runner.bounds.max().up),
        (mm("46.5"), mm("91.5"))
    );
}

// ---------------------------------------------------------------------------
// Mirroring
// ---------------------------------------------------------------------------

#[test]
fn a_mirrored_part_has_its_sources_stock_sizes_and_the_reflected_position() {
    let opened = open(&format!(
        "{SIDE}\npart side_r = mirror(side, about: mid(base, across))"
    ));
    let design = design(&opened);
    let side = part(design, "side");
    let mirrored = part(design, "side_r");
    assert_eq!(mirrored.origin, PartOrigin::Mirror { source: side.id });
    assert_eq!(mirrored.stock.name, "pine_19");
    assert_eq!(mirrored.drawn, side.drawn);
    assert_eq!(extent(mirrored)[0], (mm("581"), mm("600")));
    // Its frame is the reflection: its thickness now runs left.
    assert_eq!(side.pose.orientation.thickness(), Direction::Right);
    assert_eq!(mirrored.pose.orientation.thickness(), Direction::Left);
}

#[test]
fn a_mirror_is_about_a_plane() {
    let opened = open(&format!("{SIDE}\npart side_r = mirror(side, about: base)"));
    let found = diagnostics(&opened);
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(found[0].starts_with("type-mismatch: "), "{found:?}");
}

// ---------------------------------------------------------------------------
// Sizes and fit
// ---------------------------------------------------------------------------

#[test]
fn fit_against_a_face_reaches_to_the_nearest_joined_part_beyond_it() {
    let opened = open(&format!(
        "{SIDE}\n{}\npart stop : pine_19 {{\n  length: 300mm\n  width:  200mm\n  place {{\n    orient(length: back, width: up)\n    against(base.top)\n    flush(base.left, inset: 400mm)\n    flush(base.front)\n  }}\n}}\njoint housings : dado(block, into: [side, stop], depth: 5mm)",
        block("    against(base.top, gap: 100mm)\n    against(side.right)\n    flush(base.front)")
            .replace("length: 100mm", "length: fit")
    ));
    let block_part = part(design(&opened), "block");
    assert_eq!(
        block_part.drawn.length,
        mm("381"),
        "from 19 to the stop at 400"
    );
    assert_eq!(block_part.cut.length, mm("391"));
}

#[test]
fn fit_against_says_which_side_has_no_joined_part() {
    let opened = open(&format!(
        "{SIDE}\n{}\njoint housing : dado(block, into: side)",
        block("    against(base.top, gap: 100mm)\n    flush(base.left, inset: 100mm)\n    flush(base.front)").replace("length: 100mm", "length: fit")
    ));
    // Its left end is known (100); the side it is joined into lies to the
    // left of that, and nothing lies to the right.
    let diagnostic = the_diagnostic(
        &opened,
        "fit-undecided",
        "block: nothing decides its length",
    );
    assert_eq!(
        diagnostic.notes,
        ["no part it is joined into is to the right of it"]
    );
}

#[test]
fn a_spaced_parts_size_must_be_written() {
    let opened = open(&format!(
        "{SIDE}\npart slat[2] : pine_12 {{\n  length: fit\n  width:  20mm\n  place {{\n    against(base.top)\n    spaced(evenly, from: base.left, to: base.right)\n    flush(base.front)\n  }}\n}}\njoint housing : dado(slat[*], into: side)"
    ));
    let found = diagnostics(&opened);
    assert!(
        found.iter().all(|d| d.starts_with("fit-undecided: slat[")),
        "{found:?}"
    );
}

#[test]
fn between_measures_the_clear_distance() {
    holds(&format!(
        "{SIDE}\nrequire r : between(side, base) == 0mm && between(side.right, base.right) == 581mm \"between\""
    ));
}

#[test]
fn extent_is_the_overall_drawn_size_of_a_list_along_an_axis() {
    holds(&format!(
        "{SIDE}\nrequire r : extent([side, base], up) == 419mm && extent([side], across) == 19mm \"extent\""
    ));
}

// ---------------------------------------------------------------------------
// Regions
// ---------------------------------------------------------------------------

#[test]
fn opening_needs_all_six_sides_on_their_own_axes() {
    the_diagnostic(
        &open(
            "region r = opening(left: base.left, right: base.right, bottom: base.top, top: base.top, front: base.front)",
        ),
        "missing-field",
        "'opening' is missing 'back'",
    );
    only_error(
        &open(
            "region r = opening(left: base.top, right: base.right, bottom: base.top, top: base.top, front: base.front, back: base.back)",
        ),
        "type-mismatch",
        "'left' needs a plane across 'across', found one across 'up'",
    );
}

#[test]
fn a_region_is_always_a_region_expression() {
    only_error(
        &open("region r = 3mm"),
        "type-mismatch",
        "'r' needs a region, found a length",
    );
}

#[test]
fn a_region_reads_its_faces_and_sizes() {
    holds(&format!(
        "{SPACE}\nrequire r : space.across == 600mm && space.up == 19mm && space.deep == 300mm && between(space.top, base.top) == 0mm \"region\""
    ));
}

#[test]
fn split_without_at_needs_an_array_to_hold_its_pieces() {
    only_error(
        &open(&format!("{SPACE}\nregion s = split(space, up)")),
        "type-mismatch",
        "'s' needs a region, found regions",
    );
}

#[test]
fn split_at_a_part_makes_two_regions() {
    let opened = open(&format!(
        "{}\nregion r = opening(left: base.left, right: base.right, bottom: base.top, top: block.top, front: base.front, back: base.back)\nregion s[a, b, c] = split(r, up, at: block)",
        block("    against(base.top, gap: 100mm)\n    flush(base.left)\n    flush(base.front)")
    ));
    only_error(&opened, "type-mismatch", "'s' needs 3 regions, found 2");
}

#[test]
fn split_labels_its_pieces_along_the_axis() {
    let opened = open(&format!(
        "{SPACE}\nregion across_pieces[l, m, r] = split(space, across)\nregion deep_pieces[f, b] = split(space, deep)\nregion up_pieces[low, high] = split(space, up)"
    ));
    let design = design(&opened);
    assert_eq!(
        region(design, "across_pieces[l]").bounds.max().across,
        mm("200")
    );
    assert_eq!(
        region(design, "across_pieces[r]").bounds.min().across,
        mm("400")
    );
    assert_eq!(
        region(design, "deep_pieces[f]").bounds.max().deep,
        mm("150")
    );
    assert_eq!(region(design, "up_pieces[low]").bounds.max().up, mm("9.5"));
}

#[test]
fn a_mount_is_overlay_or_inset() {
    only_error(
        &open(&format!(
            "{SPACE}\nregion c = cover(space, 3mm, thickness: 19mm)"
        )),
        "type-mismatch",
        "'mount' needs a mount, found a length",
    );
}

#[test]
fn an_overlay_front_lies_in_front_of_the_opening() {
    let opened = open(&format!(
        "{SPACE}\nregion flat = cover(space, overlay(), thickness: 19mm)\nregion grown = cover(space, overlay(left: -2mm, right: 3mm, top: 1mm, bottom: -1mm), thickness: 19mm)"
    ));
    let design = design(&opened);
    // Every edge defaults to 0mm; its back face is on the opening's front.
    assert_eq!(
        bounds(&region(design, "flat").bounds),
        ranges([("0", "600"), ("0", "19"), ("-19", "0")])
    );
    // A negative overlay is a reveal.
    assert_eq!(
        bounds(&region(design, "grown").bounds),
        ranges([("2", "603"), ("1", "20"), ("-19", "0")])
    );
}

#[test]
fn meeting_applies_only_between_pieces_of_a_split_with_nothing_between_them() {
    let opened = open(&format!(
        "{SPACE}\nregion whole = cover(space, overlay(left: 5mm, right: 5mm, meeting: -1mm), thickness: 19mm)\npart divider : pine_19 {{\n  length: 19mm\n  width:  300mm\n  place {{\n    orient(length: up, width: back)\n    against(base.top)\n    center(base, across)\n    flush(base.front)\n  }}\n}}\nregion bay = opening(left: base.left, right: base.right, bottom: base.top, top: divider.top, front: base.front, back: base.back)\nregion half[l, r] = split(bay, across, at: divider)\nregion door = cover(half[l], overlay(left: 5mm, right: 5mm, meeting: -1mm), thickness: 19mm)"
    ));
    let design = design(&opened);
    // No neighbour: both edges take their own value.
    assert_eq!(region(design, "whole").bounds.min().across, mm("-5"));
    assert_eq!(region(design, "whole").bounds.max().across, mm("605"));
    // A divider between the halves: the right edge keeps its own 5mm.
    assert_eq!(region(design, "door").bounds.max().across, mm("295.5"));
}

#[test]
fn an_inset_front_is_flush_with_the_openings_front_with_an_even_gap() {
    let opened = open(&format!(
        "{SPACE}\nregion inside = cover(space, inset(reveal: 2mm), thickness: 10mm)"
    ));
    assert_eq!(
        bounds(&region(design(&opened), "inside").bounds),
        ranges([("2", "598"), ("2", "17"), ("0", "10")])
    );
}
