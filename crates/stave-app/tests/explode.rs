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
    // The slider is staged: up to a half, assemblies move as wholes (the
    // table has none, so the legs haven't moved); then parts move apart.
    let rest = graph.scenes.first().unwrap();
    let node = graph
        .node_ids()
        .find(|id| graph.node(*id).is_some_and(|n| n.name == "leg[front_left]"))
        .unwrap();
    let at = |amount: f32| {
        graph.pose_with(rest, &[], amount).transforms[node.index()]
            .transform_point3(glam::Vec3::ZERO)
    };
    assert!(at(0.5).length() < 1e-3);
    assert!((at(0.75) - leg * 0.5).length() < 1e-3);
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
        // It first touches the wall at 0.6deg (its hinge edge swings
        // out); it is shown 2deg past that.
        (shown.to_degrees() - 2.6).abs() < 0.2,
        "{}",
        shown.to_degrees()
    );
    let posed = view::pose_at_contact(&graph, contact, None).unwrap();
    let highlight = view::highlight(&graph, contact);
    assert_eq!(highlight.nodes.len(), 2);
    assert!(!highlight.volumes.is_empty());
    // The door is turned, the wall isn't.
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

#[test]
fn assemblies_move_out_whole_first_drawers_forward_doors_off_their_hinges() {
    let (opened, graph) = graph("examples/clothing_cabinet");
    let design = opened.design.as_ref().unwrap();
    let rest = graph.scenes.first().unwrap();
    let node = |name: &str| {
        graph
            .node_ids()
            .find(|id| graph.node(*id).is_some_and(|n| n.name == name))
            .unwrap_or_else(|| panic!("no node '{name}'"))
    };
    let half = graph.pose_with(rest, &[], 0.5);
    let moved = |name: &str| half.transforms[node(name).index()].transform_point3(glam::Vec3::ZERO);
    // The drawer comes out toward the viewer by more than its depth, as a
    // whole: its front and its back move together.
    let depth = design
        .part_named("drawer[top].side[left]")
        .map(|p| p.cut.length.to_mm_f64())
        .unwrap();
    let front = moved("drawer[top].front");
    assert!(f64::from(front.z) > depth, "{front}");
    assert_eq!(front, moved("drawer[top].back"));
    // A door comes forward off its hinges, its parts together.
    let stile = moved("door_l.stile[hinge]");
    assert!(stile.z > 100.0 && stile == moved("door_l.panel"), "{stile}");
    // The carcass hasn't moved yet.
    assert_eq!(moved("top"), glam::Vec3::ZERO);
    // The slide's cabinet member stays with the cabinet: it never comes
    // out with the drawer.
    let member = graph
        .node_ids()
        .find(|id| {
            graph
                .node(*id)
                .is_some_and(|n| n.name == "drawer[top].slides[left] cabinet member")
        })
        .unwrap();
    assert!(
        half.transforms[member.index()]
            .transform_point3(glam::Vec3::ZERO)
            .z
            .abs()
            < 1e-3
    );
}
