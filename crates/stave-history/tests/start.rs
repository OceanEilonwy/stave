//! Turning versions on: where the repository goes, a git repository at the
//! folder's root, a folder inside a bigger repository, and which files count.

mod common;

use common::{canonical, dresser_folder, git, path, write};
use stave_history::{History, HistoryError, VersionsStatus};

#[test]
fn start_makes_only_stave_history() {
    let folder = dresser_folder();
    assert_eq!(
        History::status(folder.path()).expect("status"),
        VersionsStatus::Off
    );

    History::start(folder.path()).expect("start");

    assert!(folder.path().join(".stave/history").is_dir());
    assert!(!folder.path().join(".jj").exists());
    assert!(!folder.path().join(".git").exists());
    assert_eq!(
        History::status(folder.path()).expect("status"),
        VersionsStatus::On
    );
    // Opening again finds the same repository.
    History::open(folder.path()).expect("open");
}

#[test]
fn start_in_git_root_keeps_git_history() {
    let folder = dresser_folder();
    git(folder.path(), &["init", "-q", "-b", "main"]);
    git(folder.path(), &["add", "dresser.stave"]);
    git(
        folder.path(),
        &["commit", "-q", "-m", "First cut of the dresser"],
    );

    let history = History::start(folder.path()).expect("start");

    assert!(folder.path().join(".git").is_dir());
    assert!(!folder.path().join(".jj").exists());
    let versions = history.versions(10).expect("versions");
    assert!(
        versions
            .iter()
            .any(|version| version.message == "First cut of the dresser"),
        "the git commit is a version: {versions:?}"
    );
    // Main starts where the folder's branch is, so its files are there.
    let main = history.main().expect("main");
    let files = history.files_at(&main).expect("files at main");
    assert_eq!(common::text(&files, "dresser.stave"), common::DRESSER);
}

#[test]
fn nested_folder_is_refused() {
    let outer = tempfile::tempdir().expect("temporary folder");
    git(outer.path(), &["init", "-q", "-b", "main"]);
    let project = outer.path().join("dresser");
    write(&project, "dresser.stave", common::DRESSER);

    let status = History::status(&project).expect("status");
    assert_eq!(
        status,
        VersionsStatus::Nested {
            repository_root: canonical(outer.path())
        }
    );
    let error = History::start(&project).expect_err("nested is refused");
    assert!(matches!(error, HistoryError::Nested { .. }), "{error:?}");
    assert!(
        error.to_string().contains("copy"),
        "the message offers a copy: {error}"
    );
    assert!(!project.join(".stave/history").exists());
}

#[test]
fn read_folder_skips_history_and_gitignored() {
    let folder = dresser_folder();
    write(folder.path(), ".gitignore", "build/\n*.tmp\n");
    write(folder.path(), "build/plan.pdf", "not a project file");
    write(folder.path(), "notes.tmp", "scratch");
    write(folder.path(), "drawers/drawer.stave", "width = 400mm\n");
    write(
        folder.path(),
        ".stave/agents.toml",
        "merge_policy = \"ask\"\n",
    );

    let history = History::start(folder.path()).expect("start");
    let files = history.read_folder().expect("read folder");

    let paths: Vec<String> = files
        .paths()
        .map(|relative| relative.as_path().display().to_string())
        .collect();
    assert_eq!(
        paths,
        [
            ".gitignore",
            ".stave/agents.toml",
            "drawers/drawer.stave",
            "dresser.stave"
        ]
    );
    assert_eq!(
        files.get(&path("drawers/drawer.stave")),
        Some(&b"width = 400mm\n"[..])
    );
}
