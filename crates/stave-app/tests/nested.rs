//! Executable spec: nested assemblies are swept (grammar, "Scenes",
//! motions are made one at a time; nested assemblies are swept the same
//! way).
//!
//! The corpus is `tests/turning/tray_drawer.stave`: a drawer, a frame
//! between two cabinet sides, slides out on a stud along each side, and a
//! tray inside it turns up on a pivot bolt through each of the drawer's
//! sides. The tray is an assembly inside the drawer's. Its scenes pose the
//! drawer out, the tray up, and both.

mod common;

use common::open_files_with_every_warning;
use stave_app::Opened;
use stave_lang::{Code, render_diagnostics};

fn fixture() -> String {
    let path = format!(
        "{}/tests/turning/tray_drawer.stave",
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

/// The design without its scenes, with `scenes` instead.
fn with_scenes(scenes: &str) -> String {
    let text = fixture();
    let first = text.find("scene out").expect("the scenes");
    format!("{}\n{scenes}\n", text[..first].trim_end())
}

fn codes(opened: &Opened) -> Vec<Code> {
    opened.diagnostics.iter().map(|d| d.code).collect()
}

#[test]
fn a_tray_tilting_in_a_drawer_checks_clean() {
    // The tray is swept with the drawer closed and out as the scenes
    // pose it; the drawer is swept carrying the tray closed, up and half
    // up.
    let opened = open(&fixture());
    assert!(opened.diagnostics.is_empty(), "{}", rendered(&opened));
    let design = opened.design.as_ref().unwrap();
    let tray = design
        .assemblies()
        .iter()
        .find(|assembly| assembly.name == "drawer.tray")
        .expect("the tray");
    let drawer = design
        .assemblies()
        .iter()
        .find(|assembly| assembly.name == "drawer")
        .expect("the drawer");
    assert_eq!(tray.parent, Some(drawer.id));
    assert!(tray.mechanism.is_some() && drawer.mechanism.is_some());
}

#[test]
fn the_tray_tilted_with_the_drawer_closed_hits_a_rail() {
    // A rail between the sides just above the tray's back: the tray turns
    // up into it with the drawer closed, swept from rest with the drawer
    // held closed (no scene: a scene posing the tray up would find the
    // rail in its way there already). An assembly inside a moving one used
    // to be left unswept.
    let text = changed(
        &with_scenes(""),
        "joint stretcher_tenons :",
        "part back_rail : beech_25 {\n  length: fit\n  width:  60mm\n  place {\n    orient(length: right, width: up)\n    flush(side_l.bottom, inset: 890mm)\n    center(envelope, across)\n    flush(side_l.front, inset: 410mm)\n  }\n}\n\njoint back_tenons : mortise_tenon(back_rail, into: [side_l, side_r], length: 30mm)\njoint stretcher_tenons :",
    );
    let opened = open(&text);
    assert_eq!(codes(&opened), [Code::Motion], "{}", rendered(&opened));
    let message = &opened.diagnostics[0].message;
    assert!(
        message.contains("hits back_rail when drawer.tray is opened past"),
        "{message}"
    );
    insta::assert_snapshot!(rendered(&opened));
}

#[test]
fn the_drawer_slid_out_with_the_tray_up_hits_the_front_rail() {
    // The front rail lowered: the tray lying flat passes under it, but a
    // scene poses the tray up, and the drawer, swept carrying it up, takes
    // it into the rail.
    let text = changed(
        &with_scenes("scene tilted \"Tray up\" {\n  pose drawer.tray = 90deg\n}"),
        "    flush(side_l.bottom, inset: 950mm)\n",
        "    flush(side_l.bottom, inset: 870mm)\n",
    );
    let opened = open(&text);
    assert_eq!(codes(&opened), [Code::Motion], "{}", rendered(&opened));
    let diagnostic = &opened.diagnostics[0];
    assert!(
        diagnostic
            .message
            .contains("hits front_rail when drawer is pulled out past"),
        "{}",
        diagnostic.message
    );
    assert!(
        diagnostic
            .notes
            .iter()
            .any(|note| note == "while drawer.tray is open 90deg"),
        "{}",
        rendered(&opened)
    );
    insta::assert_snapshot!(rendered(&opened));
    // Without the scene the tray is only ever flat as the drawer slides:
    // clean.
    let flat = changed(
        &with_scenes(""),
        "    flush(side_l.bottom, inset: 950mm)\n",
        "    flush(side_l.bottom, inset: 870mm)\n",
    );
    let opened = open(&flat);
    assert!(opened.diagnostics.is_empty(), "{}", rendered(&opened));
}
