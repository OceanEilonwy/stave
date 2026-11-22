//! Executable spec: set positions and loose members (grammar,
//! "Mechanisms", set positions; "Shapes", a loose member; "Scenes", a pose
//! of a drive with set positions and motions made one at a time).
//!
//! The corpus is `tests/turning/holed_cheval.stave`: the rising cheval
//! mirror of `tests/turning/rising_cheval.stave` with a row of holes in
//! each upright instead of a slot. The height drive is set at each hole
//! (`set_at: every(hole_pitch)`, four heights 100mm apart), and the bolt,
//! with the washer and knob it carries, is loose: pulled out to move the
//! frame, checked in each hole and never swept. The frame is still swept
//! between heights, and tilted at every height.

mod common;

use common::open_files_with_every_warning;
use stave_app::Opened;
use stave_app::view;
use stave_lang::units::Length;
use stave_lang::{Code, render_diagnostics};
use stave_model::mechanism::{DriveIndex, DriveValue, Mechanism};
use stave_model::solved::PoseValue;

fn fixture() -> String {
    let path = format!(
        "{}/tests/turning/holed_cheval.stave",
        env!("CARGO_MANIFEST_DIR")
    );
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

/// The design without its scenes: every drive swept from rest and from
/// its set positions only.
fn without_scenes() -> String {
    let text = fixture();
    let scene = text.find("scene lowered").expect("the scenes");
    text[..scene].trim_end().to_owned() + "\n"
}

/// The design without its scenes and with `scene` added.
fn with_scene(scene: &str) -> String {
    format!("{}\n{scene}\n", without_scenes())
}

/// The design with its height drive's `set_at:` written `set_at`.
fn set_at(text: &str, set_at: &str) -> String {
    changed(
        text,
        "set_at: every(hole_pitch))",
        &format!("set_at: {set_at})"),
    )
}

/// The codes of a design's diagnostics.
fn codes(opened: &Opened) -> Vec<Code> {
    opened.diagnostics.iter().map(|d| d.code).collect()
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

fn drive(index: usize) -> DriveIndex {
    DriveIndex::new(index).unwrap()
}

fn mm(value: i64) -> Length {
    Length::from_mm(value)
}

#[test]
fn a_frame_set_in_a_row_of_holes_checks_clean() {
    // The bolt is not swept through the wood between the holes, the
    // frame is swept between heights, and the tilt at every height.
    let opened = open(&fixture());
    assert!(opened.diagnostics.is_empty(), "{}", rendered(&opened));
    let mechanism = frame_mechanism(&opened);
    let height = mechanism.drive_at(drive(1)).expect("the height");
    let set = height.set_at.as_ref().expect("set positions");
    // From 0mm down to -300mm, every 100mm: the closed pose first.
    assert_eq!(
        set.values(),
        [0, -100, -200, -300].map(|v| DriveValue::Travel(mm(v)))
    );
    assert_eq!(set.every(), Some(DriveValue::Travel(mm(100))));
    // The tilt is continuous.
    assert!(mechanism.drive_at(drive(0)).unwrap().set_at.is_none());
    // The bolt and what it carries are loose; the stile is not.
    let design = opened.design.as_ref().unwrap();
    let pivot = design
        .hardware()
        .iter()
        .find(|h| h.name == "frame.pivot_l")
        .unwrap();
    let loose: Vec<(&str, bool)> = pivot
        .members
        .iter()
        .map(|member| (member.name.as_str(), member.loose))
        .collect();
    assert_eq!(loose, [("bolt", true), ("washer", true), ("knob", true)]);
}

#[test]
fn the_frame_is_swept_between_heights() {
    // A short, narrow stretcher 315mm to 345mm up, between the stiles and
    // joined to nothing: the bottom rail clears it at every height (above
    // it at 0mm and 100mm down, below it at 200mm and 300mm), but passes
    // through it on the way from 100mm to 200mm. The tilt is cut to 5deg,
    // so only the height's sweep meets it.
    let text = changed(
        &without_scenes(),
        "    flush(upright_l.bottom)\n",
        "    flush(upright_l.bottom, inset: 315mm)\n",
    );
    let text = changed(
        &text,
        "part stretcher : beech_25 {\n  length: fit\n  width:  100mm",
        "part stretcher : beech_25 {\n  length: 400mm\n  width:  30mm",
    );
    let text = changed(
        &text,
        "joint stretcher_tenons : mortise_tenon(stretcher, into: [upright_l, upright_r], length: 30mm)\n",
        "",
    );
    let text = changed(&text, "  param turn = 360deg", "  param turn = 5deg");
    let opened = open(&text);
    assert_eq!(codes(&opened), [Code::Motion], "{}", rendered(&opened));
    let message = &opened.diagnostics[0].message;
    assert!(message.contains("frame.height"), "{message}");
    insta::assert_snapshot!(rendered(&opened));
}

#[test]
fn the_tilt_is_swept_at_the_lowest_set_height_that_no_scene_names() {
    // The stretcher moved behind the uprights, 200mm to 220mm up: the
    // frame slides down past it, and the tilt clears it at 0mm, 100mm and
    // 200mm down, but a stile's top swings through it at 300mm down.
    // No scene names that height; it is a set position, so the tilt is
    // swept from there.
    let text = changed(
        &without_scenes(),
        "    flush(upright_l.bottom)\n    center(envelope, across)\n    center(upright_l, deep)\n",
        "    flush(upright_l.bottom, inset: 200mm)\n    center(envelope, across)\n    against(upright_l.back)\n",
    );
    let text = changed(
        &text,
        "part stretcher : beech_25 {\n  length: fit\n  width:  100mm",
        "part stretcher : beech_25 {\n  length: 700mm\n  width:  20mm",
    );
    let text = changed(
        &text,
        "joint stretcher_tenons : mortise_tenon(stretcher, into: [upright_l, upright_r], length: 30mm)\n",
        "",
    );
    let opened = open(&text);
    assert_eq!(codes(&opened), [Code::Motion], "{}", rendered(&opened));
    let diagnostic = &opened.diagnostics[0];
    assert!(
        diagnostic.message.contains("frame.tilt"),
        "{}",
        diagnostic.message
    );
    assert!(
        diagnostic
            .notes
            .iter()
            .any(|note| note.contains("frame.height held at 300mm")),
        "{}",
        rendered(&opened)
    );
    insta::assert_snapshot!(rendered(&opened));
}

#[test]
fn a_loose_member_is_checked_at_every_set_position() {
    // Only two spare holes: at the lowest set position the bolt goes into
    // the upright where no hole is drilled.
    let text = changed(
        &without_scenes(),
        "  param holes_below = 3\n",
        "  param holes_below = 3\n  param holes_drilled = 2\n",
    );
    let text = changed(
        &text,
        "pitch: hole_pitch, down: holes_below)",
        "pitch: hole_pitch, down: holes_drilled)",
    );
    let opened = open(&text);
    assert!(
        opened.diagnostics.iter().any(|d| d.code == Code::Motion
            && d.message
                == "frame.pivot_l's bolt hits upright_l when frame.height is set at 300mm"),
        "{}",
        rendered(&opened)
    );
    insta::assert_snapshot!(rendered(&opened));
}

#[test]
fn a_loose_member_still_counts_in_its_clamp() {
    // The stile set 5mm off the upright, with a 3mm washer: the knob
    // drawn tight pulls across a 2mm gap. The bolt, washer and knob are
    // loose, but the clamp's stack is checked at rest, with them in it.
    let text = without_scenes().replace("gap: washer", "gap: washer + 2mm");
    let opened = open(&text);
    assert!(
        opened.diagnostics.iter().any(|d| d.code == Code::NoContact),
        "{}",
        rendered(&opened)
    );
}

#[test]
fn a_pose_between_set_positions_is_a_motion_error() {
    let opened = open(&with_scene(
        "scene lowered \"Lowered\" {\n  pose frame.height = 120mm\n}",
    ));
    assert_eq!(codes(&opened), [Code::Motion], "{}", rendered(&opened));
    assert_eq!(
        opened.diagnostics[0].message,
        "frame.height can't be set at 120mm: it sets every 100mm"
    );
    assert_eq!(
        opened.diagnostics[0].help.as_deref(),
        Some("pose frame.height at 100mm or 200mm")
    );
    insta::assert_snapshot!(rendered(&opened));
}

#[test]
fn a_pose_not_in_a_list_of_set_positions_names_them() {
    let text = set_at(&without_scenes(), "[-100mm, -300mm]");
    let text = format!("{text}\nscene lowered \"Lowered\" {{\n  pose frame.height = 200mm\n}}\n");
    let opened = open(&text);
    assert_eq!(codes(&opened), [Code::Motion], "{}", rendered(&opened));
    assert_eq!(
        opened.diagnostics[0].message,
        "frame.height can't be set at 200mm: it sets at 0mm, 100mm and 300mm"
    );
    insta::assert_snapshot!(rendered(&opened));
}

#[test]
fn open_is_the_furthest_set_position() {
    // Set at 0mm, 100mm and 200mm down: open is 200mm, short of the
    // drive's 300mm. A value written twice is one set position.
    let text = set_at(&without_scenes(), "[-100mm, -200mm, -100mm]");
    let text = format!("{text}\nscene all_open \"All open\" {{\n  pose frame = open\n}}\n");
    let opened = open(&text);
    assert!(opened.diagnostics.is_empty(), "{}", rendered(&opened));
    let mechanism = frame_mechanism(&opened);
    assert_eq!(
        mechanism
            .drive_at(drive(1))
            .unwrap()
            .set_at
            .as_ref()
            .unwrap()
            .values(),
        [0, -100, -200].map(|v| DriveValue::Travel(mm(v)))
    );
    assert_eq!(
        mechanism.value_of(drive(1), PoseValue::Open),
        Some(DriveValue::Travel(mm(-200)))
    );
    // The scene graph opens the frame as far.
    let design = opened.design.as_ref().unwrap();
    let graph = view::scene(design);
    let stile = graph
        .node_ids()
        .find(|id| graph.node(*id).is_some_and(|n| n.name == "frame.stile_l"))
        .expect("the stile");
    let scene = graph.scene_named("all_open").expect("the scene");
    let moved = graph.pose(scene).transforms()[stile.index()].transform_point3(glam::Vec3::ZERO);
    assert!(
        (moved - glam::Vec3::new(0.0, -200.0, 0.0)).length() < 1e-3,
        "{moved}"
    );
}

#[test]
fn the_motion_slider_snaps_to_set_positions() {
    let opened = open(&fixture());
    let design = opened.design.as_ref().expect("it solves");
    let graph = view::scene(design);
    let frame = design
        .assemblies()
        .iter()
        .find(|a| a.name == "frame")
        .unwrap()
        .id;
    let drives = graph.drives(frame);
    assert!(drives[0].set_at.is_empty());
    assert_eq!(
        drives[1].set_at,
        [0, 100, 200, 300].map(|v| PoseValue::Travel(mm(v)))
    );
    // The height snaps to the nearest hole; the tilt is set anywhere.
    assert_eq!(drives[1].snapped(130.0), 100.0);
    assert_eq!(drives[1].snapped(170.0), 200.0);
    assert_eq!(drives[1].snapped(1000.0), 300.0);
    assert_eq!(drives[0].snapped(0.3), 0.3);
}

#[test]
fn two_pivots_set_at_different_positions_do_not_move_alike() {
    let text = changed(
        &without_scenes(),
        "pivot_bolt(stand_r, into: stile_r, face: right, washer_thickness: washer)",
        "pivot_bolt(stand_r, into: stile_r, face: right, washer_thickness: washer, hole_pitch: 150mm, holes_below: 2)",
    );
    let opened = open(&text);
    assert!(
        opened
            .diagnostics
            .iter()
            .any(|d| d.code == Code::Motion && d.message == "frame's hardware doesn't move alike"),
        "{}",
        rendered(&opened)
    );
}

#[test]
fn a_set_position_outside_the_range_is_a_motion_error() {
    let opened = open(&set_at(&without_scenes(), "[0mm, -350mm]"));
    assert_eq!(codes(&opened), [Code::Motion], "{}", rendered(&opened));
    assert_eq!(
        opened.diagnostics[0].message,
        "pivot_bolt's drive height can't be set at -350mm: it runs from 0mm to -300mm"
    );
    insta::assert_snapshot!(rendered(&opened));
}

#[test]
fn an_angle_in_a_travels_set_positions_is_a_unit_mismatch() {
    // As an angle in its from: or to: is.
    let opened = open(&set_at(&without_scenes(), "every(15deg)"));
    assert_eq!(
        codes(&opened),
        [Code::UnitMismatch],
        "{}",
        rendered(&opened)
    );
    insta::assert_snapshot!(rendered(&opened));
}

#[test]
fn a_step_of_nothing_is_out_of_range() {
    let opened = open(&set_at(&without_scenes(), "every(0mm)"));
    assert_eq!(codes(&opened), [Code::OutOfRange], "{}", rendered(&opened));
    assert_eq!(
        opened.diagnostics[0].message,
        "every(...) needs a step more than 0mm, found 0mm"
    );
    insta::assert_snapshot!(rendered(&opened));
}

#[test]
fn a_step_that_makes_too_many_set_positions_is_out_of_range() {
    let opened = open(&set_at(&without_scenes(), "every(0.1mm)"));
    assert_eq!(codes(&opened), [Code::OutOfRange], "{}", rendered(&opened));
    insta::assert_snapshot!(rendered(&opened));
}

#[test]
fn an_empty_list_of_set_positions_is_a_type_mismatch() {
    let opened = open(&set_at(&without_scenes(), "[]"));
    assert_eq!(
        codes(&opened),
        [Code::TypeMismatch],
        "{}",
        rendered(&opened)
    );
    assert_eq!(
        opened.diagnostics[0].message,
        "set_at: needs at least one value"
    );
    insta::assert_snapshot!(rendered(&opened));
}

#[test]
fn set_positions_written_otherwise_are_a_type_mismatch() {
    let opened = open(&set_at(&without_scenes(), "100mm"));
    assert_eq!(
        codes(&opened),
        [Code::TypeMismatch],
        "{}",
        rendered(&opened)
    );
    assert_eq!(
        opened.diagnostics[0].message,
        "set_at: needs a list of values or every(...)"
    );
}

#[test]
fn a_set_positions_list_is_formatted_like_any_call() {
    let text = set_at(&without_scenes(), "[ 0mm,-100mm ,  -300mm ]");
    let mut sources = stave_lang::SourceMap::new();
    let file = sources.add("project.stave", &text).unwrap();
    let formatted = stave_lang::format_file(file, &text).expect("it formats");
    assert!(
        formatted.contains("set_at: [0mm, -100mm, -300mm])"),
        "{formatted}"
    );
}

#[test]
fn loose_is_a_bool_written_once() {
    let twice = changed(
        &without_scenes(),
        "    loose: true\n",
        "    loose: true\n    loose: false\n",
    );
    let opened = open(&twice);
    assert_eq!(
        codes(&opened),
        [Code::DuplicateName],
        "{}",
        rendered(&opened)
    );
    let counted = changed(&without_scenes(), "    loose: true\n", "    loose: 3\n");
    let opened = open(&counted);
    assert_eq!(
        codes(&opened),
        [Code::TypeMismatch],
        "{}",
        rendered(&opened)
    );
}
