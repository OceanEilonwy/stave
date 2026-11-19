//! Place params end to end (grammar, "Place params"): a component takes a
//! `Part`, `Parts`, `Anchor` or `Region` param, is given it in the
//! assembly body, and places its parts against what it was handed; a
//! mirrored assembly re-binds each one to its counterpart on the other
//! side (grammar, "Mirroring").

mod common;

use common::{check, drawn, minus, mm, pairs, plus, region};
use stave_model::solved::RoleValue;

/// A base, two sides on it, and four components: a cleat given a `Part`
/// and an `Anchor`, a corner block given `Parts` and two `Anchor`s, a
/// shelf given a `Region` and `Parts`, and a runner on a slide given a
/// `Region`, which it hands on to the slide's `in:` role. The cleat, the
/// block and the runner are mirrored.
const PROJECT: &str = r#"
stave 1

project "Place params" {
  units: mm
}

stock pine_19 { species: pine, form: solid, thickness: 19mm, max_width: 300mm, max_length: 2400mm }

part base : pine_19 {
  length: 600mm
  width:  300mm
  place {
    ground
  }
}

part side_l : pine_19 {
  length: 400mm
  width:  300mm
  place {
    orient(length: up, width: back)
    against(base.top)
    flush(base.left)
    flush(base.front)
  }
}

part side_r = mirror(side_l, about: mid(base, across))

joint sides : screw(base, into: [side_l, side_r], length: 40mm)

/// A cleat screwed to the inside of a side.
component Cleat {
  param side   : Part
  param face   : Anchor
  param height = 200mm
  part cleat : pine_19 {
    length: side.width - 20mm
    width:  30mm
    place {
      orient(length: back, width: up)
      against(face)
      flush(side.bottom, inset: height)
      center(side, deep)
    }
  }
  joint fix : screw(cleat, into: side, length: 30mm)
}

assembly cleat_l : Cleat { side: side_l, face: side_l.right }
assembly cleat_r = mirror(cleat_l, about: mid(base, across))

/// A corner block glued and screwed into the parts it is handed.
component Block {
  param corner : Parts
  param wall   : Anchor
  param floor  : Anchor
  part block : pine_19 {
    length: 60mm
    width:  80mm
    place {
      orient(length: back, width: up)
      against(wall)
      against(floor)
      flush(base.front, inset: 100mm)
    }
  }
  joint fix : screw(block, into: corner, length: 30mm)
}

assembly block_l : Block { corner: [side_l, base], wall: side_l.right, floor: base.top }
assembly block_r = mirror(block_l, about: mid(base, across))

/// A shelf filling the space it is handed, screwed through the sides.
component Shelf {
  param space : Region
  param sides : Parts
  part board : pine_19 {
    length: space.across
    width:  space.deep
    place {
      flush(space.bottom)
      flush(space.left)
      flush(space.front)
    }
  }
  joint fix : screw(sides, into: board, length: 40mm)
}

region inside = opening(left: side_l.right, right: side_r.left, bottom: cleat_l.cleat.top,
                        top: side_l.top, front: base.front, back: base.back)
assembly shelf : Shelf { space: inside, sides: [side_l, side_r] }

/// A runner on a slide in the space it is handed, beside the space's left
/// side.
component Runner {
  param space : Region
  part runner : pine_19 {
    length: 250mm
    width:  60mm
    loose:  true
    place {
      orient(length: back, width: up)
      flush(space.left, inset: 12.7mm)
      flush(space.bottom, inset: 110mm)
      flush(space.front)
    }
  }
  hardware slide : side_mount_slide(runner, in: space, length: 250mm)
}

region bay_l = opening(left: side_l.right, right: mid(base, across), bottom: base.top,
                       top: cleat_l.cleat.bottom, front: base.front, back: base.back)
region bay_r = opening(left: mid(base, across), right: side_r.left, bottom: base.top,
                       top: cleat_l.cleat.bottom, front: base.front, back: base.back)
assembly runner_l : Runner { space: bay_l }
assembly runner_r = mirror(runner_l, about: mid(base, across))
"#;

#[test]
fn a_part_param_places_against_the_part_it_was_handed() {
    let checked = check(PROJECT);
    let design = checked.clean();
    let side = drawn(design, "side_l");
    let cleat = drawn(design, "cleat_l.cleat");
    // `against(face)` with face = side_l.right, and 200mm up side_l.
    assert_eq!(cleat.min().across, side.max().across);
    assert_eq!(cleat.min().up, plus(side.min().up, mm(200)));
    // `side.width - 20mm` long, centred on the side's depth.
    assert_eq!(minus(cleat.max().deep, cleat.min().deep), mm(280));
    assert_eq!(cleat.min().deep, plus(side.min().deep, mm(10)));
    assert_eq!(pairs(design, "cleat_l.fix"), [["cleat_l.cleat", "side_l"]]);
}

#[test]
fn a_parts_param_is_a_joints_role() {
    let checked = check(PROJECT);
    let design = checked.clean();
    assert_eq!(
        pairs(design, "block_l.fix"),
        [["block_l.block", "side_l"], ["block_l.block", "base"]]
    );
    assert_eq!(
        pairs(design, "shelf.fix"),
        [["side_l", "shelf.board"], ["side_r", "shelf.board"]]
    );
}

#[test]
fn an_anchor_param_is_a_placement_terms_anchor() {
    let checked = check(PROJECT);
    let design = checked.clean();
    let block = drawn(design, "block_l.block");
    assert_eq!(block.min().across, drawn(design, "side_l").max().across);
    assert_eq!(block.min().up, drawn(design, "base").max().up);
}

#[test]
fn a_region_param_is_the_space_the_component_fills() {
    let checked = check(PROJECT);
    let design = checked.clean();
    let inside = region(design, "inside");
    let board = drawn(design, "shelf.board");
    assert_eq!(board.min(), inside.min());
    assert_eq!(board.max().across, inside.max().across);
    assert_eq!(board.min().up, drawn(design, "cleat_l.cleat").max().up);
}

#[test]
fn mirroring_an_assembly_re_binds_its_place_params() {
    let checked = check(PROJECT);
    let design = checked.clean();
    // Part and Anchor: side_l becomes side_r, side_l.right becomes
    // side_r.left.
    let cleat = drawn(design, "cleat_r.cleat");
    assert_eq!(cleat.max().across, drawn(design, "side_r").min().across);
    assert_eq!(pairs(design, "cleat_r.fix"), [["cleat_r.cleat", "side_r"]]);
    // Parts, part by part in the same order: base lies on the mirror
    // plane, so it is its own counterpart.
    assert_eq!(
        pairs(design, "block_r.fix"),
        [["block_r.block", "side_r"], ["block_r.block", "base"]]
    );
    let block = drawn(design, "block_r.block");
    assert_eq!(block.max().across, drawn(design, "side_r").min().across);
    assert_eq!(block.min().up, drawn(design, "base").max().up);
}

#[test]
fn a_place_param_given_the_wrong_thing_is_a_type_mismatch() {
    let cases = [
        (
            "{ side: side_l, face: side_l.right }",
            "{ side: 300mm, face: side_l.right }",
            "'side' needs a part, found a length",
        ),
        (
            "wall: side_l.right",
            "wall: side_l",
            "'wall' needs a face or mid-plane, found a part",
        ),
        (
            "corner: [side_l, base]",
            "corner: 30mm",
            "'corner' needs parts, found a length",
        ),
        // `against` takes a face, so a mid-plane handed to an Anchor
        // param is wrong where it is used, as anywhere.
        (
            "{ side: side_l, face: side_l.right }",
            "{ side: side_l, face: mid(side_l, across) }",
            "'against' needs a face, found a mid-plane",
        ),
    ];
    for (from, to, message) in cases {
        let checked = check(&PROJECT.replace(from, to));
        assert_eq!(checked.messages(), [message], "{to}");
        assert!(
            checked
                .diagnostics
                .iter()
                .all(|d| d.code == stave_lang::Code::TypeMismatch)
        );
    }
}

#[test]
fn a_place_param_with_no_counterpart_in_the_mirror_is_an_error() {
    // Mirrored up and down about the middle of a side, the block's floor
    // (the base) has nothing in its reflected place.
    let checked = check(&PROJECT.replace(
        "block_r = mirror(block_l, about: mid(base, across))",
        "block_r = mirror(block_l, about: mid(side_l, up))",
    ));
    assert_eq!(
        checked.messages(),
        ["base has no counterpart on the other side of the mirror"]
    );
    assert_eq!(checked.diagnostics[0].code, stave_lang::Code::Mirror);
}

#[test]
fn a_mirrored_region_param_is_the_region_in_the_reflected_place() {
    let checked = check(PROJECT);
    let design = checked.clean();
    let space = |hardware: &str| {
        let hardware = design
            .hardware()
            .iter()
            .find(|h| h.name == hardware)
            .unwrap_or_else(|| panic!("no hardware {hardware}"));
        let region = hardware
            .roles
            .iter()
            .find_map(|(role, value)| match value {
                RoleValue::Region(region) if role.as_str() == "in" => Some(*region),
                _ => None,
            })
            .expect("an in: role");
        design.regions()[region.index()].name.clone()
    };
    assert_eq!(space("runner_l.slide"), "bay_l");
    assert_eq!(space("runner_r.slide"), "bay_r");
    // With nothing in the reflected place, the mirror says so.
    let start = PROJECT.find("region bay_r").unwrap();
    let end = PROJECT.find("assembly runner_l").unwrap();
    let without = format!("{}{}", &PROJECT[..start], &PROJECT[end..]);
    let checked = check(&without);
    assert_eq!(
        checked.messages(),
        ["bay_l has no counterpart on the other side of the mirror"]
    );
}
