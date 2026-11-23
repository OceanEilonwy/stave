//! Executable spec: running clearance (grammar, "Running clearance").
//! Wood that moves past wood needs a gap greater than the seasons close
//! up, or it binds (`error[motion]`), and a running gap beyond that, or it
//! is tight (`warning[clearance]`). A spacer in a clamp's stack is a
//! bearing, not a rub: a wooden one wears (`warning[clearance]`), a steel
//! one is clean.
//!
//! The corpus is the turning-frames designs that move
//! (`docs/reviews/2026-11-22-turning-frames.md`, appendix): the cheval
//! mirror and the drafting table in `tests/turning/`, and the camp stool
//! and the porch glider (its seat swinging inside its hanger) in
//! `tests/running/`. Each turns on pivot bolts with a 3mm steel washer
//! between the wooden parts, and checks clean. Without the washer, the
//! parts touching, each is an error, with the swell the appendix works
//! out by hand: 0.563mm for two 45mm beech parts, 0.375mm for two 30mm
//! ash legs. The sawhorse doesn't move, so nothing is asked of it.
//!
//! The glider's arms turn beside its posts too, but as drawn the seat
//! bolts' heads sit in that gap, so closing it is an overlap; the tests
//! of an arm against its post sink those heads into the arms first.

mod common;

use std::path::Path;

use common::open_files_with_every_warning;
use stave_app::Opened;
use stave_lang::{Code, Severity, render_diagnostics};
use stave_modules::MemoryFiles;

/// The designs that move: their folder under `tests/`, and the swell of
/// the two parts each pair of bolts holds side by side.
const MOVING: [(&str, &str, &str); 4] = [
    ("turning", "cheval", "0.563mm"),
    ("turning", "drafting_table", "0.563mm"),
    ("running", "stool", "0.375mm"),
    ("running", "glider", "0.563mm"),
];

fn fixture(folder: &str, name: &str) -> String {
    let path = format!("{}/tests/{folder}/{name}.stave", env!("CARGO_MANIFEST_DIR"));
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

/// The design with every washer gap set to `gap` (each component's
/// `washer` param, which places the parts and sizes the washer member).
fn with_gap(text: &str, gap: &str) -> String {
    let mut out = text.to_owned();
    for written in ["param washer  = 3mm", "param washer = 3mm"] {
        out = out.replace(written, &format!("param washer = {gap}"));
    }
    assert_ne!(out, text, "the design has a washer param");
    out
}

/// The design with the parts touching: no gap, and so no washer.
fn touching(text: &str) -> String {
    with_gap(text, "0mm")
}

/// The design with `field` added to its project block.
fn with_project_field(text: &str, field: &str) -> String {
    let start = text.find("project \"").expect("a project block");
    let brace = start + text[start..].find("{\n").expect("its brace") + 2;
    format!("{}  {field}\n{}", &text[..brace], &text[brace..])
}

/// Every diagnostic, with its code and severity.
fn codes(opened: &Opened) -> Vec<(Severity, Code)> {
    opened
        .diagnostics
        .iter()
        .map(|d| (d.severity, d.code))
        .collect()
}

#[test]
fn each_design_with_a_washer_that_fills_the_gap_checks_clean() {
    for (folder, name, _) in MOVING {
        let opened = open(&fixture(folder, name));
        assert!(
            opened.diagnostics.is_empty(),
            "{name}:\n{}",
            rendered(&opened)
        );
    }
}

#[test]
fn parts_touching_as_they_turn_bind() {
    for (folder, name, swell) in MOVING {
        let opened = open(&touching(&fixture(folder, name)));
        let errors: Vec<_> = opened
            .diagnostics
            .iter()
            .filter(|d| d.code == Code::Motion)
            .collect();
        assert!(!errors.is_empty(), "{name}:\n{}", rendered(&opened));
        for error in errors {
            assert!(
                error.message.contains(" rubs ")
                    && error.message.contains(": the gap is 0mm, and the ")
                    && error
                        .message
                        .ends_with(&format!("up to {swell} with the seasons")),
                "{name}: {}",
                error.message
            );
        }
        insta::assert_snapshot!(format!("touching_{name}"), rendered(&opened));
    }
}

#[test]
fn the_swell_is_worked_out_exactly_and_rounded_once() {
    // Two 45mm beech parts: 22.5mm / 80mm x 1mm each, 0.5625mm together,
    // shown to the micrometre.
    let opened = open(&touching(&fixture("turning", "cheval")));
    let first = opened.diagnostics.first().expect("an error");
    assert_eq!(
        first.message,
        "frame.stile_l rubs upright_l the whole way as frame turns: the gap is 0mm, and the beech closes up to 0.563mm with the seasons"
    );
    assert_eq!(
        first.help.as_deref(),
        Some(
            "leave at least 1.563mm between them (0.563mm for the seasons and a running gap of 1mm), and let them bear on steel, such as a washer, rather than on each other"
        )
    );
}

#[test]
fn a_travel_slides_and_a_turn_turns_each_drive_reported() {
    let opened = open(&touching(&fixture("turning", "drafting_table")));
    let text = rendered(&opened);
    assert!(text.contains("as board.height slides"), "{text}");
    assert!(text.contains("as board.tilt turns"), "{text}");
}

#[test]
fn a_seat_swinging_inside_its_hanger_is_measured_against_the_arms() {
    let text = changed(
        &fixture("running", "glider"),
        "    arm_r: arm_r\n  }",
        "    arm_r: arm_r\n    washer: 0mm\n  }",
    );
    let opened = open(&text);
    assert_eq!(
        codes(&opened),
        vec![(Severity::Error, Code::Motion)],
        "{}",
        rendered(&opened)
    );
    let first = opened.diagnostics.first().expect("an error");
    assert!(
        first.message.starts_with(
            "hanger.seat.seat_rail_l rubs hanger.arm_l the whole way as hanger.seat turns"
        ),
        "{}",
        first.message
    );
}

/// The glider with the hanger's `post_gap` (between each arm and its
/// post, and the thickness of the washer there) set to `gap`.
fn with_post_gap(text: &str, gap: &str) -> String {
    changed(
        text,
        "  stand_r: post_r\n}",
        &format!("  stand_r: post_r\n  post_gap: {gap}\n}}"),
    )
}

/// The glider with the seat bolts' heads sunk into the arms: a copy of
/// `pivot_bolt` whose knob sits in a counterbore in the face instead of
/// standing off it, used for the seat bolts. Nothing then sits between
/// an arm and its post, so the gap there can close.
fn with_seat_heads_sunk(text: &str) -> String {
    let start = text.find("kind pivot_bolt : fitting {").expect("the kind");
    let end = start + text[start..].find("\n}\n").expect("its end") + 3;
    let kind = &text[start..end];
    let sunk = changed(
        &changed(
            &changed(kind, "kind pivot_bolt", "kind sunk_pivot_bolt"),
            "cylinder(diameter: knob_diameter, depth: -projection..0mm)",
            "cylinder(diameter: knob_diameter, depth: 0mm..projection)",
        ),
        "  cut bolt_hole:",
        "  cut head_recess: crossing(piece, knob) { drill(diameter: knob_diameter + 1mm, depth: projection, wall: wall) }\n  cut bolt_hole:",
    );
    let text = format!("{}{sunk}\n{}", &text[..end], &text[end..]);
    let text = changed(&text, ": pivot_bolt(arm_l,", ": sunk_pivot_bolt(arm_l,");
    changed(&text, ": pivot_bolt(arm_r,", ": sunk_pivot_bolt(arm_r,")
}

#[test]
fn the_seat_bolts_heads_keep_each_arm_off_its_post() {
    // As drawn, the seat bolts' heads stand 2mm proud of the arms, in the
    // gap between each arm and its post: closing that gap drives them
    // into the posts, which is an overlap, before anything turns.
    let opened = open(&with_post_gap(&fixture("running", "glider"), "0mm"));
    assert_eq!(
        codes(&opened),
        vec![(Severity::Error, Code::Overlap)],
        "{}",
        rendered(&opened)
    );
    let first = opened.diagnostics.first().expect("an error");
    assert!(
        first
            .notes
            .iter()
            .any(|note| note == "hanger.seat.seat_l's knob goes into post_l, 2mm deep at most"),
        "{}",
        rendered(&opened)
    );
}

#[test]
fn seat_bolts_sunk_into_the_arms_check_clean() {
    // The counterbored heads still bear on the arms, so each seat bolt's
    // stack is solid, and the 3mm washers still keep the wood apart.
    let sunk = with_seat_heads_sunk(&fixture("running", "glider"));
    let opened = open(&sunk);
    assert!(opened.diagnostics.is_empty(), "{}", rendered(&opened));
    // The sunk heads still clamp: a washer too thin for the seat's gap
    // leaves air in each seat bolt's stack.
    assert_eq!(sunk.matches("washer_thickness: washer,").count(), 2);
    let thin = sunk.replace(
        "washer_thickness: washer,",
        "washer_thickness: washer - 1mm,",
    );
    let opened = open(&thin);
    let gaps: Vec<_> = opened
        .diagnostics
        .iter()
        .filter(|d| d.code == Code::NoContact)
        .collect();
    assert_eq!(gaps.len(), 2, "{}", rendered(&opened));
    for (gap, side) in gaps.iter().zip(["l", "r"]) {
        assert_eq!(
            gap.message,
            format!(
                "hanger.seat.seat_{side} draws hanger.seat.seat_rail_{side} toward hanger.seat.seat_{side}'s washer, but they are 1mm apart with nothing between"
            )
        );
    }
}

#[test]
fn an_arm_touching_its_post_binds() {
    let text = with_post_gap(&with_seat_heads_sunk(&fixture("running", "glider")), "0mm");
    let opened = open(&text);
    assert_eq!(
        codes(&opened),
        vec![(Severity::Error, Code::Motion)],
        "{}",
        rendered(&opened)
    );
    let first = opened.diagnostics.first().expect("an error");
    assert_eq!(
        first.message,
        "hanger.arm_l rubs post_l the whole way as hanger turns: the gap is 0mm, and the beech closes up to 0.563mm with the seasons"
    );
    assert!(
        first.notes.iter().any(|note| note
            == "the same line also causes: hanger.arm_r rubs post_r the whole way as hanger turns: the gap is 0mm, and the beech closes up to 0.563mm with the seasons"),
        "{}",
        rendered(&opened)
    );
    insta::assert_snapshot!(rendered(&opened));
}

#[test]
fn an_arm_close_to_its_post_is_tight() {
    let text = with_post_gap(
        &with_seat_heads_sunk(&fixture("running", "glider")),
        "1.2mm",
    );
    let opened = open(&text);
    assert_eq!(
        codes(&opened),
        vec![(Severity::Warning, Code::Clearance)],
        "{}",
        rendered(&opened)
    );
    let first = opened.diagnostics.first().expect("a warning");
    assert_eq!(
        first.message,
        "hanger.arm_l passes 1.2mm from post_l the whole way as hanger turns; leave 1.563mm (0.563mm for the seasons and a running gap of 1mm)"
    );
}

#[test]
fn every_arm_touching_both_its_post_and_its_seat_rail_binds_at_both() {
    // Each arm turns beside its post and its seat rail beside it: with
    // both gaps closed, both places rub, each reported.
    let text = with_post_gap(
        &touching(&with_seat_heads_sunk(&fixture("running", "glider"))),
        "0mm",
    );
    let opened = open(&text);
    assert_eq!(
        codes(&opened),
        vec![
            (Severity::Error, Code::Motion),
            (Severity::Error, Code::Motion)
        ],
        "{}",
        rendered(&opened)
    );
    let messages: Vec<_> = opened
        .diagnostics
        .iter()
        .map(|d| d.message.as_str())
        .collect();
    assert!(
        messages
            .iter()
            .any(|m| m.starts_with("hanger.arm_l rubs post_l the whole way as hanger turns")),
        "{messages:?}"
    );
    assert!(
        messages.iter().any(|m| m.starts_with(
            "hanger.seat.seat_rail_l rubs hanger.arm_l the whole way as hanger.seat turns"
        )),
        "{messages:?}"
    );
}

#[test]
fn a_gap_inside_the_running_gap_is_tight() {
    let opened = open(&with_gap(&fixture("turning", "cheval"), "1.2mm"));
    assert_eq!(
        codes(&opened),
        vec![(Severity::Warning, Code::Clearance)],
        "{}",
        rendered(&opened)
    );
    insta::assert_snapshot!(rendered(&opened));
}

#[test]
fn a_gap_inside_the_swell_binds() {
    let opened = open(&with_gap(&fixture("turning", "cheval"), "0.5mm"));
    assert_eq!(
        codes(&opened),
        vec![(Severity::Error, Code::Motion)],
        "{}",
        rendered(&opened)
    );
    let first = opened.diagnostics.first().expect("an error");
    assert!(
        first
            .message
            .contains("the gap is 0.5mm, and the beech closes up to 0.563mm"),
        "{}",
        first.message
    );
    // Not touching: nothing to say about bearing on steel.
    assert_eq!(
        first.help.as_deref(),
        Some(
            "leave at least 1.563mm between them (0.563mm for the seasons and a running gap of 1mm)"
        )
    );
}

#[test]
fn a_running_gap_of_nothing_leaves_only_the_seasons() {
    let tight = with_gap(&fixture("turning", "cheval"), "1.2mm");
    let opened = open(&with_project_field(&tight, "running_gap: 0mm"));
    assert!(opened.diagnostics.is_empty(), "{}", rendered(&opened));
    // Touching still binds: the seasons close it up.
    let touching = touching(&fixture("turning", "cheval"));
    let opened = open(&with_project_field(&touching, "running_gap: 0mm"));
    assert_eq!(
        codes(&opened),
        vec![(Severity::Error, Code::Motion)],
        "{}",
        rendered(&opened)
    );
    let help = opened.diagnostics.first().and_then(|d| d.help.as_deref());
    assert!(
        help.is_some_and(|help| help.starts_with(
            "leave at least 0.563mm between them (0.563mm for the seasons and a running gap of 0mm)"
        )),
        "{help:?}"
    );
}

#[test]
fn a_larger_running_gap_asks_for_more() {
    let opened = open(&with_project_field(
        &fixture("turning", "cheval"),
        "running_gap: 3mm",
    ));
    assert_eq!(
        codes(&opened),
        vec![(Severity::Warning, Code::Clearance)],
        "{}",
        rendered(&opened)
    );
    let first = opened.diagnostics.first().expect("a warning");
    assert_eq!(
        first.message,
        "frame.stile_l passes 3mm from upright_l the whole way as frame turns; leave 3.563mm (0.563mm for the seasons and a running gap of 3mm)"
    );
}

#[test]
fn a_negative_running_gap_is_out_of_range() {
    let opened = open(&with_project_field(
        &fixture("turning", "cheval"),
        "running_gap: -1mm",
    ));
    let first = opened.diagnostics.first().expect("an error");
    assert_eq!(first.code, Code::OutOfRange, "{}", rendered(&opened));
    assert_eq!(
        first.message,
        "'running_gap' must be 0mm or more, found -1mm"
    );
}

#[test]
fn a_wooden_spacer_is_a_bearing_that_wears() {
    let opened = open(&fixture("running", "disc_cheval"));
    assert_eq!(
        codes(&opened),
        vec![
            (Severity::Warning, Code::Clearance),
            (Severity::Warning, Code::Clearance)
        ],
        "{}",
        rendered(&opened)
    );
    insta::assert_snapshot!(rendered(&opened));
}

#[test]
fn a_part_that_touches_more_than_its_stack_is_not_a_spacer() {
    // A chock set under the left disc touches it: the disc now touches
    // something besides its two neighbours in the stack, so it is no
    // spacer, and the stile turning on it rubs it.
    let text = changed(
        &fixture("running", "disc_cheval"),
        "part spacer_r = mirror(spacer_l, about: mid(envelope, across))",
        "part spacer_r = mirror(spacer_l, about: mid(envelope, across))

stock beech_3 { species: beech, form: solid, thickness: 3mm, max_width: 200mm, max_length: 2400mm }

part chock : beech_3 {
  length: 30mm
  width:  20mm
  place {
    orient(length: back, width: up)
    against(upright_l.right)
    center(upright_l, deep)
    against(spacer_l.bottom)
  }
}",
    );
    let opened = open(&text);
    let text = rendered(&opened);
    assert!(
        opened.diagnostics.iter().any(
            |d| d.code == Code::Motion && d.message.starts_with("frame.stile_l rubs spacer_l ")
        ),
        "{text}"
    );
    assert!(
        !opened
            .diagnostics
            .iter()
            .any(|d| d.message.starts_with("spacer_l is wood")),
        "{text}"
    );
    // The other disc is still a spacer.
    assert!(
        opened
            .diagnostics
            .iter()
            .any(|d| d.message.starts_with("spacer_r is wood")),
        "{text}"
    );
}

#[test]
fn the_sawhorse_does_not_move_so_nothing_is_asked() {
    // Touching is how a fixed extension is built: clean.
    let text = fixture("clamps", "sawhorse")
        .replace(", gap: 3mm)", ")")
        .replace("span: 3mm", "span: 0mm");
    let opened = open(&text);
    assert!(
        !opened
            .diagnostics
            .iter()
            .any(|d| d.code == Code::Clearance || d.code == Code::Motion),
        "{}",
        rendered(&opened)
    );
}

/// `examples/clothing_cabinet_from_box` with `old` replaced by `new`.
fn cabinet(old: &str, new: &str) -> Opened {
    let folder =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/clothing_cabinet_from_box");
    let text = std::fs::read_to_string(folder.join("cabinet.stave")).expect("the cabinet");
    let mut memory = MemoryFiles::new();
    memory.insert(
        Path::new("/cabinet/cabinet.stave"),
        changed(&text, old, new),
    );
    stave_app::open_with(Path::new("/cabinet"), &memory).expect("the cabinet loads")
}

#[test]
fn overlay_fronts_seated_on_the_carcass_are_not_rubbing() {
    // The doors close onto the carcass front and swing away from it; the
    // drawer fronts rest on it and pull straight away: seating.
    let opened = cabinet("meeting: -2.5mm)", "meeting: -2.5mm)");
    assert!(opened.diagnostics.is_empty(), "{}", rendered(&opened));
}

#[test]
fn drawer_fronts_too_close_for_the_oak_bind() {
    // A 1.5mm reveal between two oak fronts about 250mm tall, which close
    // up 3.2mm with the seasons.
    let opened = cabinet(
        "top: 8mm, bottom: 8mm, meeting: -2.5mm)",
        "top: 8mm, bottom: 8mm, meeting: -0.75mm)",
    );
    let errors: Vec<_> = opened
        .diagnostics
        .iter()
        .filter(|d| d.code == Code::Motion)
        .collect();
    let first = errors.first().expect("an error");
    assert!(
        first.message.starts_with("drawer[bottom].front rubs drawer[top].front as drawer[bottom] starts to slide: the gap is 1.5mm, and the white_oak closes up to "),
        "{}",
        first.message
    );
}
