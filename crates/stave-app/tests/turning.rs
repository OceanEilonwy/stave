//! Executable spec: a frame that turns between two uprights on pivot
//! bolts (grammar, "Mechanisms": `pins:`, a role in an entry, several
//! items on one assembly, a whole turn).
//!
//! The corpus is `tests/turning/cheval.stave`: a cheval mirror whose frame
//! (two stiles and two rails, in a component) turns on a `pivot_bolt`
//! through each upright. The kind's pins run into the uprights' faces
//! (`pins: depth`), and the bolt is pinned to the upright it turns in, the
//! kind's `piece` role (`pivot bolt, piece at ...`), which stays put. The
//! two bolts are written each on its own, on the outside face of its
//! upright, and turn the frame as one, all the way round.

mod common;

use common::open_files_with_every_warning;
use stave_app::Opened;
use stave_geom::collide::Placement;
use stave_geom::mechanism::MechanismPath;
use stave_lang::placement::{Axis, Direction};
use stave_lang::units::{Angle, Length};
use stave_lang::{Code, render_diagnostics};
use stave_model::mechanism::{Mechanism, MotionShape};
use stave_model::solved::{Point, PoseValue};

fn fixture() -> String {
    let path = format!("{}/tests/turning/cheval.stave", env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"))
}

fn open(text: &str) -> Opened {
    open_files_with_every_warning(&[("project.stave", text)])
}

fn rendered(opened: &Opened) -> String {
    render_diagnostics(&opened.sources, &opened.diagnostics)
}

/// The text with `old` (written exactly once) replaced by `new`.
fn changed(text: &str, old: &str, new: &str) -> String {
    assert_eq!(text.matches(old).count(), 1, "{old}");
    text.replace(old, new)
}

/// The design with its one scene posing the frame at `pose` instead.
fn posed(pose: &str) -> String {
    changed(
        &fixture(),
        "pose frame = 15deg",
        &format!("pose frame = {pose}"),
    )
}

/// The design without its scene: the frame swept from rest only.
fn without_scene() -> String {
    let text = fixture();
    let scene = text.find("scene tilted").expect("the scene");
    text[..scene].trim_end().to_owned() + "\n"
}

fn frame_mechanism(opened: &Opened) -> &Mechanism {
    let design = opened.design.as_ref().expect("a solved design");
    design
        .assemblies()
        .iter()
        .find(|assembly| assembly.name == "frame")
        .and_then(|assembly| assembly.mechanism.as_ref())
        .expect("the frame moves")
}

#[test]
fn a_frame_turns_on_pivot_bolts_whose_pins_run_into_the_uprights() {
    let opened = open(&without_scene());
    assert!(opened.diagnostics.is_empty(), "{}", rendered(&opened));
    let mechanism = frame_mechanism(&opened);
    // The bolts' common line runs across, through the middle of the
    // uprights' faces at the bolts' height; a positive turn carries the
    // frame's top (along, up) toward across (back).
    let MotionShape::Turn { line, opening } = mechanism.shape() else {
        panic!("the frame turns about one line: {:?}", mechanism.shape());
    };
    assert_eq!(line.axis, Axis::Across);
    assert_eq!(
        (line.through.up, line.through.deep),
        (Length::from_mm(850), Length::from_mm(200))
    );
    assert_eq!(
        (line.free_side, line.swing_toward),
        (Direction::Up, Direction::Back)
    );
    assert_eq!(opening, Angle::from_deg(360));
    // The bolt is pinned to the upright's part, held, which adds no
    // freedom; the driven bolt is the one free member.
    let members: Vec<(&str, bool)> = mechanism
        .members()
        .iter()
        .map(|m| (m.name.as_str(), m.body.body().is_some()))
        .collect();
    assert_eq!(
        members,
        [
            ("bolt", true),
            ("washer", true),
            ("knob", true),
            ("piece", false)
        ]
    );
}

#[test]
fn two_pivots_written_each_on_its_own_face_move_alike() {
    // One bolt on the left face of the left upright, the other on the
    // right face of the right upright: their depths point opposite ways,
    // but their pins lie on one line, so both drive the one motion.
    let opened = open(&without_scene());
    assert!(opened.diagnostics.is_empty(), "{}", rendered(&opened));
    let design = opened.design.as_ref().unwrap();
    let names: Vec<&str> = frame_mechanism(&opened)
        .hardware()
        .iter()
        .map(|id| design.hardware()[id.index()].name.as_str())
        .collect();
    assert_eq!(names, ["frame.pivot_l", "frame.pivot_r"]);
}

#[test]
fn pivots_whose_pins_are_not_on_one_line_do_not_move_alike() {
    let text = changed(
        &without_scene(),
        "pivot_bolt(stand_r, into: stile_r, face: right, washer_thickness: washer)",
        "pivot_bolt(stand_r, into: stile_r, face: right, washer_thickness: washer, height: 800mm)",
    );
    let opened = open(&text);
    assert_eq!(
        opened
            .diagnostics
            .iter()
            .map(|d| d.code)
            .collect::<Vec<_>>(),
        [Code::Motion],
        "{}",
        rendered(&opened)
    );
    insta::assert_snapshot!(rendered(&opened));
}

#[test]
fn the_sweep_turns_the_frame_about_the_bolts() {
    // The stretcher moved up into the circle the frame's bottom rail
    // turns through: the sweep from rest finds it.
    let text = changed(
        &without_scene(),
        "flush(upright_l.bottom, inset: 100mm)",
        "flush(upright_l.bottom, inset: 200mm)",
    );
    let opened = open(&text);
    assert_eq!(
        opened
            .diagnostics
            .iter()
            .map(|d| d.code)
            .collect::<Vec<_>>(),
        [Code::Motion],
        "{}",
        rendered(&opened)
    );
    insta::assert_snapshot!(rendered(&opened));
}

// The frame posed at 15deg: the bolt turning inside its own hole in the
// upright is reported as hitting the upright, because a posed overlap and
// a sweep take a hole whose axis is the line of the turn as wood. That is
// the known gap that Change 6, "A pin turning in its own hole", in
// docs/reviews/2026-11-22-turning-frames.md (section "Change 5: one rule
// for sweeping several motions") closes, in a later step:
//
//   error[motion]: frame.pivot_l's bolt hits upright_l when frame is open 15deg
//      = note: frame.pivot_l's bolt goes 8mm into upright_l there
#[test]
#[ignore = "a pin turning in its own hole: Change 6 of the turning-frames review"]
fn the_frame_posed_tilted_back_checks_clean() {
    let opened = open(&fixture());
    assert!(opened.diagnostics.is_empty(), "{}", rendered(&opened));
}

#[test]
fn a_turn_may_run_all_the_way_round_and_a_whole_turn_is_the_closed_pose() {
    let opened = open(&posed("360deg"));
    assert!(opened.diagnostics.is_empty(), "{}", rendered(&opened));
    let path = MechanismPath::new(frame_mechanism(&opened)).expect("the frame moves");
    let at = |angle: i64| {
        Placement::of_mechanisms(&[(&path, PoseValue::Angle(Angle::from_deg(angle)))], None)
    };
    assert_eq!(at(360), Placement::Exact(Point::default()));
    assert_eq!(at(0), at(360));
    assert_eq!(
        Placement::of_mechanisms(&[(&path, PoseValue::Open)], None),
        Placement::Exact(Point::default())
    );
    assert!(matches!(at(15), Placement::Turned(_)));
}

#[test]
fn a_point_on_the_pins_own_direction_is_a_mechanism_error() {
    let text = changed(
        &without_scene(),
        "pivot bolt, piece at point(along: 0mm, across: 0mm)",
        "pivot bolt, piece at point(along: 0mm, across: 0mm, depth: 5mm)",
    );
    let opened = open(&text);
    assert_eq!(
        opened
            .diagnostics
            .iter()
            .map(|d| d.code)
            .collect::<Vec<_>>(),
        [Code::Mechanism],
        "{}",
        rendered(&opened)
    );
    insta::assert_snapshot!(rendered(&opened));
}

#[test]
fn pins_written_twice_is_a_duplicate() {
    let text = changed(
        &without_scene(),
        "    pins: depth\n",
        "    pins: depth\n    pins: along\n",
    );
    let opened = open(&text);
    assert_eq!(
        opened
            .diagnostics
            .iter()
            .map(|d| d.code)
            .collect::<Vec<_>>(),
        [Code::DuplicateName],
        "{}",
        rendered(&opened)
    );
    insta::assert_snapshot!(rendered(&opened));
}

#[test]
fn a_rail_along_the_pins_is_a_mechanism_error() {
    let text = changed(
        &without_scene(),
        "pivot bolt, piece at point(along: 0mm, across: 0mm)",
        "rail bolt, piece along depth",
    );
    let opened = open(&text);
    assert!(
        opened.diagnostics.iter().any(|d| d.code == Code::Mechanism
            && d.message
                == "this mechanism's pins run along 'depth', so a rail can't run along 'depth'"),
        "{}",
        rendered(&opened)
    );
    insta::assert_snapshot!(rendered(&opened));
}

#[test]
fn a_role_that_is_a_region_is_a_mechanism_error() {
    let text = changed(
        &without_scene(),
        "    into:  Part     // the frame's stile\n",
        "    into:  Part     // the frame's stile\n    room:  Region   // where the frame turns\n",
    );
    let text = changed(
        &text,
        "pivot bolt, piece at point(along: 0mm, across: 0mm)",
        "pivot bolt, room at point(along: 0mm, across: 0mm)",
    );
    let opened = open(&text);
    assert!(
        opened.diagnostics.iter().any(|d| d.code == Code::Mechanism
            && d.message == "a pin turns in one part; 'room' is a region"),
        "{}",
        rendered(&opened)
    );
    insta::assert_snapshot!(rendered(&opened));
}

#[test]
fn a_name_that_is_neither_a_member_nor_a_role_is_unknown() {
    let text = changed(
        &without_scene(),
        "pivot bolt, piece at point(along: 0mm, across: 0mm)",
        "pivot bolt, upright at point(along: 0mm, across: 0mm)",
    );
    let opened = open(&text);
    assert!(
        opened
            .diagnostics
            .iter()
            .any(|d| d.code == Code::UnknownName && d.message == "unknown name 'upright'"),
        "{}",
        rendered(&opened)
    );
}

#[test]
fn the_first_name_of_an_entry_is_always_a_member() {
    let text = changed(
        &without_scene(),
        "pivot bolt, piece at point(along: 0mm, across: 0mm)",
        "pivot piece, bolt at point(along: 0mm, across: 0mm)",
    );
    let opened = open(&text);
    assert!(
        opened
            .diagnostics
            .iter()
            .any(|d| d.code == Code::UnknownName && d.message == "unknown name 'piece'"),
        "{}",
        rendered(&opened)
    );
}
