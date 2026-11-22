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
    let faded = view.project().faded_nodes();
    assert!(!faded.is_empty(), "the taller sides' originals are faded");
    for node in faded {
        assert_eq!(drawing.looks[node.index()], Look::Ghost);
    }
    assert_eq!(plain + ghosts.len() + faded.len(), drawing.looks.len());

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

/// The box without its bottom (and the groove that holds it), as a
/// suggestion's files.
fn box_without_bottom() -> Files {
    let original = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/box/box.stave"),
    )
    .unwrap();
    let start = original.find("/// The bottom floats").unwrap();
    let end = start + original[start..].find("\n}\n").unwrap() + 3;
    let mut text = original.clone();
    text.replace_range(start..end, "");
    let groove = text
        .lines()
        .find(|line| line.starts_with("joint bottom_groove"))
        .unwrap()
        .to_owned();
    let text = text.replace(&groove, "");
    assert!(!text.contains("part bottom"));
    Files::new("/p", BTreeMap::from([("box.stave".to_owned(), text)])).unwrap()
}

#[test]
fn a_part_the_suggestion_takes_out_is_still_drawn_faded() {
    let project = Project::open(&box_files(), Some(&box_without_bottom())).unwrap();
    let mut view = View::new(project, 400, 300);
    let graph = view.graph().unwrap();
    let bottom = graph
        .node_ids()
        .find(|id| graph.node(*id).unwrap().name == "bottom")
        .expect("the project's bottom is still in the scene");
    assert!(view.project().faded_nodes().contains(&bottom));
    let drawing = view.drawing().unwrap();
    assert_eq!(drawing.looks[bottom.index()], Look::Ghost);
    assert!(drawing.posed.shown(bottom).is_some(), "it is drawn");

    // Every node of the project is drawn, before the ghosts.
    let alone = Project::open(&box_files(), None).unwrap();
    let base_count = alone.graph().unwrap().nodes.len();
    for node in view.project().ghost_nodes() {
        assert!(node.index() >= base_count);
    }
    assert!(graph.nodes.len() >= base_count);
    // The lid still opens, against the project's own assemblies.
    assert_eq!(view.project().motions().len(), 1);
    view.set_pose(0, 1.0).unwrap();
}

#[test]
fn a_ghost_is_picked_as_the_suggestions() {
    let original = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/box/box.stave"),
    )
    .unwrap();
    let taller = original.replace(
        "param box_height = 110mm within",
        "param box_height = 150mm within",
    );
    let suggestion = Files::new("/p", BTreeMap::from([("box.stave".to_owned(), taller)])).unwrap();
    let mut view = View::new(
        Project::open(&box_files(), Some(&suggestion)).unwrap(),
        400,
        300,
    );
    // The middle of the view is on the taller box: a ghost, see-through
    // or not, is what a ray meets first where it stands out.
    let mut seen_ghost = false;
    for step in 0..40 {
        let y = 20.0 + 6.5 * step as f32;
        if let Some(picked) = view.pick(200.0, y)
            && picked.suggested
        {
            assert!(!picked.replaced);
            assert!(
                picked.what.starts_with("Part cut from walnut 12"),
                "{picked:?}"
            );
            seen_ghost = true;
        }
    }
    assert!(seen_ghost, "some click lands on a ghost");
}

#[test]
fn what_a_part_is_is_said_in_words() {
    let mut view = box_view();
    let picked = view.pick(200.0, 150.0).unwrap();
    assert!(!picked.what.contains('_'), "{picked:?}");
    assert!(picked.what.starts_with("Part cut from"), "{picked:?}");
}

/// A frame that tilts and rises (one assembly, two named drives): one
/// slider for each drive, labelled with its name, each moving the frame
/// its own way.
#[test]
fn a_frame_with_two_drives_has_a_slider_for_each() {
    let text = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../stave-app/tests/turning/rising_cheval.stave"),
    )
    .unwrap();
    let files = Files::new("/p", BTreeMap::from([("project.stave".to_owned(), text)])).unwrap();
    let mut view = View::new(Project::open(&files, None).unwrap(), 400, 300);
    let names: Vec<&str> = view
        .project()
        .motions()
        .iter()
        .map(|motion| motion.name.as_str())
        .collect();
    assert_eq!(names, ["frame.tilt", "frame.height"]);
    let turns: Vec<bool> = view
        .project()
        .motions()
        .iter()
        .map(|motion| motion.range.turns())
        .collect();
    assert_eq!(turns, [true, false]);
    let closed = view.posed().unwrap();
    view.set_pose(1, 0.5).unwrap();
    let lowered = view.posed().unwrap();
    assert_ne!(closed, lowered);
    view.set_pose(0, 0.25).unwrap();
    let both = view.posed().unwrap();
    assert_ne!(lowered, both);
    let summary: serde_json::Value = serde_json::from_str(&view.summary_json()).unwrap();
    assert_eq!(summary["motions"][1]["name"], "frame.height");
}

/// A frame set in a row of holes (grammar, "Mechanisms", set positions):
/// its height slider snaps to the nearest hole, its tilt slider doesn't.
#[test]
fn a_drive_with_set_positions_snaps_its_slider() {
    let text = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../stave-app/tests/turning/holed_cheval.stave"),
    )
    .unwrap();
    let files = Files::new("/p", BTreeMap::from([("project.stave".to_owned(), text)])).unwrap();
    let mut view = View::new(Project::open(&files, None).unwrap(), 400, 300);
    let height = view.project().motions()[1].clone();
    assert_eq!(height.name, "frame.height");
    // Four holes 100mm apart over 300mm: a third of the way is a hole.
    assert!((height.snapped(0.4) - 1.0 / 3.0).abs() < 1e-6);
    assert!((height.snapped(0.9) - 1.0).abs() < 1e-6);
    let tilt = view.project().motions()[0].clone();
    assert_eq!(tilt.snapped(0.4), 0.4);
    // Setting the slider between holes poses the frame at the nearest.
    view.set_pose(1, 0.4).unwrap();
    let between = view.posed().unwrap();
    view.set_pose(1, 1.0 / 3.0).unwrap();
    assert_eq!(between, view.posed().unwrap());
}
