//! The chidori (`examples/japanese_joinery/chidori.stave`), checked on the
//! node in `examples/joint_samples` the way the Python design check that
//! used to live in `tools/joint-sim/chidori.py` checked it: the assembled
//! node has no wood in wood, every stick is locked, the node comes apart
//! by its steps run backwards, and it depends on the lock's rounding.
//!
//! The script's feature checks are not repeated here: without the flats
//! the lock can't slide in (`fit::a_chidori_without_flats_cannot_come_apart`,
//! a snapshot of the diagnostic), and without the rounding it doesn't fit
//! the holes at all (`fit::the_chidori_comes_apart_as_the_simulation_proves`,
//! an `overlap` at rest, which the script's slide in implies).

mod common;

use std::path::{Path, PathBuf};

use glam::DVec3;
use stave_app::Opened;
use stave_geom::collide::{self, WorldSolid};
use stave_geom::sequence::{self, Member, Moved};
use stave_geom::solid::Solid;
use stave_lang::units::Length;
use stave_model::solved::{Point, SolvedDesign};

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The joint samples, where `node` is a chidori of `stick_z` (the lock),
/// `stick_x` (the first) and `stick_y` (the second).
fn samples() -> Opened {
    stave_app::open(&workspace().join("examples/joint_samples")).unwrap()
}

/// The three sticks' cut solids, placed, in the order lock, first, second.
fn sticks(design: &SolvedDesign) -> [(String, WorldSolid); 3] {
    ["stick_z", "stick_x", "stick_y"].map(|name| {
        let part = common::part(design, name);
        let solid = Solid::of(part, design);
        (name.to_owned(), WorldSolid::of_part(part, &solid))
    })
}

/// How far the script moved each stick to see that it is locked (four
/// sampling steps of 0.4mm).
const NUDGE_NM: f64 = 1_600_000.0;

/// The 26 directions to the neighbours of a cube's centre, as exact
/// offsets of [`NUDGE_NM`] (rounded to the nanometre).
fn nudges() -> Vec<Point> {
    let mut out = Vec::new();
    for across in [-1.0f64, 0.0, 1.0] {
        for up in [-1.0f64, 0.0, 1.0] {
            for deep in [-1.0f64, 0.0, 1.0] {
                let direction = DVec3::new(across, up, deep);
                if direction == DVec3::ZERO {
                    continue;
                }
                let offset = direction.normalize() * NUDGE_NM;
                let exact = |value: f64| Length::from_nm(value.round() as i64);
                out.push(Point {
                    across: exact(offset.x),
                    up: exact(offset.y),
                    deep: exact(offset.z),
                });
            }
        }
    }
    out
}

/// The directions one stick can move [`NUDGE_NM`] without its wood going
/// into another's.
fn free_directions(index: usize) -> Vec<Point> {
    let opened = samples();
    let design = common::design(&opened);
    let all = sticks(design);
    let (_, moving) = &all[index];
    nudges()
        .into_iter()
        .filter(|by| {
            let moved = moving.translated(*by);
            all.iter()
                .enumerate()
                .filter(|(other, _)| *other != index)
                .all(|(_, (_, solid))| collide::overlaps(&moved, solid).is_empty())
        })
        .collect()
}

#[test]
fn the_assembled_node_has_no_wood_in_wood() {
    let opened = samples();
    let design = common::design(&opened);
    let all = sticks(design);
    for (i, (a_name, a)) in all.iter().enumerate() {
        for (b_name, b) in all.iter().skip(i + 1) {
            assert_eq!(collide::overlaps(a, b), Vec::new(), "{a_name} and {b_name}");
        }
    }
}

#[test]
fn the_lock_can_not_move_in_any_direction() {
    assert_eq!(free_directions(0), Vec::new());
}

#[test]
fn the_first_stick_can_not_move_in_any_direction() {
    assert_eq!(free_directions(1), Vec::new());
}

#[test]
fn the_second_stick_can_not_move_in_any_direction() {
    assert_eq!(free_directions(2), Vec::new());
}

/// The node comes apart by its `insert:` steps run backwards, every move
/// swept: the lock turns back a quarter turn, slides out along the
/// diagonal of its section until clear, and the second stick lifts off
/// the first.
#[test]
fn the_lock_turns_a_quarter_and_slides_out() {
    let opened = samples();
    let design = common::design(&opened);
    let joint = common::joint(design, "node");
    let [pair] = joint.pairs.as_slice() else {
        panic!("one node");
    };
    let members = sequence::pair_members(pair);
    let solids: Vec<Solid> = members
        .iter()
        .map(|id| Solid::of(design.part(*id).unwrap(), design))
        .collect();
    let parts: Vec<Member<'_>> = members
        .iter()
        .zip(&solids)
        .map(|(id, solid)| Member {
            part: design.part(*id).unwrap(),
            solid,
        })
        .collect();
    let steps = sequence::pair_steps(design, pair, &members, None).unwrap();
    let traced = sequence::trace(&parts, &steps).expect("the node comes apart");
    let name = |member: usize| design.part(members[member]).unwrap().name.as_str();
    let [lower, slide, turn] = traced.as_slice() else {
        panic!("three steps: {traced:?}");
    };

    assert_eq!(name(turn.member), "stick_z");
    let Moved::Turn { axis, angle, .. } = turn.moved else {
        panic!("the lock turns: {turn:?}");
    };
    assert!((axis.abs() - DVec3::Y).length() < 1e-9, "about its length");
    assert!(
        (angle.abs() - std::f64::consts::FRAC_PI_2).abs() < 1e-9,
        "a quarter turn"
    );

    assert_eq!(name(slide.member), "stick_z");
    let Moved::Slide(out) = slide.moved else {
        panic!("the lock slides: {slide:?}");
    };
    assert!(out.y.abs() < 1.0, "level: {out}");
    assert!(
        (out.x.abs() - out.z.abs()).abs() < 1.0,
        "along the diagonal: {out}"
    );
    // Out of the node: its rounded section (22.5mm) clear of the slots'
    // far ends at the sticks' corners.
    assert!(out.length() > 15e6, "{out}");

    assert_eq!(name(lower.member), "stick_y");
    let Moved::Slide(lift) = lower.moved else {
        panic!("the second stick lifts: {lower:?}");
    };
    assert!(
        lift.x == 0.0 && lift.z == 0.0 && lift.y.abs() >= 15e6,
        "{lift}"
    );
}
