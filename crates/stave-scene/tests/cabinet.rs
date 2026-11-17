//! The scene graph of the clothing cabinet example: bounds, picking and
//! scene poses.

use std::path::Path;

use glam::Vec3;
use stave_lang::placement::Direction;
use stave_lang::units::Length;
use stave_model::mechanism::{FramePoint, MechanismEntry, MemberIndex};
use stave_model::solved::{BodyCarrier, Hardware, Point, SolvedDesign};
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
    let carried = |test: &dyn Fn(&Hardware, BodyCarrier) -> bool| {
        design
            .hardware
            .iter()
            .flat_map(|h| h.members.iter().map(move |m| (h, m.carrier)))
            .filter(|(h, carrier)| test(h, *carrier))
            .count()
    };
    let held = carried(&|h, carrier| match carrier {
        BodyCarrier::Part(part) => part != h.piece,
        BodyCarrier::Region(_) => true,
        BodyCarrier::Link(_) => false,
    });
    assert_eq!(
        held, 12,
        "four hinge plates and arms, and four slide cabinet members"
    );
    let links = carried(&|_, carrier| matches!(carrier, BodyCarrier::Link(_)));
    assert_eq!(links, 8, "two links on each of the four cup hinges");
    // One node for each part and one for each member of each piece of
    // hardware, named `<hardware> <member>`.
    let members: usize = design.hardware.iter().map(|h| h.members.len()).sum();
    assert_eq!(graph.nodes.len(), design.parts.len() + members);
    for hardware in &design.hardware {
        for member in &hardware.members {
            let name = format!("{} {}", hardware.name, member.name);
            assert!(graph.nodes.iter().any(|node| node.name == name), "{name}");
        }
    }
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
    assert_eq!(
        pick(450.0, 139.0).as_deref(),
        Some("drawer[bottom].pull bar")
    );
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
    let slide = posed
        .shown(node("drawer[top].slides[left] drawer_member"))
        .unwrap();
    // (the drawer member)
    assert!(close(
        slide.transform_point3(Vec3::ZERO),
        Vec3::new(0.0, 0.0, 500.0)
    ));

    // The left door opens 110° on its cup hinges' linkage: turned by 110°
    // and moved, its front hinge corner (1, y, 19) carried to about
    // (9.97, y, 2.95) (docs/hardware/cup-hinge-linkage.md), the latch edge out to
    // the front and left.
    let angle = 110_f32.to_radians();
    let door_l = posed.shown(node("door_l.stile[latch]")).unwrap();
    let corner = Vec3::new(1.0, 1000.0, 19.0);
    let moved = door_l.transform_point3(corner);
    assert!(
        (moved - Vec3::new(9.97, 1000.0, 2.95)).abs().max_element() < 0.05,
        "{moved}"
    );
    let latch_corner = Vec3::new(449.25, 1000.0, 19.0);
    let reach = 448.25;
    let expected = moved + Vec3::new(angle.cos(), 0.0, angle.sin()) * reach;
    assert!(
        (door_l.transform_point3(latch_corner) - expected).length() < 0.01,
        "{}",
        door_l.transform_point3(latch_corner)
    );
    // The hinge cups go with the door; the plates stay on the side, and so
    // do the slides' cabinet members.
    let hinge = posed.shown(node("door_l.hinges[upper] cup")).unwrap();
    assert_eq!(hinge, door_l);
    for fixed in [
        "door_l.hinges[upper] plate",
        "door_l.hinges[upper] arm",
        "door_r.hinges[lower] plate",
        "door_r.hinges[lower] arm",
        "drawer[top].slides[left] cabinet_member",
    ] {
        assert_eq!(
            posed.shown(node(fixed)).unwrap(),
            glam::Mat4::IDENTITY,
            "{fixed}"
        );
    }

    // The right door is its mirror image.
    let door_r = posed.shown(node("door_r.stile[latch]")).unwrap();
    let corner_r = door_r.transform_point3(Vec3::new(899.0, 1000.0, 19.0));
    assert!(
        (corner_r - Vec3::new(899.0 - 8.97, 1000.0, moved.z))
            .abs()
            .max_element()
            < 0.05,
        "{corner_r}"
    );
    let latch_r = Vec3::new(450.75, 1000.0, 19.0);
    let expected_r = corner_r + Vec3::new(-angle.cos(), 0.0, angle.sin()) * reach;
    assert!((door_r.transform_point3(latch_r) - expected_r).length() < 0.01);

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
            node.name.starts_with("drawer[top].") && !node.name.ends_with("cabinet_member"),
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

#[test]
fn the_box_lid_lifts_on_its_butt_hinges() {
    let design = solve("box");
    let graph = SceneGraph::build(&design, BuildOptions::default());
    let scene = graph.scene_named("lid_open").unwrap();
    let posed = graph.pose(scene);
    let node = |name: &str| {
        graph
            .node_ids()
            .find(|id| graph.node(*id).unwrap().name == name)
            .unwrap_or_else(|| panic!("no node '{name}'"))
    };
    // The lid turns 100° about the hinge pins, in their knuckles at the top
    // of the back's back face, 2.5mm behind it: (x, 110, -202.5) in the
    // render frame, where +z is the front.
    let lid = posed.shown(node("lid.panel")).unwrap();
    let pivot = Vec3::new(150.0, 110.0, -202.5);
    assert!(close(lid.transform_point3(pivot), pivot));
    // Its front top edge swings up and back over the box.
    let angle = 100_f32.to_radians();
    let front_top = Vec3::new(150.0, 122.0, 0.0);
    let (dy, dz) = (12.0, 202.5);
    let expected = pivot
        + Vec3::new(
            0.0,
            dy * angle.cos() + dz * angle.sin(),
            -dy * angle.sin() + dz * angle.cos(),
        );
    assert!(
        close(lid.transform_point3(front_top), expected),
        "{:?} != {expected:?}",
        lid.transform_point3(front_top)
    );
    // The leaves in the lid and the knob go with it; the leaves in the back
    // and the body stay.
    assert_eq!(posed.shown(node("lid.hinges[left] leaf")).unwrap(), lid);
    assert_eq!(posed.shown(node("lid.lift_knob knob")).unwrap(), lid);
    for still in ["lid.hinges[left] fixed_leaf", "back", "front"] {
        assert_eq!(
            posed.shown(node(still)).unwrap(),
            glam::Mat4::IDENTITY,
            "{still}"
        );
    }
}

#[test]
fn every_example_builds_and_poses_every_scene() {
    for example in [
        "bookshelf",
        "box",
        "nightstand",
        "side_table",
        "stool",
        "wall_shelf",
    ] {
        let design = solve(example);
        let graph = SceneGraph::build(&design, BuildOptions::default());
        assert!(
            graph.nodes.len() >= design.parts.len(),
            "{example}: every part has a node"
        );
        for scene in &design.scenes {
            let entry = graph.scene_named(&scene.name).unwrap();
            let posed = graph.pose(entry);
            assert!(graph.bounds(&posed).is_some(), "{example}: {}", scene.name);
        }
    }
}

/// Scrubbing a door open carries each cup hinge's links with it: the two
/// members of every pivot of the door's mechanism keep their pin together
/// (a link's pin in the arm stays where it is, as the arm does; its pin in
/// the boss moves with the cup and the door), at every angle.
#[test]
fn the_hinge_links_follow_the_door() {
    let (design, graph) = cabinet();
    let door = design
        .assemblies
        .iter()
        .find(|a| a.name == "door_l")
        .unwrap();
    let mechanism = door.mechanism.as_ref().unwrap();
    let frame = mechanism.frame();
    let world = |at: FramePoint| {
        let step = |point: Point, direction: Direction, amount: Length| {
            let signed = if direction.is_positive() {
                amount.nm()
            } else {
                -amount.nm()
            };
            let axis = direction.axis();
            point.with(axis, Length::from_nm(point.get(axis).nm() + signed))
        };
        stave_geom::render::point(step(
            step(frame.origin(), frame.across(), at.across),
            frame.depth(),
            at.depth,
        ))
    };
    let node = |index: MemberIndex| {
        let member = mechanism.members().get(index.index()).unwrap();
        let name = format!("door_l.hinges[lower] {}", member.name);
        graph
            .node_ids()
            .find(|id| graph.node(*id).is_some_and(|n| n.name == name))
            .unwrap()
    };
    let pivots: Vec<_> = mechanism
        .entries()
        .iter()
        .filter_map(|entry| match *entry {
            MechanismEntry::Pivot { first, second, at } => Some((node(first), node(second), at)),
            MechanismEntry::Slot { .. }
            | MechanismEntry::Rail { .. }
            | MechanismEntry::Couple { .. } => None,
        })
        .collect();
    assert_eq!(pivots.len(), 4);
    let rest = graph.scenes.first().unwrap();
    for degrees in [0.0f32, 15.0, 45.0, 90.0, 110.0] {
        let posed = graph.pose_with(rest, &[(door.id, degrees.to_radians())], 0.0);
        for (first, second, at) in &pivots {
            let pin = world(*at);
            let a = posed.transforms[first.index()].transform_point3(pin);
            let b = posed.transforms[second.index()].transform_point3(pin);
            assert!(close(a, b), "{degrees}deg: a pivot came apart: {a} {b}");
        }
    }
}
