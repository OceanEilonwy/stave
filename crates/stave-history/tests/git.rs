//! A folder that is the root of a git repository: main is the folder's
//! branch, in step both ways, and what Stave doesn't read (symbolic links,
//! executable bits, tracked files `.gitignore` excludes) survives a save.

mod common;

use std::os::unix::fs::PermissionsExt as _;

use common::{agent, git, git_dresser_folder, git_output, text, user, with, write};
use stave_history::History;

const WIDER: &str = "width = 1000mm\nheight = 1200mm\ndepth = 450mm\n";
const DEEPER: &str = "width = 900mm\nheight = 1200mm\ndepth = 500mm\n";

fn rev_parse(folder: &std::path::Path, name: &str) -> String {
    git_output(folder, &["rev-parse", name], "")
}

#[test]
fn saves_and_applies_move_the_folder_branch() {
    let folder = git_dresser_folder();
    let mut history = History::start(folder.path()).expect("start");
    assert_eq!(
        history.main().expect("main").as_str(),
        rev_parse(folder.path(), "main")
    );

    write(folder.path(), "dresser.stave", DEEPER);
    let files = history.read_folder().expect("read folder");
    let saved = history.save(&files, &user(), "Deeper").expect("save");

    assert_eq!(saved.as_str(), rev_parse(folder.path(), "main"));
    assert_eq!(
        git_output(folder.path(), &["symbolic-ref", "HEAD"], ""),
        "refs/heads/main"
    );
    // git's index follows main, so git sees the folder as it is: clean.
    assert_eq!(
        git_output(folder.path(), &["status", "--porcelain"], ""),
        ""
    );
    assert_eq!(
        git_output(folder.path(), &["log", "-1", "--format=%s"], ""),
        "Deeper"
    );

    // An agent's apply moves it too.
    let base = history.main().expect("main");
    let changed = with(&files, "dresser.stave", WIDER);
    let id = common::suggest(
        &mut history,
        &agent(),
        None,
        &changed,
        "set width = 1000mm\n",
        "Widen",
    )
    .expect("suggest");
    let applied = history
        .apply(&id, &agent(), &base, &changed)
        .expect("apply");
    assert_eq!(applied.as_str(), rev_parse(folder.path(), "main"));
}

#[test]
fn git_commits_become_main() {
    let folder = git_dresser_folder();
    let history = History::start(folder.path()).expect("start");
    drop(history);

    write(folder.path(), "dresser.stave", WIDER);
    git(folder.path(), &["commit", "-q", "-am", "Wider, from git"]);
    let commit = rev_parse(folder.path(), "main");

    let mut history = History::open(folder.path()).expect("open");
    assert_eq!(history.main().expect("main").as_str(), commit);
    let versions = history.versions(10).expect("versions");
    assert_eq!(versions[0].message, "Wider, from git");

    // A save after that builds on the git commit and moves the branch on.
    write(folder.path(), "dresser.stave", DEEPER);
    git(folder.path(), &["commit", "-q", "-am", "Deeper, from git"]);
    write(folder.path(), "notes.txt", "glue first\n");
    let files = history.read_folder().expect("read folder");
    let saved = history.save(&files, &user(), "Notes").expect("save");
    assert_eq!(saved.as_str(), rev_parse(folder.path(), "main"));
    let versions = history.versions(10).expect("versions");
    let messages: Vec<&str> = versions
        .iter()
        .map(|version| version.message.as_str())
        .collect();
    assert_eq!(
        messages,
        ["Notes", "Deeper, from git", "Wider, from git", "First cut"]
    );
    assert_eq!(
        text(&history.files_at(&saved).expect("files"), "dresser.stave"),
        DEEPER
    );
}

#[test]
fn save_keeps_what_stave_does_not_read() {
    let folder = git_dresser_folder();
    write(folder.path(), "build.sh", "#!/bin/sh\necho build\n");
    let script = folder.path().join("build.sh");
    let mut permissions = std::fs::metadata(&script).expect("metadata").permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&script, permissions).expect("make executable");
    std::os::unix::fs::symlink("dresser.stave", folder.path().join("current.stave"))
        .expect("symbolic link");
    write(folder.path(), ".gitignore", "*.local\n");
    write(folder.path(), "shop.local", "saw = table\n");
    git(
        folder.path(),
        &["add", "build.sh", "current.stave", ".gitignore"],
    );
    git(folder.path(), &["add", "-f", "shop.local"]);
    git(folder.path(), &["commit", "-q", "-m", "Tools"]);

    let mut history = History::start(folder.path()).expect("start");
    write(folder.path(), "dresser.stave", DEEPER);
    let files = history.read_folder().expect("read folder");
    let saved = history.save(&files, &user(), "Deeper").expect("save");

    let listing = git_output(folder.path(), &["ls-tree", saved.as_str()], "");
    let mode_of = |name: &str| {
        listing
            .lines()
            .find(|line| line.ends_with(&format!("\t{name}")))
            .map(|line| line.split(' ').next().unwrap_or_default().to_owned())
    };
    assert_eq!(mode_of("build.sh").as_deref(), Some("100755"), "{listing}");
    assert_eq!(
        mode_of("current.stave").as_deref(),
        Some("120000"),
        "{listing}"
    );
    assert_eq!(
        mode_of("shop.local").as_deref(),
        Some("100644"),
        "{listing}"
    );
    assert_eq!(
        git_output(folder.path(), &["status", "--porcelain"], ""),
        ""
    );

    // Deleting a file the folder shows still deletes it.
    std::fs::remove_file(&script).expect("delete");
    let files = history.read_folder().expect("read folder");
    let saved = history.save(&files, &user(), "No script").expect("save");
    let listing = git_output(folder.path(), &["ls-tree", saved.as_str()], "");
    assert!(!listing.contains("build.sh"), "{listing}");
}
