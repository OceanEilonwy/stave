//! Fit and geometry (layer 4) and scene poses: what a user sees for each
//! broken design of these checks, and what the examples' joints do once
//! their cuts are made.

mod common;

use std::path::Path;

use stave_app::Opened;
use stave_geom::collide::{self, WorldSolid};
use stave_geom::solid::Solid;
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

fn example(name: &str) -> Opened {
    stave_app::open(&workspace().join("examples").join(name)).unwrap()
}

#[test]
fn a_tenon_that_bottoms_out_in_its_mortise() {
    insta::assert_snapshot!(rendered("tests/broken/tenon_too_long"));
}

#[test]
fn a_shelf_thicker_than_its_dado() {
    insta::assert_snapshot!(rendered("tests/broken/shelf_misses_dado"));
}

#[test]
fn a_knob_in_the_way_of_another_part() {
    insta::assert_snapshot!(rendered("tests/broken/knob_hits_top"));
}

#[test]
fn a_joint_whose_parts_never_touch() {
    insta::assert_snapshot!(rendered("tests/broken/joint_parts_apart"));
}

#[test]
fn a_pitch_that_runs_past_the_end() {
    insta::assert_snapshot!(rendered("tests/broken/pitch_runs_past_end"));
}

#[test]
fn a_lid_that_hits_the_wall_in_a_scene() {
    insta::assert_snapshot!(rendered("tests/broken/scene_pose_clash"));
}

#[test]
fn a_drawer_too_wide_for_its_slides() {
    insta::assert_snapshot!(rendered("tests/broken/drawer_does_not_fit"));
}

#[test]
fn a_dovetail_tail_wider_than_its_socket_overlaps_it() {
    insta::assert_snapshot!(rendered("tests/broken/dovetail_too_wide"));
}

#[test]
fn an_outline_that_crosses_itself_is_a_type_mismatch() {
    insta::assert_snapshot!(rendered("tests/broken/outline_crosses_itself"));
}

/// The cut solids of two parts of a design, placed.
fn solids(opened: &Opened, a: &str, b: &str) -> (WorldSolid, WorldSolid) {
    let design = common::design(opened);
    let solid = |name: &str| {
        let part = common::part(design, name);
        WorldSolid::of_part(part, &Solid::of(part, design))
    };
    (solid(a), solid(b))
}

#[test]
fn every_tenon_and_shelf_of_the_examples_sits_in_its_housing() {
    // The tenons reach into the legs, the shelves into the sides' dados:
    // their cut boxes overlap, their cut solids only touch.
    for (folder, a, b) in [
        ("side_table", "apron_front", "leg[front_left]"),
        ("bookshelf", "shelf[0]", "side_l"),
        ("box", "bottom", "front"),
        ("stool", "leg[front_left]", "seat"),
        ("stool", "rung[front]", "leg[front_left]"),
    ] {
        let opened = example(folder);
        let design = common::design(&opened);
        let boxes = (
            common::part(design, a).cut_box(),
            common::part(design, b).cut_box(),
        );
        assert!(
            collide::boxes_overlap(&boxes.0, &boxes.1),
            "{folder}: {a} doesn't reach into {b}"
        );
        let (x, y) = solids(&opened, a, b);
        assert!(collide::overlaps(&x, &y).is_empty(), "{folder}: {a} in {b}");
        assert!(collide::touches(&x, &y), "{folder}: {a} doesn't touch {b}");
    }
}

#[test]
fn the_examples_check_clean_with_their_cuts_made() {
    for folder in [
        "bookshelf",
        "box",
        "clothing_cabinet",
        "nightstand",
        "side_table",
        "stool",
        "wall_shelf",
    ] {
        let opened = example(folder);
        assert!(
            opened.diagnostics.is_empty(),
            "{folder}:\n{}",
            render_diagnostics(&opened.sources, &opened.diagnostics)
        );
    }
}

#[test]
fn a_door_whose_hinge_edge_swings_into_a_wall() {
    insta::assert_snapshot!(rendered("tests/broken/door_hits_side"));
}

#[test]
fn a_door_too_far_out_for_its_cup_hinges() {
    insta::assert_snapshot!(rendered("tests/broken/hinge_overlay"));
}

/// Before the mechanism was text, a built-in cap stopped every cup hinge
/// at 110deg. Now a kind that copies `euro_hinge`'s mechanism word for
/// word and asks it to open 125deg is stopped by the mechanism itself: its
/// links don't lock before 180deg, but past 112.1deg its inner link runs
/// into the door, which is why the kind's stop is at 110deg. The motion
/// sweep that swings doors finds it, as it finds any body in the way.
#[test]
fn a_cup_hinge_opens_at_most_110_degrees() {
    let library = std::fs::read_to_string(workspace().join("std/hardware/hinges.stave")).unwrap();
    let start = library.find("kind euro_hinge : hinge {").unwrap();
    let length = library[start..].find("\n}\n").unwrap() + 3;
    let opening = "param opening_angle = 110deg within 85deg..110deg";
    let euro_hinge = &library[start..start + length];
    assert!(euro_hinge.contains(opening) && euro_hinge.contains("mechanism {"));
    let wide_hinge = euro_hinge
        .replace("kind euro_hinge", "kind wide_hinge")
        .replace(opening, "param opening_angle = 125deg within 85deg..125deg");
    let design =
        std::fs::read_to_string(workspace().join("tests/broken/hinge_overlay/project.stave"))
            .unwrap()
            .replace(
                "overlay(left: 9mm, right: 9mm)",
                "overlay(left: 17.5mm, right: 17.5mm)",
            )
            .replace(
                "euro_hinge(panel, on: hinge_side)",
                "wide_hinge(panel, on: hinge_side)",
            )
            + "\n"
            + &wide_hinge;
    let opened = common::open_files(&[("project.stave", &design)]);
    let messages = common::diagnostics(&opened);
    assert!(
        messages.iter().any(|m| m
            == "motion: door.hinges[bottom]'s inner_link hits door.panel when door is opened past 112.1deg"),
        "{messages:?}"
    );
}

#[test]
fn two_doors_on_one_divider_hit_each_other() {
    insta::assert_snapshot!(rendered("tests/broken/doors_hit_each_other"));
}

#[test]
fn a_door_that_swings_into_a_knob() {
    insta::assert_snapshot!(rendered("tests/broken/knob_hits_door"));
}

#[test]
fn a_drawer_that_catches_on_a_rail() {
    insta::assert_snapshot!(rendered("tests/broken/drawer_hits_divider"));
}

#[test]
fn a_lid_that_drops_onto_the_back() {
    insta::assert_snapshot!(rendered("tests/broken/lid_hits_back"));
}

#[test]
fn a_tenon_pushed_in_the_wrong_way() {
    insta::assert_snapshot!(rendered("tests/broken/tenon_wrong_way"));
}

#[test]
fn a_chidori_without_flats_cannot_come_apart() {
    insta::assert_snapshot!(rendered("tests/broken/chidori_no_flats"));
}

/// The project text of a broken design.
fn broken_text(name: &str) -> String {
    std::fs::read_to_string(
        workspace()
            .join("tests/broken")
            .join(name)
            .join("project.stave"),
    )
    .unwrap()
}

#[test]
fn the_chidori_comes_apart_as_the_simulation_proves() {
    // As oracle_chidori checks: with its flats and its rounding the node
    // comes apart (a quarter turn back, out along the diagonal, the second
    // stick lifted off); without the flats the lock can't slide out, and
    // without the rounding it doesn't fit in the holes at all.
    let order = |opened: &Opened| {
        opened
            .diagnostics
            .iter()
            .filter(|d| d.code == stave_lang::Code::Order)
            .count()
    };
    let samples = example("joint_samples");
    assert_eq!(
        order(&samples),
        0,
        "{}",
        render_diagnostics(&samples.sources, &samples.diagnostics)
    );
    let with_flats = broken_text("chidori_no_flats").replace(
        "round(diameter: round_diameter)",
        "round(diameter: round_diameter, flats: flats, flats_angle: 45deg)",
    );
    let opened = common::open_files(&[("project.stave", &with_flats)]);
    assert!(
        opened.diagnostics.is_empty(),
        "{}",
        render_diagnostics(&opened.sources, &opened.diagnostics)
    );
    let no_flats = common::open_files(&[("project.stave", &broken_text("chidori_no_flats"))]);
    assert_eq!(order(&no_flats), 1);
    let unrounded = broken_text("chidori_no_flats").replace(
        "cut crossing(lock, first) {\n    round(diameter: round_diameter)\n  }",
        "",
    );
    let opened = common::open_files(&[("project.stave", &unrounded)]);
    let errors: Vec<_> = opened.diagnostics.iter().filter(|d| d.is_error()).collect();
    assert_eq!(
        errors.len(),
        1,
        "{}",
        render_diagnostics(&opened.sources, &opened.diagnostics)
    );
    assert_eq!(errors[0].code, stave_lang::Code::Overlap);
}

#[test]
fn every_motion_of_the_examples_is_clear_through_its_travel() {
    // The cabinet's doors and drawer, the nightstand's drawer and the box's
    // lid (to 180 degrees) are swept from rest and from every scene.
    for folder in ["clothing_cabinet", "nightstand", "box"] {
        let opened = example(folder);
        let design = common::design(&opened);
        assert!(
            design.assemblies.iter().any(|a| a.motion.is_some()),
            "{folder}"
        );
        assert!(opened.diagnostics.is_empty(), "{folder}");
    }
}

#[test]
fn a_door_overlay_into_the_top_is_one_error_at_the_mount() {
    insta::assert_snapshot!(rendered("tests/broken/door_overlay_into_top"));
}

#[test]
fn double_doors_with_no_gap_are_told_the_gap_they_need() {
    insta::assert_snapshot!(rendered("tests/broken/double_doors_no_gap"));
}

/// The one cut the geometry can't make yet: anything but a hole or a round
/// tenon in a round or turned part. It is a warning, the part is drawn
/// uncut there, and its fit is checked as drawn.
#[test]
fn a_notch_in_a_round_part_is_not_supported() {
    let opened = common::open(
        "stock dowel_30 { species: beech, form: dowel, diameter: 30mm }
part rail : dowel_30 {
  length: 200mm
  place {
    against(base.top)
    flush(base.left)
    flush(base.front)
  }
}
part post : pine_19 {
  length: 100mm
  width:  60mm
  place {
    orient(length: up, width: back)
    against(base.top)
    against(rail.right)
    flush(base.front)
  }
}
kind flatten : joint {
  cut crossing(piece, into) {
    notch(depth: 5mm)
  }
}
joint seat : flatten(rail, into: post)",
    );
    let found = common::diagnostics(&opened);
    assert_eq!(
        found,
        ["not-supported: cutting a notch in a round or turned part is not supported yet"],
        "{found:?}"
    );
    let note = &opened.diagnostics[0].notes;
    assert_eq!(
        note,
        &["seat: rail is drawn uncut there, and its fit is checked as drawn".to_owned()]
    );
}

/// Every message of a design, with the notes a merged diagnostic carries
/// (the other mistakes its line causes).
fn messages_and_notes(opened: &stave_app::Opened) -> Vec<String> {
    opened
        .diagnostics
        .iter()
        .flat_map(|d| std::iter::once(d.message.clone()).chain(d.notes.iter().cloned()))
        .collect()
}

/// A cup is checked against its own door like any body: it sits in the
/// door only because its kind bores the hole. `broken/hinge_link_in_wood`
/// with the catalogue hinge's pins is a clean design; take the kind's
/// `cut bore:` out and the cup (and the arm's knuckle that reaches into
/// it) overlap the door.
#[test]
fn a_cup_sits_in_its_bore_and_nowhere_else() {
    let design =
        std::fs::read_to_string(workspace().join("tests/broken/hinge_link_in_wood/project.stave"))
            .unwrap()
            .replace("depth: 2.89mm", "depth: 1.39mm");
    let bored = common::open_files(&[("project.stave", &design)]);
    assert!(
        bored.diagnostics.is_empty(),
        "{:?}",
        common::diagnostics(&bored)
    );
    let cut = "  cut bore: crossing(piece, cup) { drill(diameter: 35mm, depth: 12.8mm) }\n";
    assert!(design.contains(cut));
    let unbored = common::open_files(&[("project.stave", &design.replace(cut, ""))]);
    let messages = messages_and_notes(&unbored);
    assert!(
        messages
            .iter()
            .any(|m| m.contains("door.hinges[bottom]'s cup and door.panel overlap")),
        "{messages:?}"
    );
    assert!(
        unbored
            .diagnostics
            .iter()
            .all(|d| d.code == stave_lang::Code::Overlap),
        "{messages:?}"
    );
}

/// A butt hinge's leaf is a body like any other: the box example's lid is
/// clean, and with a copy of `butt_hinge` that cuts no mortises its leaves
/// overlap the lid and the back they should be housed in.
#[test]
fn a_leaf_without_its_mortise_overlaps() {
    let read = |name: &str| std::fs::read_to_string(workspace().join(name)).unwrap();
    let library = read("std/hardware/hinges.stave");
    let start = library.find("kind butt_hinge : hinge {").unwrap();
    let length = library[start..].find("\n}\n").unwrap() + 3;
    let butt_hinge = &library[start..start + length];
    let mortises = "  cut mortise: crossing(piece, leaf) { notch(depth: 1.5mm) }\n  cut mortise: crossing(on, fixed_leaf) { notch(depth: 1.5mm) }\n";
    assert!(butt_hinge.contains(mortises));
    let bare = butt_hinge
        .replace("kind butt_hinge", "kind bare_butt_hinge")
        .replace(mortises, "");
    let lid = read("examples/box/lid.stave");
    let housed = common::open_files(&[
        ("box.stave", &read("examples/box/box.stave")),
        ("lid.stave", &lid),
    ]);
    assert!(
        housed.diagnostics.is_empty(),
        "{:?}",
        common::diagnostics(&housed)
    );
    let unhoused = common::open_files(&[
        ("box.stave", &read("examples/box/box.stave")),
        (
            "lid.stave",
            &(lid.replace("butt_hinge(length: 40mm)", "bare_butt_hinge(length: 40mm)")
                + "\n"
                + &bare),
        ),
    ]);
    let messages = messages_and_notes(&unhoused);
    assert!(
        messages
            .iter()
            .any(|m| m.contains("'s leaf and lid.panel overlap")),
        "{messages:?}"
    );
    assert!(
        messages.iter().any(|m| m.contains("'s fixed_leaf and")),
        "{messages:?}"
    );
}
