//! The root box and shares, end to end (grammar, "Placement rules" rule 1
//! and "Regions"): `box(across:, up:, deep:)` makes the root region from
//! sizes alone, its bottom face is the floor, parts place against its
//! faces and may lie outside it; `split(..., shares: [...])` sizes the
//! pieces in proportion. The hardware envelope `box(length:, width:,
//! thickness:)` is told apart by its argument names.

mod common;

use std::path::Path;

use common::{check, drawn, minus, mm, region};
use stave_lang::Code;
use stave_lang::units::Length;

fn worked_example() -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/designs/box_root_shelf/project.stave");
    std::fs::read_to_string(path).unwrap()
}

/// A root box with one part standing on its floor; `{more}` adds items.
fn with_box(more: &str) -> String {
    format!(
        r#"stave 1

project "A box" {{
  units: mm
}}

param overall_width  = 600mm
param overall_height = 400mm
param overall_depth  = 300mm

region envelope = box(across: overall_width, up: overall_height, deep: overall_depth)

stock pine_19 {{ species: pine, form: solid, thickness: 19mm, max_width: 300mm, max_length: 2400mm }}

part base : pine_19 {{
  length: overall_width
  width:  overall_depth
  place {{
    flush(envelope.bottom)
    flush(envelope.left)
    flush(envelope.front)
  }}
}}
{more}
"#
    )
}

#[test]
fn the_worked_example_checks_clean_with_its_parts_against_the_box() {
    let checked = check(&worked_example());
    let design = checked.clean();
    let root = design.root_box().expect("the root is a box");
    assert_eq!(root.name, "envelope");
    assert_eq!(root.bounds.min().up, Length::ZERO);
    assert_eq!(root.bounds.max().across, mm(600));
    assert_eq!(root.bounds.max().up, mm(450));
    assert_eq!(root.bounds.max().deep, mm(250));
    // The floor is the box's bottom face, and the bottom stands on it.
    assert_eq!(design.floor(), Some(Length::ZERO));
    assert_eq!(drawn(design, "bottom").min(), root.bounds.min());
    // The sides stand on the bottom, at the box's left and right faces.
    assert_eq!(drawn(design, "side_l").min().across, Length::ZERO);
    assert_eq!(drawn(design, "side_r").max().across, mm(600));
    assert_eq!(drawn(design, "side_l").max().up, mm(450));
    // The lower bay is two thirds of the 431mm between the bottom and the
    // top of the box; the shelf stands on it.
    let lower = region(design, "bay[lower]");
    let upper = region(design, "bay[upper]");
    assert_eq!(lower.min().up, mm(19));
    let lower_top = 19_000_000 + 431_000_000 * 2 / 3;
    assert!((lower.max().up.nm() - lower_top).abs() <= 1, "{lower:?}");
    assert_eq!(upper.min().up, lower.max().up);
    assert_eq!(upper.max().up, mm(450));
    assert_eq!(drawn(design, "shelf").min().up, lower.max().up);
}

#[test]
fn the_floor_is_the_bottom_of_the_box_even_when_nothing_stands_on_it() {
    // The base is lifted 50mm off the bottom of the box, and kept as a part
    // meant to lift out so it is not loose.
    let text = with_box("").replace(
        "    flush(envelope.bottom)\n",
        "    flush(envelope.bottom, inset: 50mm)\n",
    );
    let text = text.replace(
        "  width:  overall_depth\n",
        "  width:  overall_depth\n  loose:  true\n",
    );
    let checked = check(&text);
    let design = checked.clean();
    assert_eq!(design.floor(), Some(Length::ZERO));
    assert_eq!(drawn(design, "base").min().up, mm(50));
}

#[test]
fn parts_may_lie_outside_the_box() {
    // A top standing on the box, wider than it on both sides, and screwed
    // to a post that stands on the base.
    let checked = check(&with_box(
        r#"
part post : pine_19 {
  length: overall_height - pine_19.thickness
  width:  50mm
  place {
    orient(length: up, width: back)
    against(base.top)
    center(envelope, across)
    center(envelope, deep)
  }
}

part top : pine_19 {
  length: overall_width + 100mm
  width:  overall_depth
  place {
    against(envelope.top)
    center(envelope, across)
    flush(envelope.front)
  }
}

joint post_to_base : screw(base, into: post, length: 40mm, count: 1)
joint top_to_post  : screw(top, into: post, length: 40mm, count: 1)
"#,
    ));
    let design = checked.clean();
    let top = drawn(design, "top");
    assert_eq!(top.min().up, mm(400));
    assert_eq!(top.min().across, Length::from_nm(-50_000_000));
    assert_eq!(top.max().across, mm(650));
    // Centred on the box's mid-plane.
    assert_eq!(
        drawn(design, "post").min().across,
        Length::from_nm(290_500_000)
    );
}

#[test]
fn every_size_of_the_box_is_more_than_zero() {
    for (from, to, message) in [
        (
            "across: overall_width",
            "across: -10mm",
            "'across' must be more than 0mm, found -10mm",
        ),
        (
            "deep: overall_depth",
            "deep: overall_depth - 300mm",
            "'deep' must be more than 0mm, found 0mm",
        ),
    ] {
        let checked = check(&with_box("").replace(from, to));
        assert_eq!(checked.messages(), [message], "{to}");
        assert_eq!(checked.diagnostics[0].code, Code::OutOfRange);
        assert!(checked.diagnostics[0].help.is_some());
    }
}

#[test]
fn a_box_needs_all_three_sizes() {
    let checked = check(&with_box("").replace(", deep: overall_depth", ""));
    assert_eq!(checked.messages(), ["'box' is missing 'deep'"]);
    assert_eq!(checked.diagnostics[0].code, Code::MissingField);
}

#[test]
fn the_argument_names_tell_the_region_from_the_hardware_envelope() {
    // `length:`, `width:` and `thickness:` make a hardware envelope, which
    // a region can't hold.
    let checked = check(&with_box(
        "region other = box(length: 10mm, width: 10mm, thickness: 10mm)\n",
    ));
    assert_eq!(
        checked.messages(),
        ["'other' needs a region, found an envelope"]
    );
    // A name of the other form is an unknown field, with the region's.
    let checked = check(&with_box("").replace("up: overall_height", "width: overall_height"));
    let mut messages = checked.messages();
    messages.sort();
    assert_eq!(
        messages,
        ["'box' has no field 'width'", "'box' is missing 'up'"]
    );
}

#[test]
fn a_ground_root_still_works_and_its_floor_is_the_lowest_part() {
    let text = with_box("")
        .replace("region envelope = box(across: overall_width, up: overall_height, deep: overall_depth)\n", "")
        .replace(
            "    flush(envelope.bottom)\n    flush(envelope.left)\n    flush(envelope.front)\n",
            "    ground\n",
        );
    let checked = check(&text);
    let design = checked.clean();
    assert!(design.root_box().is_none());
    assert_eq!(design.floor(), Some(Length::ZERO));
}

#[test]
fn shares_size_the_pieces_in_proportion_in_label_order() {
    let checked = check(&with_box(
        r#"
region bay[bottom, middle, top] = split(envelope, up, shares: [1, 2, 5])
region half[front, back]        = split(envelope, deep, shares: [0.5, 1.5])
region equal[left, right]       = split(envelope, across)
"#,
    ));
    let design = checked.clean();
    // 400mm in the ratio 1 : 2 : 5.
    assert_eq!(region(design, "bay[bottom]").max().up, mm(50));
    assert_eq!(region(design, "bay[middle]").min().up, mm(50));
    assert_eq!(region(design, "bay[middle]").max().up, mm(150));
    assert_eq!(region(design, "bay[top]").max().up, mm(400));
    // Ratios work as well as whole numbers: 300mm as 1 : 3.
    assert_eq!(region(design, "half[front]").max().deep, mm(75));
    // Without shares, equal pieces.
    let left = region(design, "equal[left]");
    assert_eq!(minus(left.max().across, left.min().across), mm(300));
}

#[test]
fn shares_must_be_numbers() {
    let checked = check(&with_box(
        "region bay[a, b] = split(envelope, up, shares: [1, true])\n",
    ));
    assert_eq!(
        checked.messages(),
        ["'shares' needs a number, found true or false"]
    );
    let checked = check(&with_box(
        "region bay[a, b] = split(envelope, up, shares: [10deg, 1])\n",
    ));
    assert_eq!(
        checked.messages(),
        ["'shares' needs a number, found an angle"]
    );
    assert_eq!(checked.diagnostics[0].code, Code::UnitMismatch);
}

#[test]
fn shares_on_a_count_that_cannot_change_are_fine() {
    let checked = check(&with_box(
        r#"
param bays = 3
region bay[bays] = split(envelope, up, shares: [1, 1, 2])
"#,
    ));
    let design = checked.clean();
    assert_eq!(region(design, "bay[2]").min().up, mm(200));
}
