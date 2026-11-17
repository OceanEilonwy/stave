//! Executable spec: `contact(a, b)` ("Built-in functions") and what a
//! failed kind rule says ("Checks and `require`"), with the rendered
//! diagnostics of every broken design the standard kinds' contact and
//! size rules catch (docs/reviews/2026-11-17-checks-gaps.md, stage 1).

mod common;

use std::path::Path;

use common::{holds, only_error, open};
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

/// A side standing on the base's left end and a shelf butting against it,
/// 100mm up: the shelf's 300 x 19 end meets the side's face.
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

#[test]
fn a_patch_has_its_longer_side_as_length_and_its_shorter_as_width() {
    holds(&format!(
        "{SIDE_AND_SHELF}
require long : contact(shelf, side).length == 300mm \"along the end\"
require wide : contact(shelf, side).width == 19mm \"across the end\"
require same : contact(side, shelf).width == contact(shelf, side).width \"either way round\""
    ));
}

#[test]
fn a_patch_is_only_where_the_parts_overlap() {
    // The side stands on the base's left end: they share the side's 19mm
    // by 300mm end. A 200mm wide block 290mm back meets only 10mm of the
    // side's 300mm width, over its own 19mm thickness.
    holds(&format!(
        "{SIDE_AND_SHELF}
part block : pine_19 {{
  length: 100mm
  width:  200mm
  place {{
    against(side.right)
    flush(side.top)
    flush(base.front, inset: 290mm)
  }}
}}
require on_base : contact(side, base).width == 19mm \"the side's end\"
require partly  : contact(block, side).width == 10mm \"only where they overlap\"
require across  : contact(block, side).length == 19mm \"the block's thickness\""
    ));
}

#[test]
fn extent_of_a_patch_measures_square_to_it_and_along_it() {
    // Square to where the shelf meets the side is the side's thickness;
    // along the patch, the side's width.
    holds(&format!(
        "{SIDE_AND_SHELF}
require behind : extent(side, thickness, of: contact(shelf, side)) == 19mm \"the side behind the shelf\"
require along  : extent(side, length, of: contact(shelf, side)) == 300mm \"the side along the patch\""
    ));
}

#[test]
fn a_turned_part_touches_with_the_diameter_at_its_end() {
    // The leg's box is 40mm, its widest diameter; its top end is 34mm.
    holds(
        "stock ash_45_sq { species: ash, form: solid, thickness: 45mm, width: 45mm }
profile leg_shape {
  measure: diameter
  at 0mm:   34mm
  at 100mm: 40mm
  at 200mm: 30mm
}
part leg : ash_45_sq {
  length: fit
  shape:  turned(leg_shape)
  place {
    orient(length: down)
    against(base.bottom)
    flush(base.left, inset: 50mm)
    flush(base.front, inset: 50mm)
  }
}
require top_end : contact(leg, base).width == 34mm \"the top of the leg\"
require square  : contact(leg, base).length == 34mm \"a circle's square\"",
    );
}

#[test]
fn parts_that_dont_touch_have_no_contact() {
    let opened = open(&format!(
        "{SIDE_AND_SHELF}
require r : contact(shelf, base).width >= 1mm \"apart\""
    ));
    only_error(&opened, "no-contact", "shelf and base don't touch");
}

#[test]
fn contact_needs_one_part_on_each_side() {
    let opened = open(&format!(
        "{SIDE_AND_SHELF}
require r : contact([shelf, side], base).width >= 1mm \"two\""
    ));
    only_error(
        &opened,
        "type-mismatch",
        "'contact' needs one part, found parts",
    );
    let opened = open(&format!(
        "{SIDE_AND_SHELF}
require r : contact(shelf, 19mm).width >= 1mm \"a length\""
    ));
    only_error(
        &opened,
        "type-mismatch",
        "'contact' needs a part, found a length",
    );
}

#[test]
fn a_patch_has_only_a_length_and_a_width() {
    let opened = open(&format!(
        "{SIDE_AND_SHELF}
require r : contact(shelf, side).thickness >= 1mm \"flat\""
    ));
    only_error(
        &opened,
        "unknown-field",
        "a contact patch has no field 'thickness'",
    );
}

#[test]
fn a_kind_rule_reads_the_patch_for_each_pair() {
    // A home-made housing that needs the shelf's end to sit wholly on the
    // side: it holds for the shelf, and fails where the short block
    // overhangs, with what it measured and the overlap in the help.
    let kind = "kind seat : joint {
  require whole : contact(piece, into).width >= piece.thickness \"the end must sit wholly on the other part\"
  insert: [push(piece, along: length)]
}";
    let opened = open(&format!(
        "{SIDE_AND_SHELF}
part block : pine_19 {{
  length: 100mm
  width:  200mm
  place {{
    against(side.right)
    flush(side.top)
    flush(base.front, inset: 290mm)
  }}
}}
{kind}
joint seats : seat([shelf, block], into: side)"
    ));
    let diagnostic = common::the_diagnostic(
        &opened,
        "require",
        "'whole' fails: the end must sit wholly on the other part",
    );
    assert_eq!(
        diagnostic.notes,
        [
            "for block and side in seats",
            "the rule measures 10mm and needs at least 19mm",
        ]
    );
    assert_eq!(
        diagnostic.help.as_deref(),
        Some(
            "side covers only 10mm of block's 200mm end; move or resize side so it covers the whole end"
        )
    );
}

// ---------------------------------------------------------------------------
// The broken designs the standard kinds' contact and size rules catch
// ---------------------------------------------------------------------------

#[test]
fn dowels_on_a_top_that_covers_too_little_of_the_sides() {
    insta::assert_snapshot!(rendered("tests/broken/dowels_overlap_too_little"));
}

#[test]
fn too_many_dowels_along_a_kick() {
    insta::assert_snapshot!(rendered("tests/broken/too_many_dowels"));
}

#[test]
fn screws_too_thick_for_the_edge_they_go_into() {
    insta::assert_snapshot!(rendered("tests/broken/screw_too_thick"));
}

#[test]
fn too_many_screws_along_an_arm() {
    insta::assert_snapshot!(rendered("tests/broken/too_many_screws"));
}

#[test]
fn a_tenon_thicker_than_its_rail() {
    insta::assert_snapshot!(rendered("tests/broken/tenon_too_thick"));
}

#[test]
fn a_round_tenon_wider_than_its_foot() {
    insta::assert_snapshot!(rendered("tests/broken/round_tenon_too_thick"));
}

#[test]
fn a_round_tenon_measured_at_the_slim_end_of_a_turned_leg() {
    insta::assert_snapshot!(rendered("tests/broken/round_tenon_turned_end"));
}

#[test]
fn a_groove_as_deep_as_the_wall() {
    insta::assert_snapshot!(rendered("tests/broken/groove_too_deep"));
}

#[test]
fn a_door_too_thin_for_its_hinge_cups() {
    insta::assert_snapshot!(rendered("tests/broken/door_too_thin_for_cup"));
}

#[test]
fn contact_between_parts_that_dont_touch() {
    insta::assert_snapshot!(rendered("tests/broken/contact_apart"));
}
