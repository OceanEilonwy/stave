//! The design corpus under `tests/designs/`: small whole projects that
//! show one feature working, end to end. Every one checks with no
//! diagnostic at all, and the tests below say what each one shows.

mod common;

use std::path::{Path, PathBuf};

use common::{mm, part};
use stave_lang::render_diagnostics;
use stave_model::solved::SolvedDesign;

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Opens a design of the corpus; panics unless it checks with no
/// diagnostic.
fn clean(name: &str) -> stave_app::Opened {
    let opened = stave_app::open(&workspace().join("tests/designs").join(name)).unwrap();
    assert!(
        opened.diagnostics.is_empty(),
        "{name}:\n{}",
        render_diagnostics(&opened.sources, &opened.diagnostics)
    );
    opened
}

fn solved(opened: &stave_app::Opened) -> &SolvedDesign {
    opened.design.as_ref().expect("a solved design")
}

#[test]
fn every_design_checks_with_no_diagnostic() {
    let mut checked = 0;
    let mut folders: Vec<PathBuf> = std::fs::read_dir(workspace().join("tests/designs"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.is_dir())
        .collect();
    folders.sort();
    for folder in folders {
        let name = folder.file_name().unwrap().to_string_lossy().into_owned();
        clean(&name);
        checked += 1;
    }
    assert!(checked >= 1, "no designs under tests/designs");
}

#[test]
fn unglued_top_is_three_boards_in_tongue_and_groove_held_by_battens() {
    let opened = clean("unglued_top");
    let design = solved(&opened);
    let top = part(design, "top");
    let boards = top.boards.as_ref().expect("the top is made of boards");
    // 420mm across the grain from boards 150mm wide at most: three of 140mm,
    // the first two cut wider by the tongue (19 / 3 mm).
    let widths: Vec<_> = boards.boards.iter().map(|b| b.width).collect();
    assert_eq!(widths, [mm("146.333333"), mm("146.333333"), mm("140")]);
    assert_eq!(boards.seams.len(), 2);
    let methods: Vec<_> = boards
        .methods
        .iter()
        .map(|m| (m.name.as_str(), m.glue))
        .collect();
    assert_eq!(methods, [("tongue_and_groove", false)]);
    // Nothing in the panel is glued: not the seams, not the battens.
    let screws = design
        .joints
        .iter()
        .find(|j| j.name == "batten_screws")
        .unwrap();
    assert!(!screws.glue);
    // The tongue and groove makes no part and uses no hardware.
    assert!(design.made_parts().next().is_none());
    assert_eq!(design.fasteners.len(), 1);
}
