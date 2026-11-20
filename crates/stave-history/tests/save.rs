//! Saving merges into main; the window catches up when someone else moved
//! main, and a clash with unsaved edits leaves conflict markers.

mod common;

use common::{DRESSER, agent, dresser_folder, path, text, user, with};
use stave_history::{ActorKind, CatchUp, History};

const WIDER: &str = "width = 1000mm\nheight = 1200mm\ndepth = 450mm\n";

/// The agent widens the dresser on main through a suggestion, the way the
/// MCP server does: read main, change it, suggest, apply.
fn agent_widens(folder: &std::path::Path) {
    let mut history = History::open(folder).expect("agent opens");
    let main = history.main().expect("main");
    let files = history.files_at(&main).expect("files at main");
    let changed = with(&files, "dresser.stave", WIDER);
    let id = common::suggest(
        &mut history,
        &agent(),
        None,
        &changed,
        "set width = 1000mm",
        "Widen the dresser",
    )
    .expect("suggest");
    history
        .apply(&id, &agent(), &main, &changed)
        .expect("apply");
}

#[test]
fn save_moves_main_and_records_actor() {
    let folder = dresser_folder();
    let mut history = History::start(folder.path()).expect("start");
    let before = history.main().expect("main");
    let files = history.read_folder().expect("read folder");

    let saved = history.save(&files, &user(), "First cut").expect("save");

    assert_ne!(saved, before);
    assert_eq!(history.main().expect("main"), saved);
    assert_eq!(history.files_at(&saved).expect("files"), files);
    let versions = history.versions(10).expect("versions");
    assert_eq!(versions.len(), 1);
    assert_eq!(versions[0].id, saved);
    assert_eq!(versions[0].actor, user());
    assert_eq!(versions[0].message, "First cut");

    let taller = with(&files, "dresser.stave", "width = 900mm\nheight = 1300mm\n");
    history
        .save(&taller, &agent(), "Make it taller")
        .expect("second save");
    let versions = history.versions(10).expect("versions");
    assert_eq!(versions.len(), 2);
    assert_eq!(versions[0].actor.kind, ActorKind::Agent);
    assert_eq!(versions[0].actor, agent());
    assert_eq!(versions[1].message, "First cut");
}

#[test]
fn catch_up_after_agent_apply_writes_files() {
    let folder = dresser_folder();
    let mut window = History::start(folder.path()).expect("start");
    let files = window.read_folder().expect("read folder");
    window.save(&files, &user(), "First cut").expect("save");
    assert_eq!(
        common::catch_up(&mut window, &files).expect("catch up"),
        CatchUp::UpToDate
    );

    agent_widens(folder.path());

    let CatchUp::Write {
        files: written,
        removed,
        conflicted,
    } = common::catch_up(&mut window, &files).expect("catch up")
    else {
        panic!("main moved, so there is something to write");
    };
    assert_eq!(text(&written, "dresser.stave"), WIDER);
    assert!(removed.is_empty());
    assert!(conflicted.is_empty());

    // Once written, the window is in step with main.
    let now = with(&files, "dresser.stave", WIDER);
    assert_eq!(
        common::catch_up(&mut window, &now).expect("catch up"),
        CatchUp::UpToDate
    );
}

#[test]
fn catch_up_with_unsaved_clash_reports_conflict() {
    let folder = dresser_folder();
    let mut window = History::start(folder.path()).expect("start");
    let files = window.read_folder().expect("read folder");
    window.save(&files, &user(), "First cut").expect("save");

    agent_widens(folder.path());

    // Meanwhile the person changed the same line and hasn't saved.
    let unsaved = with(
        &files,
        "dresser.stave",
        "width = 800mm\nheight = 1200mm\ndepth = 450mm\n",
    );
    let CatchUp::Write {
        files: written,
        conflicted,
        ..
    } = common::catch_up(&mut window, &unsaved).expect("catch up")
    else {
        panic!("main moved, so there is something to write");
    };
    assert_eq!(conflicted, [path("dresser.stave")]);
    let clashing = text(&written, "dresser.stave");
    assert!(clashing.starts_with("<<<<<<<"), "{clashing}");
    // Snapshot style: every side in full, the base after `-------`.
    assert!(clashing.contains("\n+++++++"), "{clashing}");
    assert!(clashing.contains("\n-------"), "{clashing}");
    assert!(clashing.contains("width = 800mm"), "{clashing}");
    assert!(clashing.contains("width = 1000mm"), "{clashing}");
    assert!(clashing.contains("width = 900mm"), "{clashing}");
    assert!(clashing.contains("\n>>>>>>>"), "{clashing}");
    // The lines that didn't clash are merged once, outside the markers.
    assert!(
        clashing.ends_with("height = 1200mm\ndepth = 450mm\n"),
        "{clashing}"
    );
}

#[test]
fn save_after_main_moved_merges_both() {
    let folder = dresser_folder();
    let mut window = History::start(folder.path()).expect("start");
    let files = window.read_folder().expect("read folder");
    window.save(&files, &user(), "First cut").expect("save");

    agent_widens(folder.path());

    // The person changed another line, then saved without catching up.
    let deeper = with(
        &files,
        "dresser.stave",
        "width = 900mm\nheight = 1200mm\ndepth = 500mm\n",
    );
    let saved = window.save(&deeper, &user(), "Deeper").expect("save");

    let main = window.files_at(&saved).expect("files at main");
    assert_eq!(
        text(&main, "dresser.stave"),
        "width = 1000mm\nheight = 1200mm\ndepth = 500mm\n"
    );
    let versions = window.versions(10).expect("versions");
    let messages: Vec<&str> = versions
        .iter()
        .map(|version| version.message.as_str())
        .collect();
    assert_eq!(messages[0], "Deeper");
    assert!(messages.contains(&"First cut"));
    // Main is one line: every revision has one parent.
    assert_eq!(versions.len(), 3, "{messages:?}");

    // Catching up now brings the agent's width into the folder.
    let CatchUp::Write { files: written, .. } =
        common::catch_up(&mut window, &deeper).expect("catch up")
    else {
        panic!("the folder lacks the agent's change");
    };
    assert_eq!(
        text(&written, "dresser.stave"),
        "width = 1000mm\nheight = 1200mm\ndepth = 500mm\n"
    );
    let _unused = DRESSER;
}
