//! The text and the 3D view share one selection; checking reads what the
//! editor shows.

use std::path::{Path, PathBuf};

use stave_app::check::{self, Outcome};
use stave_app::link;
use stave_app::session::{Command, Event, Selection, Session};
use stave_ide::{Analysis, Buffers, Document, FilePosition, Overlay};

fn cabinet() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/clothing_cabinet")
        .canonicalize()
        .unwrap()
}

fn checked(folder: &Path) -> Box<check::Checked> {
    match check::run(folder) {
        Outcome::Checked(checked) => checked,
        Outcome::CantOpen { message, .. } => panic!("{message}"),
    }
}

/// The symbol at the first `needle` on 1-based `line` of `file`.
fn symbol_at(
    analysis: &Analysis,
    folder: &Path,
    file: &str,
    line: usize,
    needle: &str,
) -> stave_ide::SymbolId {
    // `std/casework/...` is a standard file; anything else is in the
    // project's folder.
    let document = if file.starts_with("std/") {
        Document::from_source_path(file)
    } else {
        Document::File(folder.join(file))
    };
    let id = analysis.file(&document).unwrap();
    let start = analysis.line_start(id, line).unwrap();
    let text = &analysis.text(id).unwrap()[usize::try_from(start).unwrap()..];
    let column = text.lines().next().unwrap().find(needle).unwrap();
    analysis
        .symbol_at(FilePosition {
            file: id,
            offset: start + u32::try_from(column).unwrap(),
        })
        .unwrap()
}

#[expect(
    clippy::wildcard_enum_match_arm,
    reason = "a test picks out the variant it checks, and any other is a failure or not what it counts"
)]
fn name(design: &stave_model::solved::SolvedDesign, selection: &Selection) -> String {
    match selection {
        Selection::Part(id) => design.part(*id).unwrap().name.clone(),
        Selection::Hardware(id) => design.hardware()[id.index()].name.clone(),
        Selection::Assembly(id) => design.assemblies()[id.index()].name.clone(),
        other => format!("{other:?}"),
    }
}

#[test]
fn the_cursor_on_a_name_selects_it_in_3d() {
    let folder = cabinet();
    let checked = checked(&folder);
    let design = checked.design().unwrap();
    let analysis = Analysis::load(&folder, &Overlay::new()).unwrap();
    let select = |file: &str, line: usize, needle: &str, current: Option<&Selection>| {
        let symbol = symbol_at(&analysis, &folder, file, line, needle);
        link::selection_for(design, &analysis, symbol, current).map(|s| name(design, &s))
    };
    assert_eq!(
        select("doors.stave", 9, "door_l", None).as_deref(),
        Some("door_l")
    );
    assert_eq!(
        select("doors.stave", 18, "door_l", None).as_deref(),
        Some("door_l")
    );
    assert_eq!(
        select("doors.stave", 11, "side_l", None).as_deref(),
        Some("side_l")
    );
    // An array selects its first element; a label, that element.
    assert_eq!(
        select("carcass.stave", 79, "foot", None).as_deref(),
        Some("foot[front_left]")
    );
    assert_eq!(
        select("carcass.stave", 79, "back_left", None).as_deref(),
        Some("foot[back_left]")
    );
    // A part of a component: in the selected door if it is one, else in
    // the first door.
    assert_eq!(
        select("std/casework/frame_panel_door.stave", 21, "stile", None).as_deref(),
        Some("door_l.stile[hinge]")
    );
    let door_r_panel = design
        .parts()
        .iter()
        .find(|p| p.name == "door_r.panel")
        .map(|p| Selection::Part(p.id))
        .unwrap();
    assert_eq!(
        select(
            "std/casework/frame_panel_door.stave",
            21,
            "stile",
            Some(&door_r_panel)
        )
        .as_deref(),
        Some("door_r.stile[hinge]")
    );
    // A param or a stock is nothing the 3D view shows.
    assert_eq!(select("doors.stave", 13, "oak_19", None), None);
}

#[test]
fn selecting_in_3d_moves_the_text_to_the_line_that_places_it() {
    let folder = cabinet();
    let checked = checked(&folder);
    let design = checked.design().unwrap();
    let stile = design
        .parts()
        .iter()
        .find(|p| p.name == "door_l.stile[hinge]")
        .map(|p| Selection::Part(p.id))
        .unwrap();
    let location = link::placed_at(&checked, &stile).unwrap();
    assert_eq!(location.display(&folder), "doors.stave:9");
    assert_eq!(
        link::chip(design, &stile, &location).as_deref(),
        Some("door_l.stile[hinge] is written on line 9")
    );
    let side = design
        .parts()
        .iter()
        .find(|p| p.name == "side_l")
        .map(|p| Selection::Part(p.id))
        .unwrap();
    assert_eq!(
        link::placed_at(&checked, &side).unwrap().display(&folder),
        format!(
            "carcass.stave:{}",
            std::fs::read_to_string(folder.join("carcass.stave"))
                .unwrap()
                .lines()
                .position(|l| l.starts_with("part side_l"))
                .unwrap()
                + 1
        )
    );
}

#[test]
fn following_the_cursor_leaves_no_history() {
    let folder = cabinet();
    let mut session = Session::new(folder.clone());
    #[expect(
        clippy::wildcard_enum_match_arm,
        reason = "a test picks out the variant it checks, and any other is a failure or not what it counts"
    )]
    let ticket = session
        .apply(Command::Check)
        .into_iter()
        .find_map(|e| match e {
            Event::CheckStarted(ticket) => Some(ticket),
            _ => None,
        })
        .unwrap();
    session.finish_check(ticket, check::run(&folder));
    let back_before = session.can_go_back();
    let (design, _) = session.drawable().unwrap();
    let door = Selection::Assembly(design.assemblies()[0].id);
    let events = session.apply(Command::Link(Some(door.clone())));
    assert_eq!(events, vec![Event::SelectionChanged]);
    assert_eq!(session.selection(), Some(&door));
    assert_eq!(session.can_go_back(), back_before);
    // The same again changes nothing.
    assert!(session.apply(Command::Link(Some(door))).is_empty());
}

#[test]
fn checking_reads_unsaved_buffers() {
    let folder = cabinet();
    let doors = Document::File(folder.join("doors.stave"));
    let mut buffers = Buffers::new();
    let text = buffers.open(&doors).unwrap().to_owned();
    buffers
        .edit(
            &doors,
            &text.replace("hinge_side:  side_l", "hinge_side:  side_q"),
        )
        .unwrap();
    let Outcome::Checked(typed) = check::run_with(&folder, &buffers.overlay()) else {
        panic!("the folder opens");
    };
    assert!(typed.summary.errors > 0, "the unsaved typo is checked");
    // The file on disk is untouched and still checks clean.
    let Outcome::Checked(saved) = check::run_with(&folder, &Overlay::new()) else {
        panic!("the folder opens");
    };
    assert_eq!(saved.summary.errors, 0);
}
