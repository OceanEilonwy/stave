//! What revisions may not carry: files inside a repository's own folder
//! (`.git`, `.stave/history`) and agents' changes to the project's settings;
//! and applying from the base the caller read, whatever else the handle did.

mod common;

use std::path::{Path, PathBuf};

use common::{
    DRESSER, agent, dresser_folder, git, git_dresser_folder, git_output, path, text, user, with,
    write,
};
use stave_history::{CatchUp, History, HistoryError, PathError, RelativePath};

const WIDER: &str = "width = 1000mm\nheight = 1200mm\ndepth = 450mm\n";
const DEEPER: &str = "width = 900mm\nheight = 1200mm\ndepth = 500mm\n";

#[test]
fn repository_paths_are_not_project_paths() {
    // A suggestion's files are `ProjectFiles`, whose paths are
    // `RelativePath`s: none can name a file in a repository's own folder,
    // so no suggestion (or save, or apply) can hold one.
    for text in [
        ".git/hooks/pre-commit",
        ".GIT/config",
        "drawers/.git/config",
        ".stave/history/repo/store/x",
        ".stave/history/main.lock",
    ] {
        assert_eq!(
            RelativePath::new(Path::new(text)),
            Err(PathError::Reserved(PathBuf::from(text))),
            "{text}"
        );
    }
    assert!(RelativePath::new(Path::new(".stave/agents.toml")).is_ok());
}

#[test]
fn trees_made_outside_stave_never_hand_out_repository_paths() {
    let folder = git_dresser_folder();
    let window = History::start(folder.path()).expect("start");
    let before = window.read_folder().expect("read folder");

    // Someone builds a commit by hand (git's porcelain refuses `.git`
    // paths) holding a git hook and a file in Stave's repository, and puts
    // it on the folder's branch.
    let hook = git_output(
        folder.path(),
        &["hash-object", "-w", "--stdin"],
        "#!/bin/sh\necho owned\n",
    );
    let dresser = git_output(folder.path(), &["hash-object", "-w", "--stdin"], WIDER);
    let hooks = git_output(
        folder.path(),
        &["mktree"],
        &format!("100755 blob {hook}\tpre-commit\n"),
    );
    let git_folder = git_output(
        folder.path(),
        &["mktree"],
        &format!("040000 tree {hooks}\thooks\n"),
    );
    let history_folder = git_output(
        folder.path(),
        &["mktree"],
        &format!("100644 blob {hook}\tlock\n"),
    );
    let stave_folder = git_output(
        folder.path(),
        &["mktree"],
        &format!("040000 tree {history_folder}\thistory\n"),
    );
    let root = git_output(
        folder.path(),
        &["mktree"],
        &format!(
            "040000 tree {git_folder}\t.git\n040000 tree {stave_folder}\t.stave\n100644 blob {dresser}\tdresser.stave\n"
        ),
    );
    let commit = git_output(
        folder.path(),
        &["commit-tree", &root, "-p", "HEAD", "-m", "Sneaky"],
        "",
    );
    git(folder.path(), &["update-ref", "refs/heads/main", &commit]);

    // Opening takes the commit in as main.
    let mut window_again = History::open(folder.path()).expect("open");
    assert_eq!(window_again.main().expect("main").as_str(), commit);

    let files = window_again
        .files_at(&window_again.main().expect("main"))
        .expect("files at main");
    let paths: Vec<&RelativePath> = files.paths().collect();
    assert_eq!(paths, [&path("dresser.stave")]);

    let CatchUp::Write { files, removed, .. } = window_again.catch_up(&before).expect("catch up")
    else {
        panic!("main moved");
    };
    let paths: Vec<&RelativePath> = files.paths().collect();
    assert_eq!(paths, [&path("dresser.stave")]);
    assert!(removed.is_empty(), "{removed:?}");
    drop(window);
}

/// A project with its settings saved by the person.
fn project_with_settings() -> (tempfile::TempDir, History) {
    let folder = dresser_folder();
    write(
        folder.path(),
        ".stave/agents.toml",
        "merge_policy = \"ask\"\n",
    );
    let mut history = History::start(folder.path()).expect("start");
    let files = history.read_folder().expect("read folder");
    history.save(&files, &user(), "First cut").expect("save");
    (folder, history)
}

fn is_protected(error: &HistoryError) -> bool {
    matches!(error, HistoryError::SettingsProtected { path: protected } if *protected == path(".stave/agents.toml"))
}

#[test]
fn agents_cannot_change_the_settings() {
    let (_folder, mut history) = project_with_settings();
    let main = history.main().expect("main");
    let files = history.files_at(&main).expect("files at main");
    assert_eq!(
        text(&files, ".stave/agents.toml"),
        "merge_policy = \"ask\"\n"
    );

    // A suggestion that raises its own rights is refused.
    let raised = with(&files, ".stave/agents.toml", "merge_policy = \"auto\"\n");
    let error = history
        .suggest(&agent(), None, &raised, "", "More rights")
        .expect_err("refused");
    assert!(is_protected(&error), "{error:?}");
    // So is one that deletes the settings.
    let mut deleted = files.clone();
    deleted.remove(&path(".stave/agents.toml"));
    let error = history
        .suggest(&agent(), None, &deleted, "", "No settings")
        .expect_err("refused");
    assert!(is_protected(&error), "{error:?}");

    // A suggestion that leaves them alone is recorded; resolving it or
    // applying it can't change them either.
    let wider = with(&files, "dresser.stave", WIDER);
    let id = history
        .suggest(&agent(), None, &wider, "set width = 1000mm\n", "Widen")
        .expect("suggest");
    let raised_wider = with(&wider, ".stave/agents.toml", "merge_policy = \"auto\"\n");
    let error = history
        .write_suggestion_files(&id, &raised_wider)
        .expect_err("refused");
    assert!(is_protected(&error), "{error:?}");
    let error = history
        .apply(&id, &agent(), &main, &raised_wider)
        .expect_err("refused");
    assert!(is_protected(&error), "{error:?}");
    assert_eq!(history.main().expect("main"), main);

    history
        .apply(&id, &agent(), &main, &wider)
        .expect("apply leaves the settings alone");

    // The person changes them by saving.
    let mut window = history;
    let now = window.read_folder().expect("read folder");
    let catch_up = window.catch_up(&now).expect("catch up");
    assert!(matches!(catch_up, CatchUp::Write { .. }));
    let off = with(&wider, ".stave/agents.toml", "merge_policy = \"off\"\n");
    let saved = window.save(&off, &user(), "Agents off").expect("save");
    assert_eq!(
        text(
            &window.files_at(&saved).expect("files"),
            ".stave/agents.toml"
        ),
        "merge_policy = \"off\"\n"
    );
}

#[test]
fn apply_merges_from_the_base_given_even_after_main_was_read_again() {
    let folder = dresser_folder();
    let mut window = History::start(folder.path()).expect("start");
    let first = window.read_folder().expect("read folder");
    window.save(&first, &user(), "First cut").expect("save");

    // The agent reads main and prepares its change.
    let mut agent_history = History::open(folder.path()).expect("agent opens");
    let base = agent_history.main().expect("main");
    let files = agent_history.files_at(&base).expect("files");
    let wider = with(&files, "dresser.stave", WIDER);
    let id = agent_history
        .suggest(&agent(), None, &wider, "set width = 1000mm\n", "Widen")
        .expect("suggest");

    // Meanwhile the person saves, and another call on the agent's handle
    // reads main again.
    window
        .save(&with(&first, "dresser.stave", DEEPER), &user(), "Deeper")
        .expect("save");
    let newer = agent_history.main().expect("main");
    assert_ne!(newer, base);

    let applied = agent_history
        .apply(&id, &agent(), &base, &wider)
        .expect("apply");

    // The person's save is kept: the change from `base` was merged.
    assert_eq!(
        text(
            &agent_history.files_at(&applied).expect("files"),
            "dresser.stave"
        ),
        "width = 1000mm\nheight = 1200mm\ndepth = 500mm\n"
    );
    assert_ne!(DRESSER, WIDER);
}

#[test]
fn apply_with_an_unknown_base_is_refused() {
    let folder = dresser_folder();
    let mut history = History::start(folder.path()).expect("start");
    let files = history.read_folder().expect("read folder");
    history.save(&files, &user(), "First cut").expect("save");
    let wider = with(&files, "dresser.stave", WIDER);
    let id = history
        .suggest(&agent(), None, &wider, "set width = 1000mm\n", "Widen")
        .expect("suggest");
    let error = history
        .apply(
            &id,
            &agent(),
            &stave_history::RevisionId::new("0123"),
            &wider,
        )
        .expect_err("refused");
    assert!(
        matches!(error, HistoryError::UnknownRevision(_)),
        "{error:?}"
    );
}
