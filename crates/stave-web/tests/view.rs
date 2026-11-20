//! What the viewer shows, without a browser: the camera, the sliders, the
//! exploded view, picking, ghosts and the summary the page reads.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use stave_scene::Look;
use stave_web::{Files, Project, View};

fn box_files() -> Files {
    let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/box");
    let mut files = BTreeMap::new();
    for entry in fs::read_dir(&folder).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_str().unwrap().to_owned();
        files.insert(name, fs::read_to_string(&path).unwrap());
    }
    Files::new("/p", files).unwrap()
}

fn box_view() -> View {
    View::new(Project::open(&box_files(), None).unwrap(), 400, 300)
}

#[test]
fn a_click_in_the_middle_names_a_part_and_what_it_is_joined_to() {
    let mut view = box_view();
    let picked = view.pick(200.0, 150.0).expect("the box fills the middle");
    assert!(!picked.name.is_empty());
    assert!(!picked.joined.is_empty(), "{picked:?}");
    assert!(!picked.joined.contains(&picked.name));
    assert!(!picked.suggested);
    assert_eq!(view.drawing().unwrap().selection.len(), 1);

    let json: serde_json::Value = serde_json::from_str(&picked.to_json()).unwrap();
    assert_eq!(json["name"], picked.name.as_str());
}

#[test]
fn a_click_on_nothing_selects_nothing() {
    let mut view = box_view();
    assert!(view.pick(200.0, 150.0).is_some());
    assert!(view.pick(1.0, 1.0).is_none());
    assert!(view.drawing().unwrap().selection.is_empty());
}

#[test]
fn the_lid_slider_opens_the_lid() {
    let mut view = box_view();
    let closed = view.posed().unwrap();
    view.set_pose(0, 1.0).unwrap();
    let open = view.posed().unwrap();
    assert_ne!(closed, open);
    assert!(view.set_pose(1, 1.0).is_err());
}

#[test]
fn exploding_moves_the_parts_apart() {
    let mut view = box_view();
    let together = view.posed().unwrap();
    view.set_explode(1.0);
    assert_ne!(together, view.posed().unwrap());
    view.set_explode(0.0);
    assert_eq!(together, view.posed().unwrap());
}

#[test]
fn dragging_and_zooming_move_the_camera() {
    let mut view = box_view();
    let start = view.orbit();
    view.orbit_by(40.0, 0.0);
    assert_ne!(start.yaw, view.orbit().yaw);
    view.zoom(2.0);
    assert!(view.orbit().distance < start.distance);
}

#[test]
fn a_new_size_frames_the_design_again_until_the_camera_is_moved() {
    let mut view = box_view();
    let wide = view.orbit();
    view.resize(200, 300);
    let narrow = view.orbit();
    assert!(
        narrow.distance > wide.distance,
        "a narrower view stands back"
    );
    view.zoom(1.0);
    let zoomed = view.orbit();
    view.resize(400, 300);
    assert_eq!(
        view.orbit(),
        zoomed,
        "a moved camera stays where it was put"
    );
}

#[test]
fn a_suggestion_draws_its_ghosts_see_through_and_tinted() {
    let text = box_files();
    let original = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/box/box.stave"),
    )
    .unwrap();
    let taller = original.replace(
        "param box_height = 110mm within",
        "param box_height = 150mm within",
    );
    let suggestion = Files::new("/p", BTreeMap::from([("box.stave".to_owned(), taller)])).unwrap();
    let view = View::new(Project::open(&text, Some(&suggestion)).unwrap(), 400, 300);
    let drawing = view.drawing().unwrap();
    let ghosts = view.project().ghost_nodes();
    assert!(!ghosts.is_empty());
    for node in &ghosts {
        assert_eq!(drawing.looks[node.index()], Look::XRay);
        assert!(drawing.tints[node.index()][3] > 0.0);
    }
    let plain = drawing
        .looks
        .iter()
        .filter(|look| **look == Look::Normal)
        .count();
    assert_eq!(plain + ghosts.len(), drawing.looks.len());

    let summary: serde_json::Value = serde_json::from_str(&view.summary_json()).unwrap();
    assert_eq!(summary["suggestion"]["verdict"], "fits");
}

#[test]
fn the_summary_lists_a_slider_per_motion() {
    let view = box_view();
    let summary: serde_json::Value = serde_json::from_str(&view.summary_json()).unwrap();
    assert_eq!(summary["title"], "Box");
    assert_eq!(summary["placed"], true);
    assert_eq!(summary["errors"], 0);
    assert!(summary["suggestion"].is_null());
    let motions = summary["motions"].as_array().unwrap();
    assert_eq!(motions.len(), 1);
    assert_eq!(motions[0]["turns"], true);
}

#[test]
fn a_project_that_does_not_place_draws_nothing_and_says_so() {
    let files = Files::new(
        "/p",
        BTreeMap::from([(
            "broken.stave".to_owned(),
            "stave 1\n\nproject \"Broken\" {\n  units: mm\n}\n\npart a : nothing {}\n".to_owned(),
        )]),
    )
    .unwrap();
    let mut view = View::new(Project::open(&files, None).unwrap(), 400, 300);
    assert!(view.drawing().is_none());
    assert!(view.pick(200.0, 150.0).is_none());
    let summary: serde_json::Value = serde_json::from_str(&view.summary_json()).unwrap();
    assert_eq!(summary["placed"], false);
    assert!(summary["errors"].as_u64().unwrap() > 0);
}
