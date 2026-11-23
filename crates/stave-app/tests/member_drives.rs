//! Executable spec: drives may act on any members (grammar, "Mechanisms",
//! drives; "How a mechanism is solved", fast paths).
//!
//! The corpus is two designs in `tests/turning/`:
//!
//! - `drafting_table.stave`: on each leg a steel sleeve runs up a slot on
//!   a rail along the leg (`drive height: travel(sleeve, ...)`), and the
//!   board's bolt turns in the sleeve (`drive tilt: turn(bolt, ...)`). The
//!   board moves with the bolt, which the solver places on the sleeve
//!   from both drives. A rail joins the legs at the top.
//! - `cross_slide.stave`: on each side panel a carriage runs up on a rail
//!   (`drive x: travel(carriage, ...)`), and the frame's stud runs back on
//!   a rail along the carriage (`drive y: travel(stud, ...)`). A rail
//!   joins the panels at the top, at the back.
//!
//! Each checks clean with a scene posing both drives, and each has one
//! fault that only a scene naming the other drive's value finds.

mod common;

use common::open_files_with_every_warning;
use stave_app::Opened;
use stave_lang::placement::{Axis, Direction};
use stave_lang::units::{Angle, Length};
use stave_lang::{Code, render_diagnostics};
use stave_model::mechanism::{DriveIndex, DriveValue, Mechanism, MotionShape};
use stave_model::solved::Point;

fn fixture(name: &str) -> String {
    let path = format!("{}/tests/turning/{name}.stave", env!("CARGO_MANIFEST_DIR"));
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

fn mechanism<'a>(opened: &'a Opened, assembly: &str) -> &'a Mechanism {
    let design = opened.design.as_ref().expect("a solved design");
    design
        .assemblies()
        .iter()
        .find(|found| found.name == assembly)
        .and_then(|found| found.mechanism.as_ref())
        .unwrap_or_else(|| panic!("{assembly} moves"))
}

fn drive(index: usize) -> DriveIndex {
    DriveIndex::new(index).unwrap()
}

/// The name of the member a drive acts on.
fn driven(mechanism: &Mechanism, index: usize) -> &str {
    let member = mechanism.drives().get(index).unwrap().drive.member();
    &mechanism.members()[member.index()].name
}

#[test]
fn a_board_raised_on_a_sleeve_and_tilted_on_it_checks_clean() {
    let opened = open(&fixture("drafting_table"));
    assert!(opened.diagnostics.is_empty(), "{}", rendered(&opened));
    let mechanism = mechanism(&opened, "board");
    // Each drive acts on its own member; the board moves with the bolt,
    // the member with: its stile.
    assert_eq!(driven(mechanism, 0), "sleeve");
    assert_eq!(driven(mechanism, 1), "bolt");
    assert_eq!(mechanism.members()[mechanism.moving().index()].name, "bolt");
    let design = opened.design.as_ref().unwrap();
    let hardware: Vec<&str> = mechanism
        .hardware()
        .iter()
        .map(|id| design.hardware()[id.index()].name.as_str())
        .collect();
    assert_eq!(hardware, ["board.lift_l", "board.lift_r"]);
}

#[test]
fn each_drive_of_the_drafting_table_has_an_exact_shape() {
    let opened = open(&fixture("drafting_table"));
    let mechanism = mechanism(&opened, "board");
    let closed = mechanism.closed();
    // Raised at rest angle: the sleeve carries the bolt, and the board
    // with it, straight up.
    assert_eq!(
        mechanism.shape_of(drive(0), &closed),
        MotionShape::Travel {
            toward: Direction::Up,
            travel: Length::from_mm(200),
        }
    );
    // Tilted with the board 100mm up: about the bolt's line where the
    // sleeve has put it, and the board starts 100mm up.
    let raised = [
        DriveValue::Travel(Length::from_mm(100)),
        DriveValue::Turn(Angle::ZERO),
    ];
    let MotionShape::Turn { line, opening } = mechanism.shape_of(drive(1), &raised) else {
        panic!("the tilt turns about one line at any height");
    };
    assert_eq!(line.axis, Axis::Across);
    assert_eq!(line.through.up, Length::from_mm(950));
    assert_eq!(opening, Angle::from_deg(60));
    assert_eq!(
        mechanism.offset_at(&raised),
        Some(Point::default().with(Axis::Up, Length::from_mm(100)))
    );
    // Raised with the board tilted: not one slide of a square board, so
    // the solver poses it.
    let tilted = [
        DriveValue::Travel(Length::ZERO),
        DriveValue::Turn(Angle::from_deg(20)),
    ];
    assert_eq!(mechanism.shape_of(drive(0), &tilted), MotionShape::General);
    assert_eq!(mechanism.offset_at(&tilted), None);
}

#[test]
fn the_board_hits_the_head_rail_only_raised_all_the_way_and_tilted() {
    // Raised all the way, the tilt is swept at that height, and the stile
    // rises into the head rail; at rest height, or raised only part way
    // as the working scene names, it clears it.
    let text = format!(
        "{}\nscene full \"Raised all the way\" {{\n  pose board.height = 200mm\n}}\n",
        fixture("drafting_table")
    );
    let opened = open(&text);
    let codes: Vec<Code> = opened.diagnostics.iter().map(|d| d.code).collect();
    assert_eq!(codes, [Code::Motion], "{}", rendered(&opened));
    let message = &opened.diagnostics[0].message;
    assert!(
        message.contains("hits head_rail when board.tilt is opened past"),
        "{message}"
    );
    insta::assert_snapshot!(rendered(&opened));
}

#[test]
fn a_drive_of_a_member_held_by_a_part_that_stays_is_a_mechanism_error() {
    let text = changed(
        &fixture("drafting_table"),
        "  member sleeve {\n",
        "  member sleeve {\n    with:  piece\n",
    );
    let opened = open(&text);
    assert!(
        opened.diagnostics.iter().any(|d| d.code == Code::Mechanism
            && d.message
                == "rise_and_tilt's drive moves 'sleeve', which is with: a part that stays put"),
        "{}",
        rendered(&opened)
    );
}

#[test]
fn a_frame_carried_two_ways_on_two_members_checks_clean() {
    let opened = open(&fixture("cross_slide"));
    assert!(opened.diagnostics.is_empty(), "{}", rendered(&opened));
    let mechanism = mechanism(&opened, "frame");
    assert_eq!(driven(mechanism, 0), "carriage");
    assert_eq!(driven(mechanism, 1), "stud");
    assert_eq!(mechanism.members()[mechanism.moving().index()].name, "stud");
}

#[test]
fn each_drive_of_the_cross_slide_is_a_slide() {
    let opened = open(&fixture("cross_slide"));
    let mechanism = mechanism(&opened, "frame");
    let closed = mechanism.closed();
    assert_eq!(
        mechanism.shape_of(drive(0), &closed),
        MotionShape::Travel {
            toward: Direction::Up,
            travel: Length::from_mm(250),
        }
    );
    // Back along the carriage, with the carriage held 100mm up: the frame
    // starts 100mm up and slides back.
    let up = [
        DriveValue::Travel(Length::from_mm(100)),
        DriveValue::Travel(Length::ZERO),
    ];
    assert_eq!(
        mechanism.shape_of(drive(1), &up),
        MotionShape::Travel {
            toward: Direction::Back,
            travel: Length::from_mm(250),
        }
    );
    assert_eq!(
        mechanism.offset_at(&up),
        Some(Point::default().with(Axis::Up, Length::from_mm(100)))
    );
    // Both away from closed: one translation, exact.
    let both = [
        DriveValue::Travel(Length::from_mm(100)),
        DriveValue::Travel(Length::from_mm(50)),
    ];
    assert_eq!(
        mechanism.offset_at(&both),
        Some(
            Point::default()
                .with(Axis::Up, Length::from_mm(100))
                .with(Axis::Deep, Length::from_mm(50))
        )
    );
}

#[test]
fn the_frame_hits_the_stop_rail_only_when_it_is_all_the_way_back() {
    let text = format!(
        "{}\nscene back \"All the way back\" {{\n  pose frame.y = 250mm\n}}\n",
        fixture("cross_slide")
    );
    let opened = open(&text);
    let codes: Vec<Code> = opened.diagnostics.iter().map(|d| d.code).collect();
    assert_eq!(codes, [Code::Motion], "{}", rendered(&opened));
    let message = &opened.diagnostics[0].message;
    assert!(
        message.contains("hits stop_rail when frame.x is pulled out past 215mm"),
        "{message}"
    );
    insta::assert_snapshot!(rendered(&opened));
}
