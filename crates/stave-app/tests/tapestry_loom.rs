//! The tapestry loom, `examples/tapestry_loom`: the design the turning
//! frames review set out to write (`docs/reviews/2026-11-22-turning-frames.md`).
//! Its frame turns all the way round on a pivot bolt each side and is set
//! at one of four heights in a row of holes; the washers fill the gap
//! between the stand and the frame, and the check sweeps the frame round
//! at every height.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use stave_app::Opened;
use stave_lang::units::{Angle, Length};
use stave_lang::{Code, render_diagnostics};
use stave_model::mechanism::{DriveIndex, DriveValue, Mechanism, MechanismDrive};
use stave_modules::MemoryFiles;

fn folder() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/tapestry_loom")
}

fn text() -> String {
    let path = folder().join("loom.stave");
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// The loom with `old` (written exactly once) replaced by `new`.
fn changed(old: &str, new: &str) -> Opened {
    let text = text();
    assert_eq!(text.matches(old).count(), 1, "{old}");
    let mut memory = MemoryFiles::new();
    memory.insert(Path::new("/loom/loom.stave"), text.replace(old, new));
    stave_app::open_with(Path::new("/loom"), &memory).expect("the loom loads")
}

fn rendered(opened: &Opened) -> String {
    render_diagnostics(&opened.sources, &opened.diagnostics)
}

fn frame_mechanism(opened: &Opened) -> &Mechanism {
    opened
        .design
        .as_ref()
        .expect("a solved design")
        .assemblies()
        .iter()
        .find(|assembly| assembly.name == "frame")
        .and_then(|assembly| assembly.mechanism.as_ref())
        .expect("the frame moves")
}

#[test]
fn the_loom_checks_clean_turning_all_the_way_round_at_four_heights() {
    let opened = stave_app::open(&folder()).expect("the loom loads");
    assert!(opened.diagnostics.is_empty(), "{}", rendered(&opened));
    let mechanism = frame_mechanism(&opened);
    let tilt = mechanism
        .drive_at(DriveIndex::new(0).unwrap())
        .expect("the tilt");
    assert!(
        matches!(tilt.drive, MechanismDrive::Turn { to, .. } if to == Angle::from_deg(360)),
        "the frame turns all the way round"
    );
    assert!(tilt.set_at.is_none(), "the tilt is set anywhere");
    let height = mechanism
        .drive_at(DriveIndex::new(1).unwrap())
        .expect("the height");
    assert_eq!(
        height.set_at.as_ref().expect("set in holes").values(),
        [0, -50, -100, -150].map(|v| DriveValue::Travel(Length::from_mm(v)))
    );
}

#[test]
fn a_fifth_hole_lets_the_frame_hit_the_stretcher() {
    // The frame's lower end sweeps a circle about the pivots, about 731mm
    // in radius (the bolt to the stile's far corner): 50mm lower and it
    // reaches the stretcher's top edge, 240mm off the floor.
    let opened = changed("param holes_below = 3", "param holes_below = 4");
    let codes: Vec<Code> = opened.diagnostics.iter().map(|d| d.code).collect();
    assert_eq!(codes, [Code::Motion], "{}", rendered(&opened));
    assert!(
        opened.diagnostics[0].message.contains("hits stretcher"),
        "{}",
        rendered(&opened)
    );
}

#[test]
fn the_loom_checks_in_a_bounded_time() {
    // Measured on 2026-11-22 on a desktop: `stave check
    // examples/tapestry_loom` takes about 27ms in release and 310ms to
    // 380ms in debug, and this open about 300ms in the debug test build.
    // The bound is ten times the debug figure, so a busy CI machine
    // doesn't flake but a sweep that runs away does.
    const BOUND: Duration = Duration::from_secs(3);
    let started = Instant::now();
    let opened = stave_app::open(&folder()).expect("the loom loads");
    let taken = started.elapsed();
    assert!(opened.diagnostics.is_empty(), "{}", rendered(&opened));
    eprintln!("the loom checked in {taken:?}");
    assert!(
        taken < BOUND,
        "the loom took {taken:?} to check, more than {BOUND:?}"
    );
}
