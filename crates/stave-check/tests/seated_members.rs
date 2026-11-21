//! Seated members lie on the wood (grammar, "Cuts stay in their part",
//! `error[overhang]`): a member housed in a cut, or placed against its
//! part by its own `mount`, lies wholly on its part, for every kind and
//! with no rule in the kind. Each fixture checks clean as it is (a cup
//! hinge's arm, which is not seated, reaches past the cabinet side and is
//! still clean), and with one line changed so a seated member hangs part
//! way off its part, it is `error[overhang]` at the hardware's call,
//! saying how much of the member the part holds.

mod common;

use common::check;
use stave_lang::{Code, Diagnostic};

fn fixture(name: &str) -> String {
    let path = format!(
        "{}/tests/fixtures/seated/{name}.stave",
        env!("CARGO_MANIFEST_DIR")
    );
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"))
}

/// The fixture with `from` (written once) changed to `to`.
fn changed(name: &str, from: &str, to: &str) -> String {
    let text = fixture(name);
    assert_eq!(text.matches(from).count(), 1, "{name}: {from}");
    text.replace(from, to)
}

/// The one error a design has.
fn only_error(text: &str) -> Diagnostic {
    let checked = check(text);
    let errors: Vec<&Diagnostic> = checked
        .diagnostics
        .iter()
        .filter(|d| d.is_error())
        .collect();
    let [error] = errors.as_slice() else {
        panic!("{errors:#?}");
    };
    (*error).clone()
}

/// The one error, which must be `error[overhang]` with this message and
/// note.
fn overhang(text: &str, message: &str, note: &str) {
    let error = only_error(text);
    assert_eq!(error.code, Code::Overhang, "{error:#?}");
    assert_eq!(error.message, message);
    assert_eq!(error.notes, vec![note.to_owned()]);
}

#[test]
fn the_fixtures_check_clean() {
    for name in [
        "euro_hinge_plate",
        "wide_hinge_plate",
        "butt_hinge_leaf",
        "slide",
        "bar_pull",
    ] {
        let checked = check(&fixture(name));
        let messages: Vec<String> = checked
            .diagnostics
            .iter()
            .map(|d| format!("{}[{}] {}", d.severity, d.code, d.message))
            .collect();
        assert!(messages.is_empty(), "{name}: {messages:#?}");
    }
}

/// A door overlaying the cabinet side, its top hinge 2mm from its top: the
/// mounting plate, placed by its own `mount`, runs 30.5mm past the top of
/// the side. It makes no cut, so only the backstop sees it.
#[test]
fn a_cup_hinge_plate_off_the_end_of_its_side() {
    for name in ["euro_hinge_plate", "wide_hinge_plate"] {
        let text = changed(
            name,
            "flush(panel.top, inset: 80mm)",
            "flush(panel.top, inset: 2mm)",
        );
        overhang(
            &text,
            "door.hinge_high: its plate hangs off the end of side_l",
            "side_l holds 19.5mm of the 50mm plate",
        );
    }
}

/// A lid 20mm shorter than the wall, the hinges laid out from the wall's
/// ends: the leaf housed in the lid has half its length over air.
#[test]
fn a_butt_hinge_leaf_off_the_end_of_its_lid() {
    let text = changed("butt_hinge_leaf", "length: 875mm", "length: 855mm");
    overhang(
        &text,
        "hinges[left]: its leaf hangs off the end of lid",
        "lid holds 20mm of the 40mm leaf",
    );
}

/// A 500mm slide, whose drawer member its own `mount` places against the
/// drawer side, on a drawer side 300mm long, and on one 25mm tall.
#[test]
fn a_slide_off_its_drawer_side() {
    let longer = changed(
        "slide",
        "  length: 500mm\n  width:  100mm",
        "  length: 300mm\n  width:  100mm",
    );
    overhang(
        &longer,
        "slide: its drawer_member hangs off the end of drawer_side",
        "drawer_side holds 300mm of the 500mm drawer_member",
    );
    let taller = changed(
        "slide",
        "  length: 500mm\n  width:  100mm",
        "  length: 500mm\n  width:  25mm",
    );
    overhang(
        &taller,
        "slide: its drawer_member hangs off the edge of drawer_side",
        "drawer_side holds 25mm of the 45mm drawer_member",
    );
}

/// A bar pull run off the end of the door: its bar is not seated and may
/// stand past the edge (the overlap checks still see it), but its screws
/// are housed in their holes, so the left one may not run off the end.
#[test]
fn a_bar_pull_screw_off_the_end_of_its_door() {
    let text = changed(
        "bar_pull",
        "center(door, across)",
        "flush(door.left, inset: -16mm)",
    );
    let error = only_error(&text);
    assert_eq!(error.code, Code::Overhang, "{error:#?}");
    assert_eq!(
        error.message,
        "handle: the left shape of its screws hangs off the end of door"
    );
}

/// Further off, its left screw misses the door altogether while the right
/// one is in it: the error names the shape that misses.
#[test]
fn a_shape_that_misses_its_part_is_named() {
    let text = changed(
        "bar_pull",
        "center(door, across)",
        "flush(door.left, inset: -30mm)",
    );
    let error = only_error(&text);
    assert_eq!(error.code, Code::NoContact, "{error:#?}");
    assert_eq!(
        error.message,
        "handle: the left shape of screws meets none of the parts it should join"
    );
}
