//! Picking what the editor turns into typed values, on the solved
//! clothing cabinet: a part with the face hit, the region a ray looks
//! into, and the two parts facing each other across a gap.

use std::path::Path;

use glam::Vec3;
use stave_lang::placement::{Axis, Face};
use stave_lang::units::Length;
use stave_model::solved::{Point, SolvedDesign};
use stave_scene::{BuildOptions, Item, PartFace, Posed, Ray, SceneGraph, Target};

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

/// The cabinet at rest with both doors (and everything in their
/// assemblies) hidden.
fn doors_hidden(design: &SolvedDesign, graph: &SceneGraph) -> Posed {
    let doors: Vec<_> = design
        .assemblies
        .iter()
        .filter(|assembly| assembly.name == "door_l" || assembly.name == "door_r")
        .map(|assembly| assembly.id)
        .collect();
    assert_eq!(doors.len(), 2);
    let mut posed = graph.rest();
    for id in graph.node_ids() {
        let node = graph.node(id).unwrap();
        if node
            .assemblies
            .iter()
            .any(|assembly| doors.contains(assembly))
        {
            posed.visible[id.index()] = false;
        }
    }
    posed
}

/// A ray from far off along `direction`, aimed through `through`.
fn toward(through: Vec3, direction: Vec3) -> Ray {
    Ray {
        origin: through - direction * 5000.0,
        direction,
    }
}

/// A ray from in front of the cabinet, straight back, through `(x, y)`.
fn from_front(x: f32, y: f32) -> Ray {
    toward(Vec3::new(x, y, 0.0), Vec3::NEG_Z)
}

fn part_name(design: &SolvedDesign, face: PartFace) -> &str {
    &design.part(face.part).unwrap().name
}

#[test]
fn a_pick_on_a_part_says_which_face_was_hit() {
    let (design, graph) = cabinet();
    let rest = graph.rest();
    let face = |ray: Ray| {
        let hit = graph.pick(&rest, ray).expect("the ray hits a part");
        let face = hit.part_face().expect("the hit is on a part");
        (part_name(&design, face).to_owned(), face.face)
    };
    // The front of the left door: its panel, sitting back in its frame,
    // and the frame's hinge stile.
    assert_eq!(
        face(from_front(200.0, 1100.0)),
        ("door_l.panel".to_owned(), Face::Front)
    );
    assert_eq!(
        face(from_front(30.0, 1100.0)),
        ("door_l.stile[hinge]".to_owned(), Face::Front)
    );
    assert_eq!(
        face(from_front(200.0, 150.0)),
        ("drawer[bottom].front".to_owned(), Face::Front)
    );
    // From the left, the right, above and behind.
    assert_eq!(
        face(toward(Vec3::new(0.0, 1000.0, -300.0), Vec3::X)),
        ("side_l".to_owned(), Face::Left)
    );
    assert_eq!(
        face(toward(Vec3::new(900.0, 1000.0, -300.0), Vec3::NEG_X)),
        ("side_r".to_owned(), Face::Right)
    );
    assert_eq!(
        face(toward(Vec3::new(450.0, 1700.0, -300.0), Vec3::NEG_Y)),
        ("top".to_owned(), Face::Top)
    );
    assert_eq!(
        face(toward(Vec3::new(450.0, 1000.0, -560.0), Vec3::Z)).1,
        Face::Back
    );
    // From inside the cabinet toward the left side: its inner face.
    let inside = Ray {
        origin: Vec3::new(450.0, 1000.0, -300.0),
        direction: Vec3::NEG_X,
    };
    assert_eq!(face(inside), ("side_l".to_owned(), Face::Right));
    // Hardware has no face.
    let pull = graph.pick(&rest, from_front(450.0, 139.0)).unwrap();
    assert!(matches!(pull.item, Item::Hardware(_)));
    assert_eq!(pull.part_face(), None);
}

#[test]
fn the_face_is_the_one_the_design_places_even_with_the_door_open() {
    let (design, graph) = cabinet();
    let posed = graph.pose(graph.scene_named("all_open").unwrap());
    // The left door swings open about its hinge on the left; seen from the
    // left it now shows its front, still named `front`.
    let hit = graph
        .pick(&posed, toward(Vec3::new(-100.0, 1100.0, 200.0), Vec3::X))
        .expect("the open door is hit");
    let face = hit.part_face().unwrap();
    assert!(part_name(&design, face).starts_with("door_l."), "{face:?}");
    assert_eq!(face.face, Face::Front);
}

#[test]
fn a_ray_into_the_open_cabinet_picks_the_door_bay() {
    let (design, graph) = cabinet();
    let posed = doors_hidden(&design, &graph);
    let region_name = |ray: Ray| {
        graph
            .pick_region(&posed, ray)
            .and_then(|hit| graph.region(hit.region))
            .map(|region| region.name.clone())
    };
    assert_eq!(
        region_name(from_front(200.0, 1100.0)).as_deref(),
        Some("door_bay[left]")
    );
    assert_eq!(
        region_name(from_front(700.0, 1100.0)).as_deref(),
        Some("door_bay[right]")
    );
    match graph.pick_target(&posed, from_front(200.0, 1100.0)) {
        Some(Target::Region(hit)) => {
            assert_eq!(graph.region(hit.region).unwrap().name, "door_bay[left]");
            // The ray reaches the bay at its front face.
            assert!((hit.distance - 5000.0).abs() < 0.01, "{hit:?}");
        }
        other => panic!("expected the door bay, found {other:?}"),
    }
    // The drawers are still there: their fronts win.
    match graph.pick_target(&posed, from_front(200.0, 150.0)) {
        Some(Target::Node(hit)) => assert_eq!(
            part_name(&design, hit.part_face().unwrap()),
            "drawer[bottom].front"
        ),
        other => panic!("expected the drawer front, found {other:?}"),
    }
    // The rail stands inside the bay: pointing at it picks it.
    match graph.pick_target(&posed, from_front(200.0, 1438.0)) {
        Some(Target::Node(hit)) => {
            assert_eq!(part_name(&design, hit.part_face().unwrap()), "rod")
        }
        other => panic!("expected the rod, found {other:?}"),
    }
    // Beside the cabinet there is nothing.
    assert_eq!(graph.pick_target(&posed, from_front(1200.0, 500.0)), None);
}

#[test]
fn a_part_in_front_of_a_region_wins_the_pick() {
    let (design, graph) = cabinet();
    let rest = graph.rest();
    match graph.pick_target(&rest, from_front(200.0, 1100.0)) {
        Some(Target::Node(hit)) => {
            let face = hit.part_face().unwrap();
            assert_eq!(part_name(&design, face), "door_l.panel");
            assert_eq!(face.face, Face::Front);
        }
        other => panic!("expected the door, found {other:?}"),
    }
    // On its own, the region pick sees the empty space in front of the
    // panel, which sits back in its frame: the door's own space. The panel
    // stands inside that space, so the target is the panel.
    let space = graph.pick_region(&rest, from_front(200.0, 1100.0)).unwrap();
    assert_eq!(graph.region(space.region).unwrap().name, "door_l.door");
    // The stile is flush with the front of the door's space: no empty
    // space in front of it at all.
    assert_eq!(graph.pick_region(&rest, from_front(30.0, 1100.0)), None);
}

#[test]
fn a_region_can_be_lit() {
    let (_, graph) = cabinet();
    let bay = graph
        .regions()
        .iter()
        .find(|region| region.name == "door_bay[left]")
        .unwrap();
    let light = graph.region_highlight(bay.id).unwrap();
    assert!(light.nodes.is_empty());
    assert_eq!(light.volumes, vec![bay.volume]);
    assert_eq!(bay.volume.min, Vec3::new(19.0, 538.0, -554.0));
    assert_eq!(bay.volume.max, Vec3::new(450.0, 1681.0, 0.0));
    // Every region the design has is there, the doors' own included.
    assert!(
        graph
            .regions()
            .iter()
            .any(|region| region.name == "door_l.door")
    );
}

#[test]
fn a_ray_between_the_sides_finds_them_facing_each_other() {
    let (design, graph) = cabinet();
    let posed = doors_hidden(&design, &graph);
    let facing = graph
        .facing(&posed, from_front(200.0, 1100.0))
        .expect("the two sides face each other");
    assert_eq!(facing.axis, Axis::Across);
    assert_eq!(part_name(&design, facing.first), "side_l");
    assert_eq!(facing.first.face, Face::Right);
    assert_eq!(part_name(&design, facing.second), "side_r");
    assert_eq!(facing.second.face, Face::Left);
    // 900mm wide, less two 19mm sides.
    assert_eq!(facing.clear, Length::from_mm(862));

    // With the doors shown the ray hits a door first: no gap.
    assert_eq!(graph.facing(&graph.rest(), from_front(200.0, 1100.0)), None);
    // Beside the cabinet: nothing faces anything.
    assert_eq!(graph.facing(&posed, from_front(1200.0, 1100.0)), None);
}

#[test]
fn between_the_shelf_and_the_top_the_narrow_gap_wins() {
    let (design, graph) = cabinet();
    let rest = graph.rest();
    // Above the shelf (1502..1521) and under the top (from 1681): 160mm
    // up, against 862mm across.
    let point = Point {
        across: Length::from_mm(450),
        up: Length::from_mm(1600),
        deep: Length::from_mm(300),
    };
    let facing = graph.facing_at(&rest, point).unwrap();
    assert_eq!(facing.axis, Axis::Up);
    assert_eq!(part_name(&design, facing.first), "shelf");
    assert_eq!(facing.first.face, Face::Top);
    assert_eq!(part_name(&design, facing.second), "top");
    assert_eq!(facing.second.face, Face::Bottom);
    assert_eq!(facing.clear, Length::from_mm(160));
    // Inside a part there is no gap.
    let in_the_shelf = point.with(Axis::Up, Length::from_mm(1510));
    assert_eq!(graph.facing_at(&rest, in_the_shelf), None);
}
