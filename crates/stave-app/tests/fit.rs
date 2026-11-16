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
fn a_dovetail_is_drawn_uncut_and_left_out_of_the_fit_check() {
    insta::assert_snapshot!(rendered("tests/broken/dovetail_not_cut"));
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
