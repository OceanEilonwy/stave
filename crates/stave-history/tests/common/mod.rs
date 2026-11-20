//! Helpers shared by the integration tests: project folders in temporary
//! directories, actors, and a git command line for the tests that need a
//! git repository around a project.

#![allow(
    dead_code,
    reason = "each test file uses a different part of the helpers"
)]

use std::path::{Path, PathBuf};
use std::process::Command;

use stave_history::{Actor, ActorKind, ProjectFiles, RelativePath};

/// The person using the window.
pub fn user() -> Actor {
    Actor {
        kind: ActorKind::User,
        name: "Henry".to_owned(),
        id: "window".to_owned(),
    }
}

/// An agent connected through an AI app.
pub fn agent() -> Actor {
    Actor {
        kind: ActorKind::Agent,
        name: "Claude Desktop".to_owned(),
        id: "3f2a".to_owned(),
    }
}

/// A second agent, from another AI app.
pub fn other_agent() -> Actor {
    Actor {
        kind: ActorKind::Agent,
        name: "Codex".to_owned(),
        id: "9c01".to_owned(),
    }
}

/// A validated relative path.
pub fn path(text: &str) -> RelativePath {
    RelativePath::new(Path::new(text)).expect("a valid relative path")
}

/// Writes `text` to `relative` inside `folder`, making directories.
pub fn write(folder: &Path, relative: &str, text: &str) {
    let full = folder.join(relative);
    if let Some(parent) = full.parent() {
        std::fs::create_dir_all(parent).expect("make directories");
    }
    std::fs::write(full, text).expect("write file");
}

/// The text of one file in a set of project files.
pub fn text(files: &ProjectFiles, relative: &str) -> String {
    let bytes = files.get(&path(relative)).expect("file present");
    String::from_utf8(bytes.to_vec()).expect("utf-8")
}

/// `files` with `relative` replaced by `text`.
pub fn with(files: &ProjectFiles, relative: &str, text: &str) -> ProjectFiles {
    let mut changed = files.clone();
    changed.insert(path(relative), text.as_bytes().to_vec());
    changed
}

/// A drawer project of three lines, so separate lines can change apart.
pub const DRESSER: &str = "width = 900mm\nheight = 1200mm\ndepth = 450mm\n";

/// A new temporary folder holding `dresser.stave`.
pub fn dresser_folder() -> tempfile::TempDir {
    let folder = tempfile::tempdir().expect("temporary folder");
    write(folder.path(), "dresser.stave", DRESSER);
    folder
}

/// Runs git in `folder` with a fixed identity, failing the test on error.
pub fn git(folder: &Path, arguments: &[&str]) {
    let status = Command::new("git")
        .current_dir(folder)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .args([
            "-c",
            "user.name=Tester",
            "-c",
            "user.email=tester@example.com",
        ])
        .args(arguments)
        .status()
        .expect("run git");
    assert!(status.success(), "git {arguments:?} failed");
}

/// Runs git in `folder` with `input` on its standard input, failing the test
/// on error, and returns what it printed, trimmed.
pub fn git_output(folder: &Path, arguments: &[&str], input: &str) -> String {
    use std::io::Write as _;
    let mut child = Command::new("git")
        .current_dir(folder)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .args([
            "-c",
            "user.name=Tester",
            "-c",
            "user.email=tester@example.com",
        ])
        .args(arguments)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("run git");
    child
        .stdin
        .take()
        .expect("standard input")
        .write_all(input.as_bytes())
        .expect("write to git");
    let output = child.wait_with_output().expect("git finishes");
    assert!(output.status.success(), "git {arguments:?} failed");
    String::from_utf8(output.stdout)
        .expect("utf-8")
        .trim()
        .to_owned()
}

/// A folder that is the root of a git repository on branch `main`, with
/// `dresser.stave` committed.
pub fn git_dresser_folder() -> tempfile::TempDir {
    let folder = dresser_folder();
    git(folder.path(), &["init", "-q", "-b", "main"]);
    git(folder.path(), &["add", "dresser.stave"]);
    git(folder.path(), &["commit", "-q", "-m", "First cut"]);
    folder
}

/// The canonical form of a path, as the library reports paths.
pub fn canonical(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).expect("canonical path")
}
