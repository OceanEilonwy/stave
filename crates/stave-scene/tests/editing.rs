//! The scene for editing, on the solved clothing cabinet built from
//! std:casework assemblies (examples/clothing_cabinet_assemblies, read
//! from disk and changed in memory): the ghosts of a candidate, the parts
//! covering what the user works on, where grips sit, and regions that
//! move with their assembly.

use std::io;
use std::path::{Path, PathBuf};
use std::time::Instant;

use glam::Vec3;
use stave_lang::placement::{Axis, Face};
use stave_lang::units::Length;
use stave_model::solved::SolvedDesign;
use stave_modules::{DiskFiles, FileSource};
use stave_scene::{
    BuildOptions, FaceOf, GhostChange, GhostVerdict, Ghosts, GripOf, GripPlace, Item, Look, Nudged,
    Orbit, Projection, SceneGraph, Subject,
};

const EXAMPLE: &str = "clothing_cabinet_assemblies";

/// A file of the example, by name, and how its text is changed.
type Edit<'a> = (&'a str, &'a dyn Fn(&str) -> String);

/// The files on disk, with some files' text replaced by a function of it.
struct Changed<'a> {
    edits: &'a [Edit<'a>],
}

impl FileSource for Changed<'_> {
    fn list_stave_files(&self, folder: &Path) -> io::Result<Vec<PathBuf>> {
        DiskFiles.list_stave_files(folder)
    }

    fn read(&self, path: &Path) -> io::Result<String> {
        let text = DiskFiles.read(path)?;
        let changed = self
            .edits
            .iter()
            .find(|(name, _)| path.ends_with(Path::new(EXAMPLE).join(name)));
        Ok(match changed {
            Some((_, edit)) => edit(&text),
            None => text,
        })
    }

    fn canonical(&self, folder: &Path) -> io::Result<PathBuf> {
        DiskFiles.canonical(folder)
    }
}

fn solve_with(edits: &[Edit<'_>]) -> SolvedDesign {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples")
        .join(EXAMPLE);
    let modules =
        stave_modules::ModuleSet::load_with(&path, &Changed { edits }).expect("the example loads");
    let (defs, _) = stave_model::resolve(&modules);
    let (model, _) = stave_model::instance::instantiate(defs, modules.root);
    let (design, diagnostics) = stave_solve::solve(&model);
    design.unwrap_or_else(|| panic!("the design solves: {diagnostics:?}"))
}

fn base() -> (SolvedDesign, SceneGraph) {
    let design = solve_with(&[]);
    let graph = SceneGraph::build(&design, BuildOptions::default());
    (design, graph)
}

/// The cabinet with a second shelf in the hanging space, below the rail.
fn with_shelf() -> SolvedDesign {
    let add = |text: &str| {
        format!(
            "{text}\nassembly shelf_2 : casework.Shelf {{\n  space:       shelf.below\n  \
             sides:       [side_l, side_r]\n  panel_stock: oak_19\n  headroom:    700mm\n}}\n"
        )
    };
    solve_with(&[("carcass.stave", &add)])
}

/// The cabinet 100mm wider.
fn wider() -> SolvedDesign {
    let widen = |text: &str| {
        let widened = text.replace(
            "param cabinet_width  = 900mm",
            "param cabinet_width  = 1000mm",
        );
        assert_ne!(widened, text, "the width param is where the test expects");
        widened
    };
    solve_with(&[("cabinet.stave", &widen)])
}

fn ghost_names(ghosts: &Ghosts) -> Vec<(String, GhostChange)> {
    ghosts
        .nodes()
        .map(|(ghost, node)| (node.name.clone(), ghost.change))
        .collect()
}

#[test]
fn adding_a_shelf_ghosts_exactly_the_shelf_and_leaves_the_base_alone() {
    let (design, graph) = base();
    let before = graph.clone();
    let rest_before = graph.rest();
    let candidate = with_shelf();
    let started = Instant::now();
    let ghosts = Ghosts::new(
        &design,
        &graph,
        &candidate,
        BuildOptions::default(),
        GhostVerdict::Checking,
    );
    let took = started.elapsed();
    eprintln!("ghosts built in {took:?}");

    let names = ghost_names(&ghosts);
    assert!(!names.is_empty(), "the shelf has ghosts");
    for (name, change) in &names {
        assert!(
            name.starts_with("shelf_2"),
            "only the new shelf's nodes are ghosts, not {name}"
        );
        assert_eq!(*change, GhostChange::Added, "{name} is new");
    }
    // Every node the candidate draws for the shelf is a ghost.
    let shelf_nodes = ghosts
        .graph()
        .nodes
        .iter()
        .filter(|node| node.name.starts_with("shelf_2"))
        .count();
    assert_eq!(names.len(), shelf_nodes);
    assert!(names.iter().any(|(name, _)| name == "shelf_2.board"));
    assert!(ghosts.removed().is_empty());
    assert!(ghosts.stands_for().is_empty());
    assert!(
        ghosts
            .base_looks(&graph)
            .iter()
            .all(|look| *look == Look::Normal),
        "every base node is drawn as it is"
    );

    // The ghost's verdict changes without touching anything else.
    let mut ghosts = ghosts;
    assert_eq!(ghosts.verdict(), GhostVerdict::Checking);
    ghosts.set_verdict(GhostVerdict::Fits);
    assert_eq!(ghosts.verdict(), GhostVerdict::Fits);
    assert_ne!(GhostVerdict::Fits.tint(), GhostVerdict::Refused.tint());

    // The ghosts are posed alone: only their nodes are shown.
    let posed = ghosts.pose("rest", 0.0);
    let shown = posed.visible().iter().filter(|shown| **shown).count();
    assert_eq!(shown, names.len());

    // Removing the candidate leaves the base scene exactly as it was.
    drop(ghosts);
    assert_eq!(graph, before);
    assert_eq!(graph.rest(), rest_before);
}

#[test]
fn widening_the_cabinet_marks_the_parts_that_change() {
    let (design, graph) = base();
    let candidate = wider();
    let ghosts = Ghosts::new(
        &design,
        &graph,
        &candidate,
        BuildOptions::default(),
        GhostVerdict::Checking,
    );
    let names = ghost_names(&ghosts);
    let changed = |name: &str| {
        names
            .iter()
            .any(|(ghost, change)| ghost == name && *change == GhostChange::Changed)
    };
    for name in ["base", "top", "side_r", "divider.panel", "shelf.board"] {
        assert!(changed(name), "{name} changes: {names:?}");
    }
    assert!(
        !changed("side_l"),
        "the left side stays where it is: {names:?}"
    );
    assert!(
        names
            .iter()
            .all(|(_, change)| *change == GhostChange::Changed),
        "nothing is added"
    );
    // A changed ghost stands for the base node of the same path, shown
    // at its new size.
    for (ghost, node) in ghosts.nodes() {
        let replaced = ghost.replaces.and_then(|id| graph.node(id)).unwrap();
        assert_eq!(replaced.name, node.name);
    }
    let base_node = graph.node_of_name("base");
    let ghost_base = ghosts
        .nodes()
        .find(|(_, node)| node.name == "base")
        .map(|(_, node)| node.geometry.bounds.size().x)
        .unwrap();
    let old = graph.node(base_node).unwrap().geometry.bounds.size().x;
    assert!(
        (ghost_base - old - 100.0).abs() < 0.01,
        "{old} → {ghost_base}"
    );
    // The base nodes under changed ghosts are faded, the rest as usual.
    let looks = ghosts.base_looks(&graph);
    assert_eq!(looks[base_node.index()], Look::Ghost);
    assert_eq!(looks[graph.node_of_name("side_l").index()], Look::Normal);
}

trait ByName {
    fn node_of_name(&self, name: &str) -> stave_scene::NodeId;
}

impl ByName for SceneGraph {
    fn node_of_name(&self, name: &str) -> stave_scene::NodeId {
        self.node_ids()
            .find(|id| self.node(*id).unwrap().name == name)
            .unwrap_or_else(|| panic!("no node {name}"))
    }
}

fn region_named(graph: &SceneGraph, name: &str) -> stave_model::solved::RegionId {
    graph
        .regions()
        .iter()
        .find(|region| region.name == name)
        .unwrap_or_else(|| panic!("no region {name}"))
        .id
}

/// A camera looking at the cabinet straight on, from in front (yaw 0) or
/// from behind (yaw π).
fn camera(graph: &SceneGraph, yaw: f32, projection: Projection) -> Orbit {
    let bounds = graph.bounds(&graph.rest()).unwrap();
    Orbit {
        target: bounds.centre(),
        yaw,
        pitch: 0.0,
        distance: 4000.0,
        projection,
    }
}

fn names_of(graph: &SceneGraph, nodes: &[stave_scene::NodeId]) -> Vec<String> {
    nodes
        .iter()
        .map(|id| graph.node(*id).unwrap().name.clone())
        .collect()
}

#[test]
fn the_door_covers_its_bay_from_the_front_and_not_from_behind() {
    let (_, graph) = base();
    let rest = graph.rest();
    let bay = region_named(&graph, "door_bay[left]");
    for projection in [Projection::Perspective, Projection::Flat] {
        let front = graph.covering(
            &rest,
            &camera(&graph, 0.0, projection),
            Subject::Region(bay),
        );
        let names = names_of(&graph, &front);
        for part in ["door_l.stile[hinge]", "door_l.panel"] {
            assert!(
                names.iter().any(|name| name == part),
                "from the front the left door ({part}) covers its bay: {names:?}"
            );
        }
        assert!(
            !names.iter().any(|name| name.starts_with("door_r.")),
            "the right door does not: {names:?}"
        );
        assert!(
            !names
                .iter()
                .any(|name| name == "side_l" || name == "divider.panel" || name == "base"),
            "the walls the bay only touches don't cover it: {names:?}"
        );

        let behind = graph.covering(
            &rest,
            &camera(&graph, std::f32::consts::PI, projection),
            Subject::Region(bay),
        );
        let names = names_of(&graph, &behind);
        assert!(
            !names.iter().any(|name| name.starts_with("door_")),
            "from behind no door covers it: {names:?}"
        );
        assert!(
            names.iter().any(|name| name.starts_with("back.")),
            "from behind the back covers it: {names:?}"
        );
    }
    // With the doors open they no longer cover the bay.
    let open = graph.pose(graph.scene_named("all_open").unwrap());
    let front = graph.covering(
        &open,
        &camera(&graph, 0.0, Projection::Perspective),
        Subject::Region(bay),
    );
    let names = names_of(&graph, &front);
    // (Its hinge stile, swung forward at the bay's edge, may still hide
    // a sliver of it.)
    for part in ["door_l.panel", "door_l.stile[latch]", "door_l.handle knob"] {
        assert!(
            !names.iter().any(|name| name == part),
            "an open door ({part}) doesn't cover the bay: {names:?}"
        );
    }
}

#[test]
fn a_part_is_covered_by_what_stands_in_front_of_it() {
    let (design, graph) = base();
    let rest = graph.rest();
    let divider = design.part_named("divider.panel").unwrap().id;
    let front = graph.covering(
        &rest,
        &camera(&graph, 0.0, Projection::Perspective),
        Subject::Part(divider),
    );
    let names = names_of(&graph, &front);
    assert!(
        !names.iter().any(|name| name == "divider.panel"),
        "a part doesn't cover itself"
    );
    assert!(
        names
            .iter()
            .any(|name| name.starts_with("door_") || name.starts_with("drawer[")),
        "the fronts cover the divider's front edge: {names:?}"
    );
}

#[test]
fn a_grip_on_the_dividers_height_sits_between_the_base_and_the_divider() {
    let (design, graph) = base();
    let divider = design
        .assemblies()
        .iter()
        .find(|assembly| assembly.name == "divider")
        .unwrap()
        .id;
    let base = design.part_named("base").unwrap();
    let panel = design.part_named("divider.panel").unwrap();
    let place = graph
        .grip_place(GripOf::Length {
            assembly: divider,
            axis: Axis::Up,
            param: "height",
            lengths: &[("height", Length::from_mm(500))],
            nudged: None,
        })
        .expect("the divider's height has a place");
    let GripPlace::Length {
        from,
        to,
        start,
        handle,
    } = place
    else {
        panic!("a length grip: {place:?}");
    };
    assert_eq!(from.of, FaceOf::Part(base.id));
    assert_eq!(from.face, Face::Top);
    assert_eq!(from.at, base.drawn_box().max().get(Axis::Up));
    assert_eq!(to.of, FaceOf::Part(panel.id));
    assert_eq!(to.face, Face::Bottom);
    assert_eq!(to.at, panel.drawn_box().min().get(Axis::Up));
    assert_eq!(
        to.at.checked_sub(from.at),
        Some(Length::from_mm(500)),
        "500mm apart"
    );
    assert_eq!(handle.direction, Vec3::Y, "dragging up makes it taller");
    assert!((handle.point.y - start.y - 500.0).abs() < 0.01);
    assert!((start.y - 19.0).abs() < 0.01, "on the base's top: {start}");
    // Over the middle of the divider.
    let middle = stave_geom::render::world_box(panel.drawn_box()).centre();
    assert!((handle.point.x - middle.x).abs() < 0.01);
    assert!((handle.point.z - middle.z).abs() < 0.01);

    // A length nothing is measured by has no place.
    assert_eq!(
        graph.grip_place(GripOf::Length {
            assembly: divider,
            axis: Axis::Up,
            param: "height",
            lengths: &[("height", Length::from_mm(123))],
            nudged: None,
        }),
        None
    );
    // Nor a param the assembly isn't given.
    assert_eq!(
        graph.grip_place(GripOf::Length {
            assembly: divider,
            axis: Axis::Up,
            param: "depth",
            lengths: &[("height", Length::from_mm(500))],
            nudged: None,
        }),
        None
    );
}

/// The cabinet with both drawers' given gap changed.
fn with_drawer_gap(param: &str, value: &str) -> SolvedDesign {
    let line = format!("  pull_kind:    hw.bar_pull_160\n  {param}: {value}\n");
    let set = |text: &str| {
        let changed = text.replace("  pull_kind:    hw.bar_pull_160\n", &line);
        assert_ne!(changed, text, "the drawers are where the test expects");
        changed
    };
    solve_with(&[("drawers.stave", &set)])
}

#[test]
fn two_gaps_of_one_value_are_told_apart_by_which_moves() {
    let (design, graph) = base();
    let drawer = design
        .assemblies()
        .iter()
        .find(|assembly| assembly.name == "drawer[top]")
        .unwrap()
        .id;
    let gap = Length::from_mm(15);
    let lengths = [("bottom_gap", gap), ("top_gap", gap)];
    let grip = |param, nudged| GripOf::Length {
        assembly: drawer,
        axis: Axis::Up,
        param,
        lengths: &lengths,
        nudged,
    };
    // Both 15mm: a pair 15mm apart could be either's, so neither has a
    // place until a nudged scene says which faces each one moves.
    assert_eq!(graph.grip_place(grip("top_gap", None)), None);
    assert_eq!(graph.grip_place(grip("bottom_gap", None)), None);

    let in_drawer = |of: FaceOf| match of {
        FaceOf::Part(id) => design
            .part(id)
            .is_some_and(|part| part.name.starts_with("drawer[top].")),
        FaceOf::Region(_) => false,
    };

    let top_nudged =
        SceneGraph::build(&with_drawer_gap("top_gap", "20mm"), BuildOptions::default());
    let place = graph
        .grip_place(grip(
            "top_gap",
            Some(Nudged {
                graph: &top_nudged,
                value: Length::from_mm(20),
            }),
        ))
        .expect("the top gap has a place");
    let GripPlace::Length {
        from, to, handle, ..
    } = place
    else {
        panic!("a length grip: {place:?}");
    };
    assert!(in_drawer(to.of), "the box's top moves: {to:?}");
    assert_eq!(to.face, Face::Top);
    assert_eq!(from.face, Face::Bottom, "measured up to what is above");
    assert_eq!(from.at.checked_sub(to.at), Some(gap));
    assert_eq!(handle.direction, -Vec3::Y, "dragging down widens it");

    let bottom_nudged = SceneGraph::build(
        &with_drawer_gap("bottom_gap", "20mm"),
        BuildOptions::default(),
    );
    let place = graph
        .grip_place(grip(
            "bottom_gap",
            Some(Nudged {
                graph: &bottom_nudged,
                value: Length::from_mm(20),
            }),
        ))
        .expect("the bottom gap has a place");
    let GripPlace::Length {
        from, to, handle, ..
    } = place
    else {
        panic!("a length grip: {place:?}");
    };
    assert!(in_drawer(to.of), "the box's bottom moves: {to:?}");
    assert_eq!(to.face, Face::Bottom);
    assert_eq!(to.at.checked_sub(from.at), Some(gap));
    assert_eq!(handle.direction, Vec3::Y, "dragging up widens it");

    // A nudged scene that moves nothing 20mm apart names no faces.
    assert_eq!(
        graph.grip_place(grip(
            "top_gap",
            Some(Nudged {
                graph: &graph,
                value: Length::from_mm(20),
            }),
        )),
        None
    );
}

#[test]
fn a_shares_grip_sits_on_the_boundary_and_an_envelopes_on_its_face() {
    let (_, graph) = base();
    let bottom = region_named(&graph, "drawer_bay[bottom]");
    let top = region_named(&graph, "drawer_bay[top]");
    let place = graph
        .grip_place(GripOf::Share {
            low: bottom,
            high: top,
            axis: Axis::Up,
        })
        .unwrap();
    let GripPlace::Share { at, handle } = place else {
        panic!("a share grip: {place:?}");
    };
    let low = graph.region(bottom).unwrap().bounds;
    assert_eq!(at, low.max().get(Axis::Up));
    assert_eq!(handle.direction, Vec3::Y);
    let volume = graph.region(bottom).unwrap().volume;
    assert!((handle.point.x - volume.centre().x).abs() < 0.01);
    // Neighbours given the wrong way round don't touch there.
    assert_eq!(
        graph.grip_place(GripOf::Share {
            low: top,
            high: bottom,
            axis: Axis::Up,
        }),
        None
    );

    // The handle of a size on the face it moves: here the opening's right
    // face, which stands in for an envelope box in this design.
    let opening = region_named(&graph, "front_opening");
    let place = graph
        .grip_place(GripOf::Envelope {
            region: opening,
            face: Face::Right,
        })
        .unwrap();
    let GripPlace::Envelope { face, handle } = place else {
        panic!("an envelope grip: {place:?}");
    };
    let volume = graph.region(opening).unwrap().volume;
    assert_eq!(face.face, Face::Right);
    assert_eq!(handle.direction, Vec3::X);
    assert!((handle.point.x - volume.max.x).abs() < 0.01);
    assert!((handle.point.y - volume.centre().y).abs() < 0.01);
    assert!((handle.point.z - volume.centre().z).abs() < 0.01);
}

#[test]
fn a_region_a_component_exposes_moves_with_its_assembly() {
    let (_, graph) = base();
    let door = region_named(&graph, "door_l.door");
    let bay = region_named(&graph, "door_bay[left]");
    let rest = graph.rest();
    let open = graph.pose(graph.scene_named("all_open").unwrap());
    let at_rest = graph.region_volume(&rest, door).unwrap();
    assert_eq!(at_rest, graph.region(door).unwrap().volume);
    let swung = graph.region_volume(&open, door).unwrap();
    assert_ne!(swung, at_rest, "the door's space swings open with the door");
    // It goes where the door's parts go.
    let stile = graph
        .node_ids()
        .find(|id| {
            let node = graph.node(*id).unwrap();
            node.name.starts_with("door_l.") && matches!(node.item, Item::Part(_))
        })
        .unwrap();
    assert_eq!(open.region(door), open.transforms()[stile.index()]);
    // A declared region stays put.
    assert_eq!(
        graph.region_volume(&open, bay),
        graph.region_volume(&rest, bay)
    );
    let light = graph.region_highlight_posed(&open, door).unwrap();
    assert_eq!(light.volumes, vec![swung]);
}
