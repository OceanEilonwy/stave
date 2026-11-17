//! Exploded views of every example: what moves, and what may stay.

use std::path::Path;

use glam::Vec3;
use stave_model::solved::{PartId, SolvedDesign, StepKind};
use stave_scene::{BuildOptions, Item, SceneGraph};

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

/// Every example that has a design (the Japanese joinery folder is a
/// library).
const EXAMPLES: [&str; 8] = [
    "bookshelf",
    "box",
    "clothing_cabinet",
    "joint_samples",
    "nightstand",
    "side_table",
    "stool",
    "wall_shelf",
];

/// Each part's offset when fully exploded, by part id.
fn offsets(design: &SolvedDesign, graph: &SceneGraph) -> Vec<Vec3> {
    let mut offsets = vec![Vec3::ZERO; design.parts.len()];
    for id in graph.node_ids() {
        if let Some(Item::Part(part)) = graph.node(id).map(|node| node.item) {
            offsets[part.index()] = graph.explode_offset(id);
        }
    }
    offsets
}

fn offset_of(design: &SolvedDesign, offsets: &[Vec3], name: &str) -> Vec3 {
    let part = design
        .part_named(name)
        .unwrap_or_else(|| panic!("no part '{name}'"));
    offsets[part.id.index()]
}

#[test]
fn the_stool_rungs_come_out_of_their_sockets_with_the_legs() {
    let design = solve("stool");
    let graph = SceneGraph::build(&design, BuildOptions::default());
    let offsets = offsets(&design, &graph);
    let at = |name: &str| offset_of(&design, &offsets, name);
    // The seat stays; every leg drops off it and moves out at its corner.
    assert_eq!(at("seat"), Vec3::ZERO);
    let leg = at("leg[front_left]");
    assert!(leg.y < -100.0 && leg.x < -30.0 && leg.z > 30.0, "{leg}");
    // A rung drops with the legs it is held between, and goes out with
    // them the other way, but not along its length: the legs pull off its
    // ends, so its tenons still point at their sockets.
    for (name, out) in [
        ("rung[front]", Vec3::Z),
        ("rung[back]", -Vec3::Z),
        ("side_rung[left]", -Vec3::X),
        ("side_rung[right]", Vec3::X),
    ] {
        let rung = at(name);
        assert!((rung.y - leg.y).abs() < 1e-3, "{name} drops: {rung}");
        assert!(rung.dot(out) > 30.0, "{name} goes out: {rung}");
    }
    let front = at("rung[front]");
    assert!(front.x.abs() < 1e-3, "{front}");
    let gap = at("leg[front_right]").x - front.x;
    assert!(
        gap > 30.0,
        "the front right leg pulls off the rung's end: {gap}"
    );
}

/// The moving part of a joint pair (its last push or drive) and the parts
/// it comes out of, for pairs inside one assembly.
fn movers(design: &SolvedDesign) -> Vec<(PartId, Vec<PartId>)> {
    let mut movers = Vec::new();
    for joint in &design.joints {
        for pair in &joint.pairs {
            let Some(step) = pair
                .insert
                .iter()
                .rev()
                .find(|step| !matches!(step.kind, StepKind::Rotate))
            else {
                continue;
            };
            let Some(mover) = design.part(step.part) else {
                continue;
            };
            let partners: Vec<PartId> = pair
                .roles
                .iter()
                .map(|role| role.part)
                .chain(pair.made_parts.iter().copied())
                .filter(|part| *part != mover.id)
                .collect();
            if partners
                .iter()
                .filter_map(|part| design.part(*part))
                .all(|part| part.assembly == mover.assembly)
            {
                movers.push((mover.id, partners));
            }
        }
    }
    movers
}

/// What some offsets all share, axis by axis.
fn shared(offsets: impl Iterator<Item = Vec3>) -> Vec3 {
    offsets
        .reduce(|a, b| {
            let axis = |a: f32, b: f32| {
                if a * b > 0.0 {
                    a.signum() * a.abs().min(b.abs())
                } else {
                    0.0
                }
            };
            Vec3::new(axis(a.x, b.x), axis(a.y, b.y), axis(a.z, b.z))
        })
        .unwrap_or(Vec3::ZERO)
}

#[test]
fn no_part_is_left_where_it_was_unless_the_rule_keeps_it() {
    // A part stays when it is what the others go into (no joint moves it
    // out), or when it is held from both sides and the parts it is held
    // between move apart and share no move it could follow.
    for example in EXAMPLES {
        let design = solve(example);
        let graph = SceneGraph::build(&design, BuildOptions::default());
        let offsets = offsets(&design, &graph);
        let movers = movers(&design);
        let mut stays = Vec::new();
        for part in &design.parts {
            if offsets[part.id.index()].length() > 0.5 {
                continue;
            }
            let pairs: Vec<&Vec<PartId>> = movers
                .iter()
                .filter(|(mover, _)| *mover == part.id)
                .map(|(_, partners)| partners)
                .collect();
            match pairs.len() {
                0 => stays.push(part.name.clone()),
                1 => panic!(
                    "{example}: {} comes out of one joint but stays put",
                    part.name
                ),
                _ => {
                    let follow = shared(
                        pairs
                            .iter()
                            .flat_map(|partners| partners.iter())
                            .map(|partner| offsets[partner.index()]),
                    );
                    assert!(
                        follow.length() < 0.5,
                        "{example}: {} is held between parts that all move {follow}, \
                         but stays put",
                        part.name
                    );
                    stays.push(part.name.clone());
                }
            }
        }
        println!("{example}: stays {stays:?}");
        assert!(stays.len() < design.parts.len(), "{example}: nothing moves");
    }
}
