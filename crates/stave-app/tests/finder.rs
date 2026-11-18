//! The marks on the model tree's rows and the open files' tabs, and the
//! finder (Ctrl+K), worked out headless from real projects.

use std::path::{Path, PathBuf};

use stave_app::check::{self, Checked, Outcome};
use stave_app::finder::{self, Goal, Kind, Sources};
use stave_app::problems::{self, Mark};
use stave_app::session::Selection;
use stave_ide::{Analysis, Overlay};
use stave_scene::Item;

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn checked(folder: &Path) -> Checked {
    match check::run(folder) {
        Outcome::Checked(checked) => *checked,
        Outcome::CantOpen { message, .. } => panic!("can't open {}: {message}", folder.display()),
    }
}

fn part_item(checked: &Checked, name: &str) -> Item {
    let design = checked.design().expect("the design solves");
    let part = design
        .parts
        .iter()
        .find(|p| p.name == name)
        .unwrap_or_else(|| panic!("no part '{name}'"));
    Item::Part(part.id)
}

#[test]
fn a_door_that_hits_a_wall_marks_the_door_the_wall_and_the_file() {
    let checked = checked(&workspace().join("tests/broken/door_hits_side"));
    let design = checked.design().expect("it solves");
    let marks = problems::marks(&checked);
    let door = design
        .assemblies
        .iter()
        .find(|a| a.name == "door")
        .expect("a door");
    assert_eq!(marks.assembly(door.id), Some(Mark::Error));
    assert_eq!(
        marks.item(part_item(&checked, "door.panel")),
        Some(Mark::Error)
    );
    assert_eq!(
        marks.item(part_item(&checked, "wall")),
        Some(Mark::Involved)
    );
    let file = checked
        .opened
        .sources
        .files()
        .map(|(_, f)| f.path().to_owned())
        .find(|path| path.ends_with("project.stave"))
        .expect("the project's file");
    assert_eq!(marks.file(&file), Some(Mark::Error));
}

#[test]
fn a_clean_project_has_no_marks() {
    let checked = checked(&workspace().join("examples/stool"));
    let marks = problems::marks(&checked);
    assert_eq!(marks, problems::Marks::default());
}

#[test]
fn marks_keep_the_worst() {
    assert_eq!(Mark::Warning.worse(Mark::Error), Mark::Error);
    assert_eq!(Mark::Involved.worse(Mark::Warning), Mark::Warning);
    assert_eq!(Mark::of(stave_lang::Severity::Note), None);
}

#[test]
fn the_finder_finds_parts_files_names_and_lines() {
    let folder = workspace().join("examples/clothing_cabinet");
    let checked = checked(&folder);
    let analysis = Analysis::load(&folder, &Overlay::new()).expect("the analysis loads");
    let sources = Sources {
        shown: Some(&checked),
        latest: Some(&checked),
        analysis: Some(&analysis),
    };
    let hits = finder::search(sources, "hinge");
    let kinds: Vec<Kind> = hits.iter().map(|h| h.kind).collect();
    let mut sorted = kinds.clone();
    sorted.sort();
    assert_eq!(kinds, sorted, "grouped in order: {hits:#?}");

    let first_part = hits.iter().find(|h| h.kind == Kind::Part).expect("a part");
    assert!(first_part.label.contains("hinges"), "{first_part:?}");
    assert!(matches!(
        first_part.goal,
        Goal::Select(Selection::Hardware(_))
    ));
    assert!(hits.iter().any(|h| h.kind == Kind::Line
        && h.label.contains("hinge_side")
        && h.detail.starts_with("doors.stave:")));
    assert!(
        hits.iter()
            .filter(|h| h.kind == Kind::Line)
            .all(|h| matches!(h.goal, Goal::Open(_)))
    );
    for kind in Kind::ALL {
        assert!(hits.iter().filter(|h| h.kind == kind).count() <= finder::LIMIT_PER_KIND);
    }
    let doors = finder::search(sources, "doors");
    assert!(
        doors
            .iter()
            .any(|h| h.kind == Kind::File && h.label == "doors.stave"),
        "{doors:#?}"
    );
    assert!(finder::search(sources, "  ").is_empty());
}

#[test]
fn the_finder_finds_problems_and_selects_them() {
    let checked = checked(&workspace().join("tests/broken/door_hits_side"));
    let sources = Sources {
        shown: Some(&checked),
        latest: Some(&checked),
        analysis: None,
    };
    let hits = finder::search(sources, "wall");
    let problem = hits
        .iter()
        .find(|h| h.kind == Kind::Problem)
        .expect("the problem");
    assert_eq!(problem.severity, Some(stave_lang::Severity::Error));
    assert!(matches!(problem.goal, Goal::Select(Selection::Problem(_))));
}
