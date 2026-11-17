//! "How it comes apart" read off the exploded poses, and the view's empty
//! states.

use std::path::{Path, PathBuf};

use stave_app::apart::{self, Stage};
use stave_app::check::{self, Checked, Outcome};
use stave_app::empty::{self, Reason};
use stave_app::session::{Command, Event, Session};

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn checked(folder: &Path) -> Checked {
    match check::run(folder) {
        Outcome::Checked(checked) => *checked,
        Outcome::CantOpen { message, .. } => panic!("can't open {}: {message}", folder.display()),
    }
}

fn lines(name: &str) -> Vec<String> {
    let checked = checked(&workspace().join("examples").join(name));
    let (design, graph) = checked.drawable().expect("the design solves");
    let scene = graph.scenes.first().expect("a scene");
    apart::steps(design, graph, scene)
        .into_iter()
        .map(|step| format!("{} {}: {}", step.stage.number(), step.lead, step.text))
        .collect()
}

#[test]
fn the_cabinet_comes_apart_in_two_stages() {
    let lines = lines("clothing_cabinet");
    println!("{lines:#?}");
    assert_eq!(
        lines[0],
        "1 Drawers: slide forward on their slides until they clear the carcass"
    );
    assert_eq!(
        lines[1],
        "1 Doors: come forward off their hinges. Cups go with the door, plates stay on the side"
    );
    assert!(
        lines[2].starts_with("2 Drawer boxes: come apart: "),
        "{}",
        lines[2]
    );
    assert!(
        lines[3].starts_with("2 Door frames: come apart: "),
        "{}",
        lines[3]
    );
    assert!(
        lines[4].starts_with("2 Carcass: comes apart: "),
        "{}",
        lines[4]
    );
    assert_eq!(lines.len(), 5);
}

#[test]
fn a_design_without_assemblies_has_only_its_parts_stage() {
    let checked = checked(&workspace().join("examples/stool"));
    let (design, graph) = checked.drawable().expect("the design solves");
    let steps = apart::steps(design, graph, graph.scenes.first().expect("a scene"));
    println!("{steps:#?}");
    assert!(!steps.is_empty());
    assert!(steps.iter().all(|step| step.stage == Stage::Parts));
    assert_eq!(steps[0].lead, "Carcass");
    assert!(steps[0].text.contains("out of"), "{}", steps[0].text);
}

#[test]
fn the_steps_are_the_same_every_time() {
    assert_eq!(lines("nightstand"), lines("nightstand"));
}

#[test]
fn the_view_can_be_saved_as_gltf() {
    let checked = checked(&workspace().join("examples/stool"));
    let (_, graph) = checked.drawable().expect("the design solves");
    let posed = graph.pose_with(graph.scenes.first().expect("a scene"), &[], 1.0);
    let path = std::env::temp_dir().join(format!("stave-apart-{}.glb", std::process::id()));
    apart::save_glb(graph, &posed, &path).expect("saved");
    let bytes = std::fs::read(&path).expect("read back");
    std::fs::remove_file(&path).ok();
    assert_eq!(bytes.get(..4), Some(&b"glTF"[..]));
}

fn session_for(folder: PathBuf) -> Session {
    let mut session = Session::new(folder.clone());
    let events = session.apply(Command::Check);
    let ticket = events
        .iter()
        .find_map(|event| match event {
            Event::CheckStarted(ticket) => Some(*ticket),
            _ => None,
        })
        .expect("a check starts");
    session.finish_check(ticket, check::run(&folder));
    session
}

#[test]
fn a_library_folder_says_so_instead_of_a_blank_view() {
    let session = session_for(workspace().join("examples/japanese_joinery"));
    let state = empty::of(&session).expect("an empty state");
    assert_eq!(state.reason, Reason::Library);
    assert_eq!(state.title, "Nothing to show");
    assert!(state.description.contains("library"));
    assert!(state.offers_open);
}

#[test]
fn the_empty_state_follows_the_session() {
    let unchecked = Session::new(workspace().join("examples/stool"));
    assert_eq!(
        empty::of(&unchecked).map(|s| s.reason),
        Some(Reason::Checking)
    );
    assert_eq!(
        empty::of(&session_for(workspace().join("examples/stool"))),
        None
    );
    let broken = session_for(workspace().join("tests/broken/unknown_name"));
    assert_eq!(empty::of(&broken).map(|s| s.reason), Some(Reason::Errors));
    let missing = session_for(workspace().join("tests/no_such_folder"));
    assert_eq!(
        empty::of(&missing).map(|s| s.reason),
        Some(Reason::CantOpen)
    );
}
