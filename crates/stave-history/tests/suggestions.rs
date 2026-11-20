//! Suggestions, applying them, the lock around main, undo, discard, paths
//! and the merge policy.

mod common;

use std::path::Path;
use std::sync::{Arc, Barrier};

use common::{DRESSER, agent, dresser_folder, other_agent, path, text, user, with, write};
use stave_history::{
    ActorKind, CatchUp, History, HistoryError, MergePolicy, PathError, RelativePath, UndoScope,
    merge_policy,
};

const WIDER: &str = "width = 1000mm\nheight = 1200mm\ndepth = 450mm\n";
const WIDER_TALLER: &str = "width = 1000mm\nheight = 1300mm\ndepth = 450mm\n";
const DEEPER: &str = "width = 900mm\nheight = 1200mm\ndepth = 500mm\n";

/// A started project with its first revision saved by the person.
fn saved_project() -> (tempfile::TempDir, History) {
    let folder = dresser_folder();
    let mut history = History::start(folder.path()).expect("start");
    let files = history.read_folder().expect("read folder");
    history.save(&files, &user(), "First cut").expect("save");
    (folder, history)
}

#[test]
fn suggestion_survives_reopen() {
    let (folder, history) = saved_project();
    let main = history.main().expect("main");
    drop(history);

    let id = {
        let mut agent_history = History::open(folder.path()).expect("open");
        let files = agent_history.files_at(&main).expect("files");
        let wider = with(&files, "dresser.stave", WIDER);
        let id = common::suggest(
            &mut agent_history,
            &agent(),
            None,
            &wider,
            "set width = 1000mm\n",
            "Widen the dresser",
        )
        .expect("suggest");
        let taller = with(&files, "dresser.stave", WIDER_TALLER);
        let onto = common::suggest(
            &mut agent_history,
            &agent(),
            Some(&id),
            &taller,
            "set height = 1300mm\n",
            "Taller too",
        )
        .expect("suggest onto");
        assert_eq!(onto, id);
        id
    };

    let reopened = History::open(folder.path()).expect("reopen");
    let suggestions = reopened.suggestions().expect("suggestions");
    assert_eq!(suggestions.len(), 1);
    let suggestion = &suggestions[0];
    assert_eq!(suggestion.id, id);
    assert_eq!(suggestion.agent, agent());
    assert_eq!(suggestion.title, "Widen the dresser");
    assert_eq!(
        suggestion.patches,
        ["set width = 1000mm\n", "set height = 1300mm\n"]
    );
    assert_eq!(suggestion.base, main);
    assert!(suggestion.conflicted.is_empty());
    let files = reopened.suggestion_files(&id).expect("suggestion files");
    assert_eq!(text(&files, "dresser.stave"), WIDER_TALLER);
    // Main is untouched by a suggestion.
    assert_eq!(reopened.main().expect("main"), main);
}

#[test]
fn apply_moves_main() {
    let (_folder, mut history) = saved_project();
    let main = history.main().expect("main");
    let files = history.files_at(&main).expect("files");
    let wider = with(&files, "dresser.stave", WIDER);
    let id = common::suggest(
        &mut history,
        &agent(),
        None,
        &wider,
        "set width = 1000mm\n",
        "Widen the dresser",
    )
    .expect("suggest");

    let applied = history.apply(&id, &agent(), &main, &wider).expect("apply");

    assert_eq!(history.main().expect("main"), applied);
    assert_eq!(
        text(&history.files_at(&applied).expect("files"), "dresser.stave"),
        WIDER
    );
    let versions = history.versions(10).expect("versions");
    assert_eq!(versions[0].id, applied);
    assert_eq!(versions[0].actor, agent());
    assert_eq!(versions[0].message, "Widen the dresser");
    assert!(history.suggestions().expect("suggestions").is_empty());
    let again = history
        .apply(&id, &agent(), &main, &wider)
        .expect_err("gone");
    assert!(
        matches!(again, HistoryError::UnknownSuggestion(_)),
        "{again:?}"
    );
}

#[test]
fn apply_refused_while_suggestion_has_clashes() {
    let (_folder, mut history) = saved_project();
    let main = history.main().expect("main");
    let files = history.files_at(&main).expect("files");
    let wider = with(&files, "dresser.stave", WIDER);
    let id = common::suggest(
        &mut history,
        &agent(),
        None,
        &wider,
        "set width = 1000mm\n",
        "Widen the dresser",
    )
    .expect("suggest");
    // The person changes the same line on main.
    let narrower = with(
        &files,
        "dresser.stave",
        "width = 800mm\nheight = 1200mm\ndepth = 450mm\n",
    );
    history.save(&narrower, &user(), "Narrower").expect("save");

    let conflicted = history.rebase_suggestion(&id, &agent()).expect("rebase");
    assert_eq!(conflicted, [path("dresser.stave")]);
    let suggestion = &history.suggestions().expect("suggestions")[0];
    assert_eq!(suggestion.conflicted, [path("dresser.stave")]);
    assert_eq!(suggestion.base, history.main().expect("main"));
    let clashing = text(
        &history.suggestion_files(&id).expect("files"),
        "dresser.stave",
    );
    assert!(clashing.contains("<<<<<<<"), "{clashing}");

    let refused = history
        .apply(&id, &agent(), &main, &wider)
        .expect_err("refused");
    assert!(
        matches!(refused, HistoryError::SuggestionConflicted { .. }),
        "{refused:?}"
    );

    // Resolving writes the suggestion's files; then it applies.
    let resolved = with(&files, "dresser.stave", WIDER);
    common::write_suggestion_files(&mut history, &id, &resolved).expect("resolve");
    assert!(
        history.suggestions().expect("suggestions")[0]
            .conflicted
            .is_empty()
    );
    let main = history.main().expect("main");
    history
        .apply(&id, &agent(), &main, &resolved)
        .expect("apply");
}

#[test]
fn concurrent_save_and_apply_both_land() {
    for round in 0..4 {
        let (folder, history) = saved_project();
        drop(history);
        let barrier = Arc::new(Barrier::new(2));

        let window_folder = folder.path().to_owned();
        let window_barrier = Arc::clone(&barrier);
        let window = std::thread::spawn(move || {
            let mut window = History::open(&window_folder).expect("window opens");
            let files = window.read_folder().expect("read folder");
            let deeper = with(&files, "dresser.stave", DEEPER);
            window_barrier.wait();
            window.save(&deeper, &user(), "Deeper").expect("save");
        });

        let agent_folder = folder.path().to_owned();
        let agent_barrier = Arc::clone(&barrier);
        let agent_thread = std::thread::spawn(move || {
            let mut history = History::open(&agent_folder).expect("agent opens");
            let main = history.main().expect("main");
            let files = history.files_at(&main).expect("files");
            let wider = with(&files, "dresser.stave", WIDER);
            let id = common::suggest(
                &mut history,
                &agent(),
                None,
                &wider,
                "set width = 1000mm\n",
                "Widen the dresser",
            )
            .expect("suggest");
            agent_barrier.wait();
            history.apply(&id, &agent(), &main, &wider).expect("apply");
        });

        window.join().expect("window thread");
        agent_thread.join().expect("agent thread");

        let history = History::open(folder.path()).expect("open");
        let main = history.main().expect("main");
        assert_eq!(
            text(&history.files_at(&main).expect("files"), "dresser.stave"),
            "width = 1000mm\nheight = 1200mm\ndepth = 500mm\n",
            "round {round}: both changes are on main"
        );
        let versions = history.versions(10).expect("versions");
        let messages: Vec<&str> = versions
            .iter()
            .map(|version| version.message.as_str())
            .collect();
        // Following first parents reaches every revision: main is one line.
        assert_eq!(messages.len(), 3, "round {round}: {messages:?}");
        assert!(messages.contains(&"Deeper"), "{messages:?}");
        assert!(messages.contains(&"Widen the dresser"), "{messages:?}");
        assert_eq!(messages[2], "First cut");
    }
}

#[test]
fn agent_undo_only_reverts_own() {
    let (folder, mut window) = saved_project();
    let first = window.main().expect("main");
    let mut history = History::open(folder.path()).expect("agent opens");
    let files = history.files_at(&first).expect("files");
    let wider = with(&files, "dresser.stave", WIDER);
    let id = common::suggest(
        &mut history,
        &agent(),
        None,
        &wider,
        "set width = 1000mm\n",
        "Widen the dresser",
    )
    .expect("suggest");
    history.apply(&id, &agent(), &first, &wider).expect("apply");

    // Another agent can't undo it: it isn't theirs.
    let mut other = History::open(folder.path()).expect("other opens");
    let error = other
        .undo(&other_agent(), UndoScope::Own)
        .expect_err("nothing of theirs");
    assert!(matches!(error, HistoryError::NothingToUndo), "{error:?}");

    // The person saves after the apply.
    let deeper = with(&files, "dresser.stave", DEEPER);
    window.save(&deeper, &user(), "Deeper").expect("save");

    let error = history
        .undo(&agent(), UndoScope::Own)
        .expect_err("a later save builds on the apply");
    assert!(
        matches!(error, HistoryError::UndoBlocked { .. }),
        "{error:?}"
    );
    let message = error.to_string();
    assert!(message.contains("Henry"), "names who: {message}");
    assert!(message.contains("Deeper"), "names the save: {message}");
}

#[test]
fn undo_reverts_own_apply_and_window_catches_up() {
    let (folder, mut window) = saved_project();
    let first = window.main().expect("main");
    let mut history = History::open(folder.path()).expect("agent opens");
    let files = history.files_at(&first).expect("files");
    let wider = with(&files, "dresser.stave", WIDER);
    let id = common::suggest(
        &mut history,
        &agent(),
        None,
        &wider,
        "set width = 1000mm\n",
        "Widen the dresser",
    )
    .expect("suggest");
    history.apply(&id, &agent(), &first, &wider).expect("apply");
    // The window caught up with the apply and wrote it.
    let folder_files = window.read_folder().expect("read folder");
    let CatchUp::Write { files: written, .. } =
        common::catch_up(&mut window, &folder_files).expect("catch up")
    else {
        panic!("the apply is new to the window");
    };
    write(
        folder.path(),
        "dresser.stave",
        &text(&written, "dresser.stave"),
    );

    let undone = history.undo(&agent(), UndoScope::Own).expect("undo");
    assert!(undone.contains("Widen the dresser"), "{undone}");
    assert_eq!(history.main().expect("main"), first);

    // The window catches up with the undo: the width goes back.
    let folder_files = window.read_folder().expect("read folder");
    let CatchUp::Write { files: written, .. } =
        common::catch_up(&mut window, &folder_files).expect("catch up")
    else {
        panic!("the undo is new to the window");
    };
    assert_eq!(text(&written, "dresser.stave"), DRESSER);

    // The person's Undo reaches anyone's change, newest first: the agent's
    // suggestion, then their own first save.
    let undone = window.undo(&user(), UndoScope::Any).expect("undo any");
    assert!(undone.starts_with("suggest"), "{undone}");
    let undone = window.undo(&user(), UndoScope::Any).expect("undo any");
    assert!(undone.contains("First cut"), "{undone}");
    assert_eq!(
        window
            .files_at(&window.main().expect("main"))
            .expect("files"),
        stave_history::ProjectFiles::new()
    );
    let error = window
        .undo(&user(), UndoScope::Any)
        .expect_err("nothing left");
    assert!(matches!(error, HistoryError::NothingToUndo), "{error:?}");
}

#[test]
fn discard_removes_suggestion() {
    let (_folder, mut history) = saved_project();
    let main = history.main().expect("main");
    let files = history.files_at(&main).expect("files");
    let wider = with(&files, "dresser.stave", WIDER);
    let id = common::suggest(
        &mut history,
        &agent(),
        None,
        &wider,
        "set width = 1000mm\n",
        "Widen the dresser",
    )
    .expect("suggest");

    history.discard(&id, &user()).expect("discard");

    assert!(history.suggestions().expect("suggestions").is_empty());
    assert_eq!(history.main().expect("main"), main);
    let error = history.suggestion_files(&id).expect_err("gone");
    assert!(
        matches!(error, HistoryError::UnknownSuggestion(_)),
        "{error:?}"
    );
    // Discarding is a change of its own and can be undone.
    history
        .undo(&user(), UndoScope::Own)
        .expect("undo the discard");
    let back = history.suggestions().expect("suggestions");
    assert_eq!(back.len(), 1);
    assert_eq!(back[0].agent.kind, ActorKind::Agent);
}

#[test]
fn relative_path_rejects_parent_and_absolute() {
    assert!(matches!(
        RelativePath::new(Path::new("../outside.stave")),
        Err(PathError::Parent(_))
    ));
    assert!(matches!(
        RelativePath::new(Path::new("drawers/../../outside.stave")),
        Err(PathError::Parent(_))
    ));
    assert!(matches!(
        RelativePath::new(Path::new("/etc/passwd")),
        Err(PathError::Absolute(_))
    ));
    assert_eq!(
        RelativePath::new(Path::new("drawers/drawer.stave"))
            .expect("inside")
            .as_path(),
        Path::new("drawers/drawer.stave")
    );
}

#[test]
fn read_folder_skips_symbolic_links() {
    let outside = tempfile::tempdir().expect("temporary folder");
    write(outside.path(), "secret.txt", "not the project's");
    let folder = dresser_folder();
    std::os::unix::fs::symlink(
        outside.path().join("secret.txt"),
        folder.path().join("link.stave"),
    )
    .expect("file link");
    std::os::unix::fs::symlink(outside.path(), folder.path().join("linked")).expect("folder link");

    let history = History::start(folder.path()).expect("start");
    let files = history.read_folder().expect("read folder");

    let paths: Vec<&RelativePath> = files.paths().collect();
    assert_eq!(paths, [&path("dresser.stave")]);
}

#[test]
fn merge_policy_reads_project_then_global_then_ask() {
    let folder = dresser_folder();
    let config = tempfile::tempdir().expect("config folder");
    assert_eq!(merge_policy(folder.path(), config.path()), MergePolicy::Ask);

    write(config.path(), "agents.toml", "merge_policy = \"auto\"\n");
    assert_eq!(
        merge_policy(folder.path(), config.path()),
        MergePolicy::Auto
    );

    write(
        folder.path(),
        ".stave/agents.toml",
        "merge_policy = \"off\"\n",
    );
    assert_eq!(merge_policy(folder.path(), config.path()), MergePolicy::Off);

    write(
        folder.path(),
        ".stave/agents.toml",
        "merge_policy = \"ask\"\n",
    );
    assert_eq!(merge_policy(folder.path(), config.path()), MergePolicy::Ask);

    // A project file without a readable setting falls back to the global.
    write(
        folder.path(),
        ".stave/agents.toml",
        "merge_policy = \"sometimes\"\n",
    );
    assert_eq!(
        merge_policy(folder.path(), config.path()),
        MergePolicy::Auto
    );
}
