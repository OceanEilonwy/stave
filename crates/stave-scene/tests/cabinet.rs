//! The scene graph of the clothing cabinet example: bounds, picking and
//! scene poses.

use std::path::Path;

use glam::Vec3;
use stave_model::solved::SolvedDesign;
use stave_scene::{BuildOptions, Orbit, Ray, SceneGraph};

fn solve(folder: &str) -> SolvedDesign {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples")
        .join(folder);
    let modules = stave_modules::ModuleSet::load(&path).expect("the example loads");
    let (defs, _) = stave_model::resolve(&modules);
    let (model, _) = stave_model::instance::instantiate(defs, modules.root);
    let (design, diagnostics) = stave_solve::solve(&model);
    design.unwrap_or_else(|| panic!("the example solves: {diagnostics:?}"))
}

fn cabinet() -> (SolvedDesign, SceneGraph) {
    let design = solve("clothing_cabinet");
    let graph = SceneGraph::build(&design, BuildOptions::default());
    (design, graph)
}

fn close(a: Vec3, b: Vec3) -> bool {
    (a - b).abs().max_element() < 0.01
}

#[test]
fn every_part_and_hardware_has_a_node() {
    let (design, graph) = cabinet();
    let fixed = design.hardware.iter().filter(|h| h.fixed.is_some()).count();
    assert_eq!(fixed, 8, "four hinge plates and four slide cabinet members");
    assert_eq!(
        graph.nodes.len(),
        design.parts.len() + design.hardware.len() + fixed
    );
    assert_eq!(graph.scenes.first().map(|s| s.name.as_str()), Some("rest"));
    let stile = graph
        .nodes
        .iter()
        .find(|node| node.name == "door_r.stile[hinge]")
        .unwrap();
    let door_r = design
        .assemblies
        .iter()
        .find(|a| a.name == "door_r")
        .unwrap();
    assert_eq!(stile.assemblies, vec![door_r.id]);
}

#[test]
fn framing_bounds_cover_the_cabinet() {
    let (_, graph) = cabinet();
    let bounds = graph.bounds(&graph.rest()).unwrap();
    // Feet down to -80, top up to 1700; the top overhangs 20mm each side
    // and at the front; the bar pulls stand 54mm in front.
    assert!(
        close(bounds.min, Vec3::new(-20.0, -80.0, -560.0)),
        "{bounds:?}"
    );
    assert!(
        close(bounds.max, Vec3::new(920.0, 1700.0, 54.0)),
        "{bounds:?}"
    );
    let orbit = Orbit::framing(bounds, 1.6);
    let clip = orbit.view_projection(1.6);
    for corner in bounds.corners() {
        let p = clip.project_point3(corner);
        assert!(p.x.abs() <= 1.0 && p.y.abs() <= 1.0);
    }
}

fn from_front(x: f32, y: f32) -> Ray {
    Ray {
        origin: Vec3::new(x, y, 5000.0),
        direction: Vec3::NEG_Z,
    }
}

#[test]
fn picking_from_the_front() {
    let (_, graph) = cabinet();
    let rest = graph.rest();
    let pick = |x, y| {
        graph
            .pick(&rest, from_front(x, y))
            .and_then(|hit| graph.node(hit.node))
            .map(|node| node.name.clone())
    };
    // The door's panel sits back in its frame; the frame is in front.
    assert_eq!(pick(200.0, 1100.0).as_deref(), Some("door_l.panel"));
    assert_eq!(pick(30.0, 1100.0).as_deref(), Some("door_l.stile[hinge]"));
    assert_eq!(pick(700.0, 1100.0).as_deref(), Some("door_r.panel"));
    assert_eq!(pick(200.0, 150.0).as_deref(), Some("drawer[bottom].front"));
    assert_eq!(pick(450.0, 139.0).as_deref(), Some("drawer[bottom].pull"));
    assert_eq!(pick(450.0, 1690.0).as_deref(), Some("top"));
    assert_eq!(pick(95.0, -40.0).as_deref(), Some("foot[front_left]"));
    assert_eq!(pick(1200.0, 500.0), None);

    // Through the framing camera: the middle of the view lands on the
    // cabinet, and the nearest thing along that ray is in front.
    let bounds = graph.bounds(&rest).unwrap();
    let orbit = Orbit::framing(bounds, 1.0);
    let hit = graph.pick(&rest, orbit.ray(500.0, 500.0, 1000.0, 1000.0));
    assert!(hit.is_some());
}

#[test]
fn the_open_scene_swings_doors_and_slides_drawers() {
    let (_, graph) = cabinet();
    let scene = graph.scene_named("all_open").unwrap();
    let posed = graph.pose(scene);
    let node = |name: &str| {
        graph
            .node_ids()
            .find(|id| graph.node(*id).unwrap().name == name)
            .unwrap()
    };

    // The top drawer comes 500mm toward the front; the bottom one stays.
    let top = posed.shown(node("drawer[top].front")).unwrap();
    assert!(close(
        top.transform_point3(Vec3::ZERO),
        Vec3::new(0.0, 0.0, 500.0)
    ));
    let bottom = posed.shown(node("drawer[bottom].front")).unwrap();
    assert!(close(bottom.transform_point3(Vec3::ZERO), Vec3::ZERO));
    // The slides travel with it.
    let slide = posed.shown(node("drawer[top].slides[left]")).unwrap();
    // (the drawer member)
    assert!(close(
        slide.transform_point3(Vec3::ZERO),
        Vec3::new(0.0, 0.0, 500.0)
    ));

    // The left door turns 110° about its outer front corner, (1, y, 19):
    // the latch edge swings out to the front and left.
    let angle = 110_f32.to_radians();
    let door_l = posed.shown(node("door_l.stile[latch]")).unwrap();
    let pivot = Vec3::new(1.0, 1000.0, 19.0);
    assert!(close(door_l.transform_point3(pivot), pivot));
    let latch_corner = Vec3::new(449.25, 1000.0, 19.0);
    let reach = 448.25;
    let expected = pivot + Vec3::new(angle.cos(), 0.0, angle.sin()) * reach;
    assert!(close(door_l.transform_point3(latch_corner), expected));
    // The hinge cups go with the door; the plates stay on the side, and so
    // do the slides' cabinet members.
    let hinge = posed.shown(node("door_l.hinges[upper]")).unwrap();
    assert_eq!(hinge, door_l);
    for fixed in [
        "door_l.hinges[upper] plate",
        "door_r.hinges[lower] plate",
        "drawer[top].slides[left] cabinet member",
    ] {
        assert_eq!(
            posed.shown(node(fixed)).unwrap(),
            glam::Mat4::IDENTITY,
            "{fixed}"
        );
    }

    // The right door is its mirror image.
    let door_r = posed.shown(node("door_r.stile[latch]")).unwrap();
    let pivot_r = Vec3::new(899.0, 1000.0, 19.0);
    let latch_r = Vec3::new(450.75, 1000.0, 19.0);
    let expected_r = pivot_r + Vec3::new(-angle.cos(), 0.0, angle.sin()) * reach;
    assert!(close(door_r.transform_point3(latch_r), expected_r));

    // The carcass doesn't move.
    let side = posed.shown(node("side_l")).unwrap();
    assert_eq!(side, glam::Mat4::IDENTITY);
}

#[test]
fn a_scene_can_show_one_assembly() {
    let (_, graph) = cabinet();
    let scene = graph.scene_named("drawer_bench").unwrap();
    let posed = graph.pose(scene);
    for id in graph.node_ids() {
        let node = graph.node(id).unwrap();
        // The slides' cabinet members are screwed to the carcass, not the
        // drawer.
        assert_eq!(
            posed.shown(id).is_some(),
            node.name.starts_with("drawer[top].") && !node.name.ends_with("cabinet member"),
            "{}",
            node.name
        );
    }
}

#[test]
fn the_joint_samples_build() {
    let design = solve("joint_samples");
    let graph = SceneGraph::build(&design, BuildOptions::default());
    assert_eq!(graph.nodes.len(), design.parts.len());
    let glb = graph.to_glb(&graph.rest()).unwrap();
    assert_eq!(&glb[0..4], b"glTF");
}
