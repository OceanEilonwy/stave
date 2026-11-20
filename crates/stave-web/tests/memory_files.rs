//! The viewer runs the pipeline on a project's text held in memory, as it
//! does in the browser, and builds the same scene the window builds from
//! the folder on disk.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use stave_web::{Files, Project, SuggestionVerdict};

fn example(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples")
        .join(name)
}

/// Every file of an example folder, keyed by its name in the folder.
fn read_files(folder: &Path) -> BTreeMap<String, String> {
    let mut files = BTreeMap::new();
    for entry in fs::read_dir(folder).unwrap() {
        let path = entry.unwrap().path();
        if path.is_file() {
            let name = path.file_name().unwrap().to_str().unwrap().to_owned();
            files.insert(name, fs::read_to_string(&path).unwrap());
        }
    }
    files
}

#[test]
fn loads_box_from_memory_files() {
    let folder = example("box");
    let files = Files::new("/p", read_files(&folder)).unwrap();
    let project = Project::open(&files, None).unwrap();
    let graph = project.graph().expect("the box places");

    let on_disk = stave_app::open(&folder).unwrap();
    let expected = stave_app::view::scene(on_disk.design.as_ref().unwrap());

    assert!(!expected.nodes.is_empty());
    assert_eq!(graph.nodes.len(), expected.nodes.len());
    assert_eq!(project.error_count(), on_disk.error_count());
    assert_eq!(project.verdict(), None);
    assert!(project.ghost_nodes().is_empty());
}

/// The box with its `box_height` set to `height`, as a suggestion's files.
fn taller_box(height: &str) -> Files {
    let text = fs::read_to_string(example("box").join("box.stave")).unwrap();
    let changed = text.replace(
        "param box_height = 110mm within 80mm..200mm",
        &format!("param box_height = {height} within 80mm..200mm"),
    );
    assert_ne!(text, changed);
    Files::new("/p", BTreeMap::from([("box.stave".to_owned(), changed)])).unwrap()
}

#[test]
fn a_suggestion_that_checks_clean_is_drawn_as_ghosts_that_fit() {
    let files = Files::new("/p", read_files(&example("box"))).unwrap();
    let suggestion = taller_box("150mm");
    let project = Project::open(&files, Some(&suggestion)).unwrap();
    assert_eq!(project.verdict(), Some(SuggestionVerdict::Fits));
    assert_eq!(project.suggestion_error_count(), Some(0));
    // The taller sides are ghosts; the lid's panel isn't changed.
    let ghosts = project.ghost_nodes();
    let graph = project.graph().unwrap();
    let names: Vec<&str> = ghosts
        .iter()
        .map(|id| graph.node(*id).unwrap().name.as_str())
        .collect();
    assert!(names.contains(&"end_l"), "{names:?}");
    assert!(ghosts.len() < graph.nodes.len());
}

#[test]
fn a_suggestion_with_errors_is_refused() {
    let files = Files::new("/p", read_files(&example("box"))).unwrap();
    // Out of the parameter's range.
    let suggestion = taller_box("400mm");
    let project = Project::open(&files, Some(&suggestion)).unwrap();
    assert_eq!(project.verdict(), Some(SuggestionVerdict::Refused));
    assert!(project.suggestion_error_count().unwrap() > 0);
    // The project as it is still draws.
    assert!(project.graph().is_some());
}

#[test]
fn the_box_lid_is_a_motion() {
    let files = Files::new("/p", read_files(&example("box"))).unwrap();
    let project = Project::open(&files, None).unwrap();
    let motions = project.motions();
    assert_eq!(motions.len(), 1, "{motions:?}");
    assert!(motions[0].range.turns());
}
