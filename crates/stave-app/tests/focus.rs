//! Focus mode, driven by the examples: holes drawn in turned parts, cut
//! faces tinted, a joint's steps traced and played, a hinge's members,
//! smallest gap, path and view from above.

use std::path::{Path, PathBuf};

use glam::Vec3;
use stave_app::focus::{self, FindError, Options, Target};
use stave_app::view::{self, Item, SceneGraph};
use stave_geom::clearance::{Closest, closest_for};
use stave_geom::collide::WorldSolid;
use stave_geom::sequence::{Moved, pair_members, pair_steps, trace};
use stave_geom::solid::Solid;
use stave_lang::units::Length;
use stave_model::solved::{CutKind, Part, SolvedDesign};
use stave_scene::{Look, Orbit, Projection};

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn graph(folder: &str) -> (stave_app::Opened, SceneGraph) {
    let opened = stave_app::open(&workspace().join(folder)).unwrap();
    let graph = view::scene(opened.design.as_ref().expect("it solves"));
    (opened, graph)
}

fn part<'d>(design: &'d SolvedDesign, name: &str) -> &'d Part {
    design
        .part_named(name)
        .unwrap_or_else(|| panic!("no part '{name}'"))
}

fn open_focus(
    opened: &stave_app::Opened,
    graph: &SceneGraph,
    name: &str,
    pair: Option<usize>,
) -> focus::Focus {
    let design = opened.design.as_ref().unwrap();
    let target = focus::find(design, name, pair).unwrap();
    focus::open(opened, design, graph, target).unwrap()
}

fn row<'f>(focus: &'f focus::Focus, label: &str) -> &'f str {
    focus
        .rows
        .iter()
        .find(|r| r.label == label)
        .map(|r| r.value.as_str())
        .unwrap_or_else(|| panic!("no row '{label}' in {:?}", focus.rows))
}

// ---------------------------------------------------------------------------
// Holes in turned parts and cut faces
// ---------------------------------------------------------------------------

#[test]
fn a_turned_leg_is_drawn_with_its_sockets_bored() {
    let (opened, _) = graph("examples/stool");
    let design = opened.design.as_ref().unwrap();
    let leg = part(design, "leg[front_left]");
    let drilled = stave_geom::cut_part(leg, &Solid::of(leg, design));
    // The same leg without its two sockets.
    let mut plain = leg.clone();
    plain
        .cuts
        .retain(|cut| !matches!(cut.kind, CutKind::Drill { .. }));
    let whole = stave_geom::cut_part(&plain, &Solid::of(&plain, design));
    let removed = whole.mesh.signed_volume() - drilled.mesh.signed_volume();
    // Two 22mm holes, 15mm deep from the blank's face, so a little under
    // 15mm into the round: each about π·11²·13.5 ≈ 5100 mm³.
    assert!(
        (8_000.0..12_500.0).contains(&removed),
        "the sockets remove {removed} mm³"
    );
    // Their walls and bottoms are cut faces; most of the leg isn't.
    let cut = drilled.mesh.vertices.iter().filter(|v| v.cut > 0.5).count();
    assert!(cut > 2 * 48 * 4, "{cut} cut vertices");
    let stock = drilled.mesh.vertices.iter().filter(|v| v.cut < 0.5).count();
    assert!(stock > cut);
    // The cut faces low on the leg are the sockets' walls, around the rung
    // holes 150mm and 185mm above the floor (y from -420 to 0).
    let low = drilled
        .mesh
        .vertices
        .iter()
        .filter(|v| v.cut > 0.5 && v.position[1] < -100.0)
        .map(|v| v.position[1]);
    let (lowest, highest) = low.fold((f32::INFINITY, f32::NEG_INFINITY), |(a, b), y| {
        (a.min(y), b.max(y))
    });
    assert!(
        lowest >= -281.0 - 0.01 && highest <= -224.0 + 0.01,
        "{lowest}..{highest}"
    );
}

#[test]
fn cut_faces_are_marked_and_the_stock_surface_is_not() {
    let (opened, _) = graph("examples/clothing_cabinet");
    let design = opened.design.as_ref().unwrap();
    let side = part(design, "side_l");
    let mesh = stave_geom::cut_part(side, &Solid::of(side, design)).mesh;
    // side_l is across 0..19: its outside face (x = 0) is the board's own
    // surface; its dados are cut into the inside face.
    let outside: Vec<f32> = mesh
        .vertices
        .iter()
        .filter(|v| v.position[0].abs() < 1e-3 && v.normal[0] < -0.9)
        .map(|v| v.cut)
        .collect();
    assert!(!outside.is_empty());
    assert!(outside.iter().all(|cut| *cut == 0.0));
    // The floor of the dados (9mm deep, facing inward at x = 10) is cut.
    let floors = mesh
        .vertices
        .iter()
        .filter(|v| (v.position[0] - 10.0).abs() < 1e-3 && v.normal[0] > 0.9)
        .count();
    assert!(floors > 0);
    assert!(
        mesh.vertices
            .iter()
            .filter(|v| (v.position[0] - 10.0).abs() < 1e-3 && v.normal[0] > 0.9)
            .all(|v| v.cut == 1.0)
    );
    // A round tenon on a turned foot is a cut face too.
    let foot = part(design, "foot[front_left]");
    let mesh = stave_geom::cut_part(foot, &Solid::of(foot, design)).mesh;
    assert!(mesh.vertices.iter().any(|v| v.cut == 1.0));
}

// ---------------------------------------------------------------------------
// Joint steps traced
// ---------------------------------------------------------------------------

#[test]
fn a_joint_traces_its_steps_forwards() {
    let (opened, _) = graph("examples/stool");
    let design = opened.design.as_ref().unwrap();
    let joint = design
        .joints
        .iter()
        .find(|j| j.name == "rung_sockets")
        .unwrap();
    let pair = &joint.pairs[0];
    let members = pair_members(pair);
    let parts: Vec<&Part> = members.iter().map(|id| design.part(*id).unwrap()).collect();
    let solids: Vec<Solid> = parts.iter().map(|p| Solid::of(p, design)).collect();
    let sequence: Vec<_> = parts
        .iter()
        .zip(&solids)
        .map(|(part, solid)| stave_geom::sequence::Member { part, solid })
        .collect();
    let steps = pair_steps(design, pair, &members, None).unwrap();
    let traced = trace(&sequence, &steps).unwrap();
    assert_eq!(traced.len(), 1);
    // The rung comes out of its 15mm socket along its length (across),
    // away from the leg: until its box is clear, 15mm and the contact
    // tolerance less.
    let Moved::Slide(vector) = traced[0].moved else {
        panic!("{:?}", traced[0].moved)
    };
    assert!((vector.length() - 14_950_000.0).abs() < 1.0, "{vector}");
    assert!(vector.x > 0.0 && vector.y.abs() < 1.0 && vector.z.abs() < 1.0);
    assert_eq!(traced[0].placement(1.0), traced[0].done);

    // The chidori: slide, slide on the diagonal, quarter turn.
    let (opened, _) = graph("examples/joint_samples");
    let design = opened.design.as_ref().unwrap();
    let joint = design.joints.iter().find(|j| j.name == "node").unwrap();
    let pair = &joint.pairs[0];
    let members = pair_members(pair);
    let parts: Vec<&Part> = members.iter().map(|id| design.part(*id).unwrap()).collect();
    let solids: Vec<Solid> = parts.iter().map(|p| Solid::of(p, design)).collect();
    let sequence: Vec<_> = parts
        .iter()
        .zip(&solids)
        .map(|(part, solid)| stave_geom::sequence::Member { part, solid })
        .collect();
    let steps = pair_steps(design, pair, &members, None).unwrap();
    let traced = trace(&sequence, &steps).unwrap();
    assert_eq!(traced.len(), 3);
    assert!(matches!(traced[0].moved, Moved::Slide(_)));
    assert!(matches!(traced[1].moved, Moved::Slide(_)));
    let Moved::Turn { angle, .. } = traced[2].moved else {
        panic!()
    };
    assert!((angle.abs() - std::f64::consts::FRAC_PI_2).abs() < 1e-9);
}

#[test]
fn focus_on_a_socket_plays_the_rung_in() {
    let (opened, graph) = graph("examples/stool");
    let found = open_focus(&opened, &graph, "rung_sockets", None);
    assert_eq!(found.title, "rung[front] into leg[front_left] (socket)");
    assert_eq!(
        found.context,
        "Stool · Joint focus · rung[front] into leg[front_left] (socket)"
    );
    assert_eq!(found.heading, "Socket");
    assert_eq!(
        found.description.as_deref(),
        Some(
            "A socket: a round hole that takes the whole end of a round part, such as a clothes rod between two cabinet sides."
        )
    );
    assert_eq!(row(&found, "Goes in"), "rung[front]");
    assert_eq!(row(&found, "Receives it"), "leg[front_left]");
    assert_eq!(row(&found, "Hole"), "22 mm across, 15 mm deep");
    assert_eq!(row(&found, "Fit"), "touches, nothing overlaps");
    let insert = found.insert.as_ref().unwrap();
    assert_eq!(insert.text, "insert: [push(piece, along: length)]");
    assert!(
        insert.source.ends_with("socket.stave:10"),
        "{}",
        insert.source
    );
    assert!(found.assembles());
    assert_eq!(found.step_count(), 1);

    let rest = graph.scenes.first().unwrap();
    let options = Options::default();
    let frame = found.frame(&graph, rest, 0.6, &options);
    assert_eq!(
        frame.step.as_deref(),
        Some("Step 1 of 1 · push(rung[front], along: length)")
    );
    assert_eq!(frame.status, "rung[front] is 6 mm from home");
    // The rung is 6mm out (along +x), the leg where it is.
    let design = opened.design.as_ref().unwrap();
    let rung = graph
        .node_of(Item::Part(part(design, "rung[front]").id))
        .unwrap();
    let leg = graph
        .node_of(Item::Part(part(design, "leg[front_left]").id))
        .unwrap();
    let moved = frame.posed.transforms[rung.index()].transform_point3(Vec3::ZERO);
    assert!(
        (moved.x - 5.98).abs() < 0.05 && moved.y.abs() < 1e-3,
        "{moved}"
    );
    assert_eq!(
        frame.posed.transforms[leg.index()].transform_point3(Vec3::ZERO),
        Vec3::ZERO
    );
    // The leg is see-through, the rung as it is, the seat a ghost; and the
    // rung's home is outlined, dashed.
    let seat = graph.node_of(Item::Part(part(design, "seat").id)).unwrap();
    assert_eq!(frame.looks[leg.index()], Look::XRay);
    assert_eq!(frame.looks[rung.index()], Look::Normal);
    assert_eq!(frame.looks[seat.index()], Look::Ghost);
    assert!(!frame.overlay.lines.is_empty());
    // Together: home, nothing outlined. Toggles off: nothing faded.
    let together = found.frame(&graph, rest, 1.0, &options);
    assert_eq!(together.status, "together: rung[front] is home");
    assert!(together.overlay.lines.is_empty());
    let plain = found.frame(
        &graph,
        rest,
        0.0,
        &Options {
            xray: false,
            fade_others: false,
            tint_cut_faces: false,
        },
    );
    assert!(plain.looks.iter().all(|look| *look == Look::Normal));
    assert_eq!(plain.status, "rung[front] is 15 mm from home");
    // Framed on the rung and its leg, not the whole stool.
    let zone = found.zone().unwrap();
    assert!(zone.size().max_element() < 300.0, "{zone:?}");
}

#[test]
fn focus_finds_joints_pairs_and_hardware_by_name() {
    let (opened, graph) = graph("examples/clothing_cabinet");
    let design = opened.design.as_ref().unwrap();
    assert!(matches!(
        focus::find(design, "shelf_dados", Some(2)),
        Ok(Target::Joint { pair: 2, .. })
    ));
    assert_eq!(
        focus::find(design, "shelf_dados", Some(9)),
        Err(FindError::NoPair {
            name: "shelf_dados".to_owned(),
            count: 4,
            pair: 9
        })
    );
    assert!(matches!(
        focus::find(design, "door_l.hinges[upper]", None),
        Ok(Target::Hardware(_))
    ));
    let missing = focus::find(design, "no_such_thing", None).unwrap_err();
    assert!(
        missing
            .to_string()
            .starts_with("no joint or hardware is called 'no_such_thing'")
    );
    // What a selection offers: a side's joints, a hinge itself.
    let side = part(design, "side_l");
    let targets = focus::targets_of(design, Item::Part(side.id));
    let labels: Vec<&str> = targets.iter().map(|(_, l)| l.as_str()).collect();
    assert!(labels.contains(&"shelf into side_l (dado)"), "{labels:?}");
    assert!(labels.contains(&"rod into side_l (socket)"), "{labels:?}");
    let hinge = design
        .hardware
        .iter()
        .find(|h| h.name == "door_l.hinges[upper]")
        .unwrap();
    let targets = focus::targets_of(design, Item::Hardware(hinge.id));
    assert_eq!(targets.len(), 1);
    assert_eq!(targets[0].1, "door_l.hinges[upper] (euro_hinge)");
    let _ = graph;
}

#[test]
fn focus_on_the_chidori_turns_the_lock_last() {
    let (opened, graph) = graph("examples/joint_samples");
    let found = open_focus(&opened, &graph, "node", None);
    assert_eq!(found.step_count(), 3);
    assert_eq!(found.heading, "Chidori");
    let rest = graph.scenes.first().unwrap();
    let frame = found.frame(&graph, rest, 5.0 / 6.0, &Options::default());
    assert_eq!(
        frame.step.as_deref(),
        Some("Step 3 of 3 · rotate(stick_z, by: 90deg, about: length)")
    );
    assert_eq!(frame.status, "stick_z has 45° still to turn");
    let start = found.frame(&graph, rest, 0.0, &Options::default());
    assert_eq!(
        start.step.as_deref(),
        Some("Step 1 of 3 · push(stick_y, along: thickness)")
    );
}

// ---------------------------------------------------------------------------
// Hardware
// ---------------------------------------------------------------------------

#[test]
fn focus_on_a_cup_hinge_names_its_members_and_measures_its_swing() {
    let (opened, graph) = graph("examples/clothing_cabinet");
    let found = open_focus(&opened, &graph, "door_l.hinges[upper]", None);
    assert_eq!(found.heading, "How a cup hinge works");
    assert!(
        found
            .description
            .as_deref()
            .unwrap()
            .starts_with("A concealed 35mm cup hinge for an overlay door")
    );
    let members: Vec<(u8, &str)> = found.members.iter().map(|m| (m.number, m.name)).collect();
    assert_eq!(
        members,
        [(1, "plate"), (2, "arm"), (3, "links"), (4, "cup")]
    );
    assert!(
        found.members[2]
            .caption
            .starts_with("The two links: steel bars")
    );
    assert_eq!(row(&found, "Opens to"), "110°");
    assert_eq!(row(&found, "Closest to the side"), "3.5 mm");
    assert_eq!(row(&found, "Closest to anything"), "top: 1.5 mm");
    assert_eq!(row(&found, "Swing check"), "clear all the way");
    let captions: Vec<&str> = found.key_poses.iter().map(|p| p.caption.as_str()).collect();
    assert_eq!(captions, ["0°", "45°", "110°"]);
    assert!(!found.assembles());
    let rest = graph.scenes.first().unwrap();
    let frame = found.frame(&graph, rest, 45.0 / 110.0, &Options::default());
    assert_eq!(frame.status, "open 45°");
    // Every member is numbered on the drawing; the door's edge is dotted.
    let mut numbers: Vec<u8> = frame.overlay.labels.iter().map(|l| l.number).collect();
    numbers.sort_unstable();
    assert_eq!(numbers, [1, 2, 3, 4]);
    assert!(frame.overlay.lines.len() > 10);
    // Where it comes closest to the side is marked with the gap.
    let notes: Vec<&str> = frame
        .overlay
        .notes
        .iter()
        .map(|n| n.text.as_str())
        .collect();
    assert_eq!(notes, ["3.5 mm"]);
    assert_eq!(
        frame
            .overlay
            .lines
            .iter()
            .filter(|l| l.kind == stave_scene::LineKind::Gap)
            .count(),
        1
    );
    // The hinge, its door stile and the side are drawn; the rest fade.
    let design = opened.design.as_ref().unwrap();
    let top = graph.node_of(Item::Part(part(design, "top").id)).unwrap();
    let side = graph
        .node_of(Item::Part(part(design, "side_l").id))
        .unwrap();
    assert_eq!(frame.looks[top.index()], Look::Ghost);
    assert_eq!(frame.looks[side.index()], Look::Normal);
    // From above: cut just above the arm, framed on the hinge and path.
    let plan = found.plan().unwrap();
    let hinge = design
        .hardware
        .iter()
        .find(|h| h.name == "door_l.hinges[upper]")
        .unwrap();
    let arm_top = stave_geom::render::point(hinge.mechanism.as_ref().unwrap().arm.bounds.max()).y;
    assert!((plan.section.offset - (arm_top + 0.5)).abs() < 1e-3);
    assert_eq!(plan.section.normal, Vec3::Y);
    let orbit = Orbit::plan(plan.bounds, 1.5, 0.0);
    assert_eq!(orbit.projection, Projection::Flat);
    assert!(orbit.backward().y > 0.999);
    for corner in plan.bounds.corners() {
        let p = orbit.view_projection(1.5).project_point3(corner);
        assert!(p.x.abs() <= 1.0 && p.y.abs() <= 1.0, "{p}");
    }
}

#[test]
fn the_smallest_gap_over_a_swing_is_within_the_bound() {
    let (opened, _) = graph("examples/clothing_cabinet");
    let design = opened.design.as_ref().unwrap();
    let door = design
        .assemblies
        .iter()
        .find(|a| a.name == "door_l")
        .unwrap();
    let solid = |p: &Part| WorldSolid::of_part(p, &Solid::of(p, design));
    let moving: Vec<WorldSolid> = door
        .parts
        .iter()
        .map(|id| solid(design.part(*id).unwrap()))
        .collect();
    let top = solid(part(design, "top"));
    let motion = door.motion.unwrap();
    let Closest { gap, at, .. } =
        closest_for(&moving, std::slice::from_ref(&top), &motion).unwrap();
    // The door's top edge passes under the top's overhang 1.5mm below it,
    // all the way round.
    assert_eq!(gap, Length::from_nm(1_500_000));
    let found = closest_for(&moving, std::slice::from_ref(&top), &motion).unwrap();
    let (on_door, on_top) = found.points.unwrap();
    assert!((on_door.distance(on_top) - 1_500_000.0).abs() < 1_000.0);
    assert!((0.0..=110.0_f64.to_radians() + 1e-9).contains(&at));
    // A drawer slides out past the dividers above and below it, exactly.
    let drawer = design
        .assemblies
        .iter()
        .find(|a| a.name == "drawer[top]")
        .unwrap();
    let moving: Vec<WorldSolid> = drawer
        .parts
        .iter()
        .map(|id| solid(design.part(*id).unwrap()))
        .collect();
    let divider = solid(part(design, "divider"));
    let found = closest_for(
        &moving,
        std::slice::from_ref(&divider),
        &drawer.motion.unwrap(),
    )
    .unwrap();
    assert!(found.gap > Length::ZERO);
}

#[test]
fn focus_on_a_slide_follows_the_drawer() {
    let (opened, graph) = graph("examples/clothing_cabinet");
    let found = open_focus(&opened, &graph, "drawer[top].slides[left]", None);
    assert_eq!(found.heading, "How a drawer slide works");
    assert_eq!(row(&found, "Pulls out"), "500 mm");
    let members: Vec<&str> = found.members.iter().map(|m| m.name).collect();
    assert_eq!(members, ["cabinet_member", "drawer_member"]);
    assert!(found.plan().is_none());
    let rest = graph.scenes.first().unwrap();
    let frame = found.frame(&graph, rest, 0.5, &Options::default());
    assert_eq!(frame.status, "out 250 mm");
}

/// Times focus mode's posing; run with
/// `cargo test --release -p stave-app --test focus -- --ignored --nocapture`.
#[test]
#[ignore = "a timing, not a check"]
fn focus_poses_at_interactive_rates() {
    for (folder, name) in [
        ("examples/clothing_cabinet", "door_l.hinges[upper]"),
        ("examples/stool", "rung_sockets"),
        ("examples/joint_samples", "node"),
    ] {
        let (opened, graph) = graph(folder);
        let started = std::time::Instant::now();
        let found = open_focus(&opened, &graph, name, None);
        let opening = started.elapsed();
        let rest = graph.scenes.first().unwrap();
        let started = std::time::Instant::now();
        for k in 0..1000_u16 {
            let _ = found.frame(&graph, rest, f32::from(k) / 1000.0, &Options::default());
        }
        let posing = started.elapsed() / 1000;
        println!("{name}: opening focus {opening:?}, one frame posed {posing:?}");
    }
}

#[test]
fn focus_on_a_knob_names_the_knob_and_its_screw() {
    let (opened, graph) = graph("examples/clothing_cabinet");
    let found = open_focus(&opened, &graph, "door_l.handle", None);
    assert_eq!(found.heading, "How a pull is fixed");
    let members: Vec<(u8, &str)> = found.members.iter().map(|m| (m.number, m.name)).collect();
    assert_eq!(members, [(1, "knob"), (2, "screw")]);
    let rest = graph.scenes.first().unwrap();
    let frame = found.frame(&graph, rest, 0.0, &Options::default());
    let labels: Vec<&stave_scene::Label> = frame.overlay.labels.iter().collect();
    assert_eq!(labels.len(), 2);
    // The knob's number is in front of the screw's (the knob stands off
    // the door's front face, +z).
    assert!(labels[0].at.z > labels[1].at.z);
}

/// Every edge of a mesh, by the positions of its ends, and how many
/// triangles use it: a watertight mesh uses each exactly twice.
fn open_edges(mesh: &stave_geom::Mesh) -> Vec<([u32; 3], [u32; 3], usize)> {
    use std::collections::BTreeMap;
    let key = |p: Vec3| p.to_array().map(|c| (c + 0.0).to_bits());
    let mut counts: BTreeMap<([u32; 3], [u32; 3]), usize> = BTreeMap::new();
    for [a, b, c] in mesh.triangles() {
        if (b - a).cross(c - a).length_squared() < 1e-12 {
            continue;
        }
        for (p, q) in [(a, b), (b, c), (c, a)] {
            let (p, q) = (key(p), key(q));
            let edge = if p <= q { (p, q) } else { (q, p) };
            *counts.entry(edge).or_default() += 1;
        }
    }
    counts
        .into_iter()
        .filter(|(_, count)| *count != 2)
        .map(|((p, q), count)| (p, q, count))
        .collect()
}

#[test]
fn drilled_meshes_are_watertight() {
    let (opened, _) = graph("examples/stool");
    let design = opened.design.as_ref().unwrap();
    for name in [
        "leg[front_left]",
        "leg[front_right]",
        "leg[back_left]",
        "leg[back_right]",
        "seat",
    ] {
        let part = part(design, name);
        let mesh = stave_geom::cut_part(part, &Solid::of(part, design)).mesh;
        let open = open_edges(&mesh);
        assert!(
            open.is_empty(),
            "{name}: {} open edges, e.g. {:?}",
            open.len(),
            open.first()
        );
    }
}
