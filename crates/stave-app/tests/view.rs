//! What the viewer's side panel says about the hardware of the examples.

use std::path::{Path, PathBuf};

use stave_app::view::{Item, describe};

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The side panel's rows for a piece of hardware of an example, as
/// `label: value`.
fn rows(example: &str, hardware: &str) -> Vec<String> {
    let opened = stave_app::open(&workspace().join("examples").join(example)).unwrap();
    let design = opened.design.as_ref().expect("the example solves");
    let found = design
        .hardware
        .iter()
        .find(|h| h.name == hardware)
        .unwrap_or_else(|| panic!("no hardware '{hardware}'"));
    describe(design, Item::Hardware(found.id))
        .unwrap()
        .rows
        .iter()
        .map(|row| format!("{}: {}", row.label, row.value))
        .collect()
}

#[test]
fn a_cup_hinge_cup_is_described_as_deep() {
    // A member sunk into the part that carries it is deep; every member is
    // listed by its own name, with what holds it.
    let rows = rows("clothing_cabinet", "door_l.hinges[lower]");
    for row in [
        "Member: cup, cylinder 35 mm across, 12.8 mm deep",
        "Member: plate, box 50 × 36 × 11 mm, fixed to side_l",
        "Member: inner link, bar 1.5 mm thick, moved by the mechanism",
        "Mechanism: 4 pivots; turns up to 110deg",
    ] {
        assert!(rows.contains(&row.to_owned()), "{row}: {rows:?}");
    }
}

#[test]
fn a_butt_hinge_is_two_leaves() {
    let rows = rows("box", "lid.hinges[left]");
    assert!(
        rows.contains(&"Member: leaf, box 40 × 12 × 1.5 mm".to_owned()),
        "{rows:?}"
    );
    assert!(
        rows.contains(&"Member: fixed leaf, box 40 × 12 × 1.5 mm, housed in back".to_owned()),
        "{rows:?}"
    );
    assert!(
        rows.contains(&"Mechanism: 1 pivot; turns up to 180deg".to_owned()),
        "{rows:?}"
    );
}

#[test]
fn a_knob_stands_off_its_face() {
    let rows = rows("nightstand", "drawer.pull");
    assert!(
        rows.contains(&"Member: knob, cylinder 30 mm across, 28 mm long".to_owned()),
        "{rows:?}"
    );
}
