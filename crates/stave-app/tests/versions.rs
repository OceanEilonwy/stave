//! The window's versions, headless: saving merges into main, catching up
//! with main writes only what no unsaved edit holds, the question about
//! turning versions on, and applying a suggestion from the window.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use stave_app::versions::{
    self, Actor, ActorKind, CatchUpWhen, CaughtUp, Declined, History, OnSave, ProjectFiles,
    RelativePath, VersionsError, VersionsStatus,
};

fn user() -> Actor {
    versions::user_actor("Henry")
}

fn agent() -> Actor {
    Actor {
        kind: ActorKind::Agent,
        name: "Claude Desktop".to_owned(),
        id: "3f2a".to_owned(),
    }
}

fn path(text: &str) -> RelativePath {
    RelativePath::new(Path::new(text)).expect("a path inside the project")
}

fn write(folder: &Path, relative: &str, text: &str) {
    let full = folder.join(relative);
    if let Some(parent) = full.parent() {
        std::fs::create_dir_all(parent).expect("make directories");
    }
    std::fs::write(full, text).expect("write file");
}

fn read(folder: &Path, relative: &str) -> String {
    std::fs::read_to_string(folder.join(relative)).expect("read file")
}

fn text(files: &ProjectFiles, relative: &str) -> String {
    String::from_utf8(files.get(&path(relative)).expect("present").to_vec()).expect("utf-8")
}

fn with(files: &ProjectFiles, relative: &str, text: &str) -> ProjectFiles {
    let mut changed = files.clone();
    changed.insert(path(relative), text.as_bytes().to_vec());
    changed
}

/// Two plain text files, so main can change one while the window holds the
/// other unsaved.
fn two_file_project() -> tempfile::TempDir {
    let folder = tempfile::tempdir().expect("temporary folder");
    write(folder.path(), "a.txt", "one\n");
    write(folder.path(), "b.txt", "two\n");
    folder
}

/// A copy of `examples/box`, a real design, in a folder of its own.
fn box_project() -> tempfile::TempDir {
    let folder = tempfile::tempdir().expect("temporary folder");
    let example = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/box");
    for entry in std::fs::read_dir(example).expect("examples/box") {
        let entry = entry.expect("entry");
        std::fs::copy(entry.path(), folder.path().join(entry.file_name())).expect("copy");
    }
    folder
}

/// An agent's suggestion changing `relative` to `text`, applied to main as
/// an agent would, so main moves without the window.
fn agent_changes(folder: &Path, relative: &str, text: &str) {
    let mut history = History::open(folder).expect("open");
    let main = history.main().expect("main");
    let files = history.files_at(&main).expect("files");
    let changed = with(&files, relative, text);
    let id = history
        .suggest(&agent(), None, &changed, "a patch", "A change")
        .expect("suggest");
    history
        .apply(&id, &agent(), &main, &changed)
        .expect("apply");
}

fn unsaved(paths: &[&str]) -> BTreeSet<RelativePath> {
    paths.iter().map(|relative| path(relative)).collect()
}

#[test]
fn save_merges_into_main() {
    let folder = two_file_project();
    versions::turn_on(folder.path(), &user()).expect("turn on");
    write(folder.path(), "a.txt", "one, changed\n");

    let saved = versions::save(
        folder.path(),
        &user(),
        &[folder.path().join("a.txt")],
        &BTreeSet::new(),
    )
    .expect("save");

    let history = History::open(folder.path()).expect("open");
    let main = history.main().expect("main");
    assert_eq!(saved.revision.as_ref(), Some(&main));
    assert_eq!(
        text(&history.files_at(&main).expect("files"), "a.txt"),
        "one, changed\n"
    );
    let latest = history.versions(1).expect("versions");
    let latest = latest.first().expect("a version");
    assert_eq!(latest.actor.kind, ActorKind::User);
    assert_eq!(latest.message, "Saved a.txt");
    assert_eq!(saved.caught_up, CaughtUp::UpToDate);
}

#[test]
fn save_after_an_agent_applied_keeps_both_changes() {
    let folder = two_file_project();
    versions::turn_on(folder.path(), &user()).expect("turn on");
    agent_changes(folder.path(), "b.txt", "two, from the agent\n");
    write(folder.path(), "a.txt", "one, from the person\n");

    let saved = versions::save(
        folder.path(),
        &user(),
        &[folder.path().join("a.txt")],
        &BTreeSet::new(),
    )
    .expect("save");

    // The window caught up at once: the agent's change is in the folder.
    assert_eq!(read(folder.path(), "b.txt"), "two, from the agent\n");
    assert_eq!(read(folder.path(), "a.txt"), "one, from the person\n");
    assert!(matches!(saved.caught_up, CaughtUp::Wrote { .. }));
    let history = History::open(folder.path()).expect("open");
    let main = history
        .files_at(&history.main().expect("main"))
        .expect("files");
    assert_eq!(text(&main, "a.txt"), "one, from the person\n");
    assert_eq!(text(&main, "b.txt"), "two, from the agent\n");
}

#[test]
fn catch_up_reloads_clean_buffers_only() {
    // Main changed b; the window holds a unsaved. a's unsaved edits don't
    // stand in the way: b is written now, a is left alone.
    let main = ProjectFiles::new();
    let main = with(&main, "a.txt", "one\n");
    let main = with(&main, "b.txt", "two, from the agent\n");
    let folder_files = with(&main, "b.txt", "two\n");
    assert_eq!(
        versions::when_to_catch_up(&main, &folder_files, &unsaved(&["a.txt"])),
        CatchUpWhen::Now
    );
    // The window holds b unsaved: b waits for the next save, so the
    // person's unsaved edits are never overwritten.
    assert_eq!(
        versions::when_to_catch_up(&main, &folder_files, &unsaved(&["b.txt"])),
        CatchUpWhen::AtNextSave {
            waiting: vec![path("b.txt")]
        }
    );
    // A file main added that the window holds unsaved waits too.
    let added = with(&main, "c.txt", "three\n");
    assert_eq!(
        versions::when_to_catch_up(&added, &folder_files, &unsaved(&["c.txt"])),
        CatchUpWhen::AtNextSave {
            waiting: vec![path("c.txt")]
        }
    );

    // And on a real project: only b is written, a keeps what is on disk.
    let folder = two_file_project();
    versions::turn_on(folder.path(), &user()).expect("turn on");
    agent_changes(folder.path(), "b.txt", "two, from the agent\n");
    let caught_up = versions::catch_up(folder.path(), &unsaved(&["a.txt"])).expect("catch up");
    assert_eq!(
        caught_up,
        CaughtUp::Wrote {
            written: vec![folder.path().join("b.txt")],
            removed: Vec::new(),
            conflicted: Vec::new(),
        }
    );
    assert_eq!(read(folder.path(), "b.txt"), "two, from the agent\n");
    assert_eq!(read(folder.path(), "a.txt"), "one\n");
    assert_eq!(
        versions::catch_up(folder.path(), &unsaved(&["a.txt"])).expect("again"),
        CaughtUp::UpToDate
    );
}

#[test]
fn catch_up_waits_while_the_changed_file_is_unsaved() {
    let folder = two_file_project();
    versions::turn_on(folder.path(), &user()).expect("turn on");
    agent_changes(folder.path(), "b.txt", "two, from the agent\n");

    let caught_up = versions::catch_up(folder.path(), &unsaved(&["b.txt"])).expect("catch up");
    assert_eq!(
        caught_up,
        CaughtUp::Waiting {
            files: vec![path("b.txt")]
        }
    );
    assert_eq!(read(folder.path(), "b.txt"), "two\n", "nothing written");

    // The person saves their own b: both changes meet in main, and the
    // folder catches up with the merge (a clash, here, with markers).
    write(folder.path(), "b.txt", "two, from the person\n");
    let saved = versions::save(
        folder.path(),
        &user(),
        &[folder.path().join("b.txt")],
        &BTreeSet::new(),
    )
    .expect("save");
    let CaughtUp::Wrote { conflicted, .. } = saved.caught_up else {
        panic!(
            "the folder catches up after the save: {:?}",
            saved.caught_up
        );
    };
    assert_eq!(conflicted, vec![path("b.txt")]);
    let merged = read(folder.path(), "b.txt");
    assert!(merged.contains("two, from the agent"), "{merged}");
    assert!(merged.contains("two, from the person"), "{merged}");
}

#[test]
fn turn_on_versions_prompt_needed() {
    assert_eq!(
        versions::on_save(&VersionsStatus::Off, false),
        OnSave::AskToTurnOn
    );
    assert_eq!(
        versions::on_save(&VersionsStatus::Off, true),
        OnSave::Nothing
    );
    assert_eq!(
        versions::on_save(&VersionsStatus::On, false),
        OnSave::Record
    );
    assert_eq!(
        versions::on_save(
            &VersionsStatus::Nested {
                repository_root: PathBuf::from("/somewhere")
            },
            false
        ),
        OnSave::Nothing
    );
}

#[test]
fn not_now_is_remembered_per_project() {
    let config = tempfile::tempdir().expect("config folder");
    let declined = Declined::new(config.path().join("stave").join("versions-not-now"));
    let first = two_file_project();
    let second = two_file_project();
    assert!(!declined.contains(first.path()));
    declined.add(first.path()).expect("remember");
    declined.add(first.path()).expect("remember twice");
    assert!(declined.contains(first.path()));
    assert!(!declined.contains(second.path()));
    let text = std::fs::read_to_string(config.path().join("stave").join("versions-not-now"))
        .expect("written");
    assert_eq!(text.lines().count(), 1, "{text}");
}

#[test]
fn turning_on_records_the_project_as_it_is() {
    let folder = two_file_project();
    let first = versions::turn_on(folder.path(), &user()).expect("turn on");
    let history = History::open(folder.path()).expect("open");
    assert_eq!(history.main().expect("main"), first);
    let shown = versions::read(folder.path(), 10).expect("read");
    assert_eq!(shown.status, VersionsStatus::On);
    assert_eq!(shown.versions.len(), 1);
    assert!(shown.suggestions.is_empty());
}

#[test]
fn reading_an_off_project_lists_nothing() {
    let folder = two_file_project();
    let shown = versions::read(folder.path(), 10).expect("read");
    assert_eq!(shown.status, VersionsStatus::Off);
    assert!(shown.versions.is_empty());
    assert!(shown.suggestions.is_empty());
    assert!(!folder.path().join(".stave").exists(), "nothing written");
}

/// Records a suggestion holding `patch`, as an agent would: the files are
/// the project after the patch.
fn suggest_patch(folder: &Path, patch: &str, after: &[(&str, &str)]) -> versions::SuggestionId {
    let mut history = History::open(folder).expect("open");
    let main = history.main().expect("main");
    let mut files = history.files_at(&main).expect("files");
    for (relative, text) in after {
        files = with(&files, relative, text);
    }
    history
        .suggest(&agent(), None, &files, patch, "Make the box longer")
        .expect("suggest")
}

#[test]
fn applying_from_the_window_replays_the_patch_on_main() {
    let folder = box_project();
    versions::turn_on(folder.path(), &user()).expect("turn on");
    let original = read(folder.path(), "box.stave");
    let longer = original.replace("box_length = 300mm", "box_length = 320mm");
    let id = suggest_patch(
        folder.path(),
        "patch \"Longer\" {\n  set box_length = 320mm\n}\n",
        &[("box.stave", &longer)],
    );
    // The person changed another line meanwhile, and saved it.
    let deeper = original.replace("box_depth  = 200mm", "box_depth  = 210mm");
    write(folder.path(), "box.stave", &deeper);
    versions::save(
        folder.path(),
        &user(),
        &[folder.path().join("box.stave")],
        &BTreeSet::new(),
    )
    .expect("save");

    let applied = versions::apply(
        folder.path(),
        &versions::SuggestionId::new(id.as_str()),
        &user(),
    )
    .expect("apply");
    assert_eq!(applied.title, "Make the box longer");
    let history = History::open(folder.path()).expect("open");
    let main = history
        .files_at(&history.main().expect("main"))
        .expect("files");
    let applied_text = text(&main, "box.stave");
    assert!(
        applied_text.contains("box_length = 320mm"),
        "{applied_text}"
    );
    assert!(
        applied_text.contains("box_depth  = 210mm"),
        "{applied_text}"
    );
    assert!(history.suggestions().expect("suggestions").is_empty());
}

#[test]
fn applying_a_patch_that_breaks_the_design_is_refused() {
    let folder = box_project();
    versions::turn_on(folder.path(), &user()).expect("turn on");
    let id = suggest_patch(
        folder.path(),
        "patch \"Too short\" {\n  set box_height = 60mm\n}\n",
        &[],
    );
    let main_before = History::open(folder.path())
        .expect("open")
        .main()
        .expect("main");
    let refused = versions::apply(folder.path(), &id, &user());
    assert!(
        matches!(refused, Err(VersionsError::Refused { .. })),
        "{refused:?}"
    );
    let history = History::open(folder.path()).expect("open");
    assert_eq!(history.main().expect("main"), main_before, "main unmoved");
    assert_eq!(history.suggestions().expect("suggestions").len(), 1);
}

#[test]
fn window_undo_reverts_the_last_change_by_anyone() {
    let folder = two_file_project();
    versions::turn_on(folder.path(), &user()).expect("turn on");
    agent_changes(folder.path(), "b.txt", "two, from the agent\n");
    versions::catch_up(folder.path(), &BTreeSet::new()).expect("catch up");
    assert_eq!(read(folder.path(), "b.txt"), "two, from the agent\n");

    let undone = versions::undo(folder.path(), &user(), &BTreeSet::new()).expect("undo");
    assert!(undone.what.contains("A change"), "{}", undone.what);
    assert_eq!(read(folder.path(), "b.txt"), "two\n");
}

#[test]
fn discarding_removes_the_suggestion() {
    let folder = two_file_project();
    versions::turn_on(folder.path(), &user()).expect("turn on");
    let id = suggest_patch(folder.path(), "a patch", &[("a.txt", "one, suggested\n")]);
    versions::discard(folder.path(), &id, &user()).expect("discard");
    let shown = versions::read(folder.path(), 10).expect("read");
    assert!(shown.suggestions.is_empty());
    assert_eq!(read(folder.path(), "a.txt"), "one\n");
}
