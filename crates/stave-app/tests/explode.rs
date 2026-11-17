//! Exploded views and problems shown in the viewer: where parts go when a
//! scene is pulled apart, and how a problem poses and highlights the design.

use std::path::{Path, PathBuf};

use stave_app::view::{self, Item, SceneGraph};

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn graph(folder: &str) -> (stave_app::Opened, SceneGraph) {
    let opened = stave_app::open(&workspace().join(folder)).unwrap();
    let graph = view::scene(opened.design.as_ref().expect("it solves"));
    (opened, graph)
}

/// The explode offset of a part, millimetres in the render frame.
fn offset(graph: &SceneGraph, name: &str) -> glam::Vec3 {
    let node = graph
        .node_ids()
        .find(|id| graph.node(*id).is_some_and(|n| n.name == name))
        .unwrap_or_else(|| panic!("no node '{name}'"));
    graph.explode_offset(node)
}

#[test]
fn an_exploded_side_table_pulls_the_legs_off_the_aprons() {
    let (_, graph) = graph("examples/side_table");
    // An apron tenoned into two legs stays; the legs move away from it,
    // out at the corner (left and toward the viewer for the front left
    // leg), and the slats lift off the stretchers they are screwed to.
    assert_eq!(offset(&graph, "apron_front"), glam::Vec3::ZERO);
    let leg = offset(&graph, "leg[front_left]");
    assert!(leg.x < -30.0 && leg.z > 30.0, "{leg}");
    let back_right = offset(&graph, "leg[back_right]");
    assert!(back_right.x > 30.0 && back_right.z < -30.0, "{back_right}");
    assert!(offset(&graph, "slat[0]").y > 30.0);
    // Pulled apart by half, everything is halfway.
    let rest = graph.scenes.first().unwrap();
    let half = graph.pose_with(rest, &[], 0.5);
    let node = graph
        .node_ids()
        .find(|id| graph.node(*id).is_some_and(|n| n.name == "leg[front_left]"))
        .unwrap();
    let moved = half.transforms[node.index()].transform_point3(glam::Vec3::ZERO);
    assert!((moved - leg * 0.5).length() < 1e-3);
}

#[test]
fn hardware_explodes_with_its_part_and_scenes_say_when_they_explode() {
    let (opened, graph) = graph("examples/clothing_cabinet");
    let design = opened.design.as_ref().unwrap();
    let bench = graph.scene_named("drawer_bench").unwrap();
    assert!(bench.explode);
    for hardware in &design.hardware {
        let node = graph.node_of(Item::Hardware(hardware.id)).unwrap();
        let piece = graph.node_of(Item::Part(hardware.piece)).unwrap();
        assert_eq!(
            graph.explode_offset(node),
            graph.explode_offset(piece),
            "{}",
            hardware.name
        );
    }
}

#[test]
fn a_problem_poses_the_design_where_it_hits_and_shows_the_shared_wood() {
    let (opened, graph) = graph("tests/broken/door_hits_side");
    let problems = view::problems(&opened, &workspace());
    let contact = problems[0]
        .contact
        .as_ref()
        .expect("a motion problem has a contact");
    let motion = contact.motion.expect("it is a motion");
    let range = view::contact_motion_range(&graph, contact).unwrap();
    assert!(range.turns);
    let shown = graph.amount(motion.assembly, motion.shown).unwrap();
    assert!(
        (shown.to_degrees() - 92.1).abs() < 0.2,
        "{}",
        shown.to_degrees()
    );
    let posed = view::pose_at_contact(&graph, contact, None).unwrap();
    let highlight = view::highlight(&graph, contact);
    assert_eq!(highlight.nodes.len(), 2);
    assert!(!highlight.volumes.is_empty());
    // The door is turned, the side isn't.
    let door = highlight
        .nodes
        .iter()
        .find(|id| graph.node(**id).unwrap().name == "door.panel")
        .unwrap();
    assert_ne!(posed.transforms[door.index()], glam::Mat4::IDENTITY);
    // A slider moves it back to closed.
    let closed = view::pose_at_contact(&graph, contact, Some(0.0)).unwrap();
    assert_eq!(closed.transforms[door.index()], glam::Mat4::IDENTITY);
}

#[test]
fn an_overlap_at_rest_highlights_both_parts() {
    let (opened, graph) = graph("tests/broken/tenon_too_long");
    let problems = view::problems(&opened, &workspace());
    let contact = problems[0].contact.as_ref().unwrap();
    assert!(contact.motion.is_none() && contact.scene.is_none());
    let highlight = view::highlight(&graph, contact);
    assert_eq!(highlight.nodes.len(), 2);
    let volume = highlight.volumes[0];
    // 3mm past the bottom of the mortise, across the rail's length.
    assert!((volume.size().x - 3.0).abs() < 1e-3, "{volume:?}");
}

#[test]
fn a_turn_moves_nothing_out_so_the_chidori_lock_leaves_on_its_diagonal() {
    // The chidori's last step turns the lock; the push before it slides
    // the lock out along the diagonal of its section, level.
    let (_, graph) = graph("examples/joint_samples");
    let lock = offset(&graph, "stick_z");
    assert!(lock.y.abs() < 1e-3, "{lock}");
    assert!(
        (lock.x.abs() - lock.z.abs()).abs() < 1e-2 && lock.x.abs() > 20.0,
        "{lock}"
    );
}
