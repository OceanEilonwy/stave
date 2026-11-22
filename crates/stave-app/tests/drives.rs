//! Executable spec: named drives (grammar, "Mechanisms", drives; "Scenes",
//! a pose names a drive, and motions are made one at a time).
//!
//! The corpus is `tests/turning/rising_cheval.stave`: the cheval mirror of
//! `tests/turning/cheval.stave` with each pivot bolt running in a slot
//! routed along its upright (`slot bolt, piece from ... to ...`, a role in
//! the entry) and turning in it. The bolt's kind has two drives, `tilt` (a
//! turn) and `height` (a travel along the slot): the frame is one
//! assembly that moves two ways. Its scenes pose `frame.height` alone and
//! `frame.tilt` with `frame.height`.

mod common;

use common::open_files_with_every_warning;
use stave_app::Opened;
use stave_app::view;
use stave_geom::collide::Placement;
use stave_geom::mechanism::MechanismPath;
use stave_lang::placement::{Axis, Direction};
use stave_lang::units::{Angle, Length};
use stave_lang::{Code, render_diagnostics};
use stave_model::mechanism::{DriveIndex, DriveValue, Mechanism, MotionShape};
use stave_model::solved::{Point, PoseValue};

fn fixture() -> String {
    let path = format!(
        "{}/tests/turning/rising_cheval.stave",
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

/// The design without its scenes: every drive swept from rest only.
fn without_scenes() -> String {
    let text = fixture();
    let scene = text.find("scene lowered").expect("the scenes");
    text[..scene].trim_end().to_owned() + "\n"
}

/// The design without its scenes and with `scene` added.
fn with_scene(scene: &str) -> String {
    format!("{}\n{scene}\n", without_scenes())
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

#[test]
fn a_frame_that_rises_and_tilts_checks_clean() {
    // Scenes pose the height alone, and the tilt and the height together;
    // each drive is swept at rest and at each value a scene names.
    let opened = open(&fixture());
    assert!(opened.diagnostics.is_empty(), "{}", rendered(&opened));
    let mechanism = frame_mechanism(&opened);
    let names: Vec<Option<&str>> = mechanism
        .drives()
        .iter()
        .map(|named| named.name.as_deref())
        .collect();
    assert_eq!(names, [Some("tilt"), Some("height")]);
    assert_eq!(mechanism.drive_named("height"), Some(drive(1)));
    // The bolt in its slot, pinned to the upright's part: three less one,
    // two freedoms, one for each drive. The two bolts move alike, drive
    // for drive.
    let design = opened.design.as_ref().unwrap();
    let hardware: Vec<&str> = mechanism
        .hardware()
        .iter()
        .map(|id| design.hardware()[id.index()].name.as_str())
        .collect();
    assert_eq!(hardware, ["frame.pivot_l", "frame.pivot_r"]);
}

#[test]
fn each_drive_has_an_exact_shape_with_the_other_held() {
    let opened = open(&without_scenes());
    assert!(opened.diagnostics.is_empty(), "{}", rendered(&opened));
    let mechanism = frame_mechanism(&opened);
    let closed = mechanism.closed();
    // The tilt at rest height: a turn about the bolts' line, as a pivot.
    let MotionShape::Turn { line, opening } = mechanism.shape_of(drive(0), &closed) else {
        panic!("the tilt turns about one line");
    };
    assert_eq!(line.axis, Axis::Across);
    assert_eq!(
        (line.through.up, line.through.deep),
        (Length::from_mm(850), Length::from_mm(200))
    );
    assert_eq!(opening, Angle::from_deg(360));
    // The tilt with the frame 150mm down: about the line where the bolts
    // are then, and the moving bodies start 150mm down.
    let lowered = [
        DriveValue::Turn(Angle::ZERO),
        DriveValue::Travel(Length::from_mm(-150)),
    ];
    let MotionShape::Turn { line, .. } = mechanism.shape_of(drive(0), &lowered) else {
        panic!("the tilt turns about one line at any height");
    };
    assert_eq!(line.through.up, Length::from_mm(700));
    assert_eq!(
        mechanism.offset_at(&lowered),
        Some(Point::default().with(Axis::Up, Length::from_mm(-150)))
    );
    // The height at rest angle: a travel straight down the slot, 300mm.
    assert_eq!(
        mechanism.shape_of(drive(1), &closed),
        MotionShape::Travel {
            toward: Direction::Down,
            travel: Length::from_mm(300),
        }
    );
    // The height with the frame tilted: the bodies are turned, so the
    // solver poses it.
    let tilted = [
        DriveValue::Turn(Angle::from_deg(15)),
        DriveValue::Travel(Length::ZERO),
    ];
    assert_eq!(mechanism.shape_of(drive(1), &tilted), MotionShape::General);
    assert_eq!(mechanism.offset_at(&tilted), None);
}

#[test]
fn a_scene_poses_both_drives_together() {
    let opened = open(&without_scenes());
    let path = MechanismPath::new(frame_mechanism(&opened)).expect("the frame moves");
    let down = Placement::of_poses(
        &[(
            &path,
            &[PoseValue::Closed, PoseValue::Travel(Length::from_mm(150))][..],
        )],
        None,
    );
    assert_eq!(
        down,
        Placement::Exact(Point::default().with(Axis::Up, Length::from_mm(-150)))
    );
    let open = Placement::of_poses(&[(&path, &[PoseValue::Open, PoseValue::Open][..])], None);
    // Open is a whole turn and the whole travel: 300mm down, not turned.
    assert_eq!(
        open,
        Placement::Exact(Point::default().with(Axis::Up, Length::from_mm(-300)))
    );
    let both = Placement::of_poses(
        &[(
            &path,
            &[
                PoseValue::Angle(Angle::from_deg(90)),
                PoseValue::Travel(Length::from_mm(150)),
            ][..],
        )],
        None,
    );
    let Placement::Turned(matrix) = both else {
        panic!("turned: {both:?}");
    };
    // The bolts' line at rest (850mm up, 200mm deep) is moved 150mm down,
    // and a point 100mm above it turns a quarter, toward the back.
    let at = |up: f64, deep: f64| glam::DVec3::new(0.0, up * 1e6, deep * 1e6);
    let pin = matrix.transform_point3(at(850.0, 200.0));
    assert!((pin - at(700.0, 200.0)).length() < 10.0, "{pin}");
    let above = matrix.transform_point3(at(950.0, 200.0));
    assert!((above - at(700.0, 300.0)).length() < 10.0, "{above}");
}

#[test]
fn the_scene_graph_poses_each_drive_on_its_own_slider() {
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
    let names: Vec<Option<&str>> = drives.iter().map(|d| d.name.as_deref()).collect();
    assert_eq!(names, [Some("tilt"), Some("height")]);
    assert!(drives[0].range.turns() && !drives[1].range.turns());
    let stile = graph
        .node_ids()
        .find(|id| graph.node(*id).is_some_and(|n| n.name == "frame.stile_l"))
        .expect("the stile");
    let rest = graph.scenes.first().unwrap();
    // The height's slider alone moves the stile straight down.
    let posed = graph.pose_with_drives(rest, &[(frame, drive(1), 120.0)], 0.0);
    let moved = posed.transforms()[stile.index()].transform_point3(glam::Vec3::ZERO);
    assert!(
        (moved - glam::Vec3::new(0.0, -120.0, 0.0)).length() < 1e-3,
        "{moved}"
    );
    // A scene poses both: the reading scene's stile is lower and turned.
    let reading = graph.scene_named("reading").expect("the scene");
    let posed = graph.pose(reading);
    let turned = posed.transforms()[stile.index()];
    assert!((turned.x_axis.truncate() - glam::Vec3::X).length() < 1e-4);
    assert!((turned.y_axis.truncate() - glam::Vec3::Y).length() > 0.1);
}

#[test]
fn the_tilt_is_swept_at_each_height_a_scene_names() {
    // The stretcher raised to 100..200mm, and the bolts' travel shortened
    // to 250mm, where the frame's bottom only meets it. A scene lowers the
    // frame 250mm: the tilt, swept with the frame held there, turns the
    // frame's ends through the stretcher. From rest it clears.
    let text = changed(
        &without_scenes(),
        "    flush(upright_l.bottom)\n",
        "    flush(upright_l.bottom, inset: 100mm)\n",
    );
    let text = changed(&text, "  param drop = 300mm", "  param drop = 250mm");
    let text = format!("{text}\nscene lowered \"Lowered\" {{\n  pose frame.height = 250mm\n}}\n");
    let opened = open(&text);
    assert_eq!(codes(&opened), [Code::Motion], "{}", rendered(&opened));
    let message = &opened.diagnostics[0].message;
    assert!(message.contains("frame.tilt"), "{message}");
    insta::assert_snapshot!(rendered(&opened));
}

#[test]
fn the_height_is_swept_through_its_travel() {
    // The stretcher raised to 100..200mm: lowering the frame 300mm runs its
    // bottom rail into it after 250mm.
    let text = changed(
        &without_scenes(),
        "    flush(upright_l.bottom)\n",
        "    flush(upright_l.bottom, inset: 100mm)\n",
    );
    let opened = open(&text);
    assert_eq!(codes(&opened), [Code::Motion], "{}", rendered(&opened));
    let message = &opened.diagnostics[0].message;
    assert!(message.contains("frame.height"), "{message}");
    insta::assert_snapshot!(rendered(&opened));
}

#[test]
fn open_and_closed_set_every_drive() {
    let opened = open(&with_scene(
        "scene all_open \"All open\" {\n  pose frame = open\n}",
    ));
    assert!(opened.diagnostics.is_empty(), "{}", rendered(&opened));
    let opened = open(&with_scene(
        "scene shut \"Shut\" {\n  pose frame = closed\n}",
    ));
    assert!(opened.diagnostics.is_empty(), "{}", rendered(&opened));
}

#[test]
fn a_pose_of_a_frame_that_moves_two_ways_names_its_drive() {
    let opened = open(&with_scene(
        "scene tilted \"Tilted\" {\n  pose frame = 15deg\n}",
    ));
    assert_eq!(codes(&opened), [Code::Motion], "{}", rendered(&opened));
    assert_eq!(
        opened.diagnostics[0].message,
        "`frame` moves two ways; write frame.tilt or frame.height"
    );
    insta::assert_snapshot!(rendered(&opened));
}

#[test]
fn a_length_for_a_turning_drive_is_a_type_mismatch() {
    let opened = open(&with_scene(
        "scene tilted \"Tilted\" {\n  pose frame.tilt = 150mm\n}",
    ));
    assert_eq!(
        codes(&opened),
        [Code::TypeMismatch],
        "{}",
        rendered(&opened)
    );
    insta::assert_snapshot!(rendered(&opened));
}

#[test]
fn a_drive_posed_twice_in_one_scene_is_a_duplicate() {
    let opened = open(&with_scene(
        "scene tilted \"Tilted\" {\n  pose frame.tilt = 15deg\n  pose frame.tilt = 30deg\n}",
    ));
    assert_eq!(
        codes(&opened),
        [Code::DuplicateName],
        "{}",
        rendered(&opened)
    );
    insta::assert_snapshot!(rendered(&opened));
}

#[test]
fn a_drive_the_frame_does_not_have_is_unknown() {
    let opened = open(&with_scene(
        "scene tilted \"Tilted\" {\n  pose frame.tlt = 15deg\n}",
    ));
    assert_eq!(codes(&opened), [Code::UnknownName], "{}", rendered(&opened));
    insta::assert_snapshot!(rendered(&opened));
}

#[test]
fn a_pose_past_a_drives_range_is_a_motion_error() {
    let opened = open(&with_scene(
        "scene lowered \"Lowered\" {\n  pose frame.height = 400mm\n}",
    ));
    assert_eq!(codes(&opened), [Code::Motion], "{}", rendered(&opened));
    assert_eq!(
        opened.diagnostics[0].message,
        "frame.height can't pull out 400mm: its hardware goes 300mm at most"
    );
}

#[test]
fn an_unnamed_drive_among_two_is_a_mechanism_error() {
    let text = changed(&without_scenes(), "drive height: travel", "drive: travel");
    let opened = open(&text);
    assert!(
        opened.diagnostics.iter().any(|d| d.code == Code::Mechanism
            && d.message == "this mechanism has 2 drives, so each needs a name"),
        "{}",
        rendered(&opened)
    );
    insta::assert_snapshot!(rendered(&opened));
}

#[test]
fn two_drives_with_one_name_are_a_duplicate() {
    let text = changed(
        &without_scenes(),
        "drive height: travel",
        "drive tilt: travel",
    );
    let opened = open(&text);
    assert!(
        opened
            .diagnostics
            .iter()
            .any(|d| d.code == Code::DuplicateName && d.message == "two drives are called 'tilt'"),
        "{}",
        rendered(&opened)
    );
    insta::assert_snapshot!(rendered(&opened));
}

#[test]
fn a_drive_named_as_an_item_of_the_component_is_a_duplicate() {
    // The component's `washer` param renamed `height`: `pose frame.height`
    // would name both it and the bolts' drive.
    let text = without_scenes()
        .replace("param washer  = 3mm", "param height  = 3mm")
        .replace("gap: washer", "gap: height")
        .replace("washer_thickness: washer", "washer_thickness: height");
    let opened = open(&text);
    // One mistake, one error: at the first bolt, though both have the
    // drive.
    assert_eq!(
        codes(&opened),
        [Code::DuplicateName],
        "{}",
        rendered(&opened)
    );
    insta::assert_snapshot!(rendered(&opened));
}

#[test]
fn a_stop_with_two_drives_is_a_mechanism_error() {
    let text = changed(
        &without_scenes(),
        "    drive height: travel(bolt, from: 0mm, to: -drop)\n",
        "    drive height: travel(bolt, from: 0mm, to: -drop)\n    stop: -drop \"the slot ends here\"\n",
    );
    let opened = open(&text);
    assert!(
        opened.diagnostics.iter().any(|d| d.code == Code::Mechanism
            && d.message == "a stop belongs to a mechanism with one drive"),
        "{}",
        rendered(&opened)
    );
    insta::assert_snapshot!(rendered(&opened));
}

#[test]
fn two_turns_of_one_member_are_a_mechanism_error() {
    let text = changed(
        &without_scenes(),
        "drive height: travel(bolt, from: 0mm, to: -drop)",
        "drive swing: turn(bolt, from: 0deg, to: 90deg)",
    );
    let opened = open(&text);
    assert!(
        opened.diagnostics.iter().any(|d| d.code == Code::Mechanism
            && d.message == "`tilt` and `swing` both turn `bolt`: a member turns one way"),
        "{}",
        rendered(&opened)
    );
}

#[test]
fn two_drives_need_two_freedoms() {
    // A pivot instead of the slot leaves the bolt one freedom.
    let text = changed(
        &without_scenes(),
        "slot bolt, piece from point(along: 0mm, across: 0mm) to point(along: -drop, across: 0mm)",
        "pivot bolt, piece at point(along: 0mm, across: 0mm)",
    );
    let opened = open(&text);
    assert!(
        opened.diagnostics.iter().any(|d| d.code == Code::Mechanism
            && d.message
                == "pivot_bolt's mechanism has 1 freedoms; it needs exactly 2, one for each drive"),
        "{}",
        rendered(&opened)
    );
    insta::assert_snapshot!(rendered(&opened));
}

#[test]
fn a_patch_replaces_a_scene_that_poses_drives() {
    // Patches have no path into a scene's poses; a scene's poses are
    // changed by replacing the scene, which keeps working with drives.
    let text = fixture();
    let mut sources = stave_lang::SourceMap::new();
    let file = sources.add("project.stave", &text).unwrap();
    let module = stave_lang::parse_file(file, &text);
    let patch_text = "stave 1\n\npatch \"read higher\" {\n  replace scene reading \"Tilted back\" {\n    pose frame.tilt   = 30deg\n    pose frame.height = 50mm\n  }\n}\n";
    let patch_file = sources.add("change.stave", patch_text).unwrap();
    let patch = stave_lang::parse_patch_file(patch_file, patch_text);
    assert!(!patch.has_errors(), "{:?}", patch.diagnostics);
    let target = stave_lang::PatchTarget::new(vec![stave_lang::ModuleFile {
        name: "project.stave",
        parse: &module,
    }]);
    let changed = stave_lang::apply_patch(&target, &patch).expect("the patch applies");
    let [patched] = changed.as_slice() else {
        panic!("one file changes: {changed:?}");
    };
    assert!(
        patched.text.contains("pose frame.tilt = 30deg"),
        "{}",
        patched.text
    );
    let opened = open(&patched.text);
    assert!(opened.diagnostics.is_empty(), "{}", rendered(&opened));
}
