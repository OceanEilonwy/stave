//! `stave mcp install` and `stave mcp uninstall`, run as the real binary
//! in a temporary home with an empty `PATH`: the real `~/.config/Claude`,
//! `~/.codex` and `claude` are never touched.

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

fn stave(home: &Path, arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_stave"))
        .args(arguments)
        .env("HOME", home)
        .env("PATH", home.join("bin"))
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("CODEX_HOME")
        .output()
        .unwrap()
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

#[test]
fn install_codex_writes_this_stave_and_says_so() {
    let home = tempfile::tempdir().unwrap();
    let output = stave(home.path(), &["mcp", "install", "--codex"]);
    assert!(output.status.success(), "{}", text(&output.stderr));
    let said = text(&output.stdout);
    assert!(said.contains("Codex: added Stave"), "{said}");
    let config = fs::read_to_string(home.path().join(".codex/config.toml")).unwrap();
    let program = Path::new(env!("CARGO_BIN_EXE_stave"))
        .canonicalize()
        .unwrap();
    assert!(
        config.contains(&format!("command = \"{}\"", program.display())),
        "{config}"
    );
    assert!(config.contains("args = [\"mcp\"]"), "{config}");

    let output = stave(home.path(), &["mcp", "uninstall", "--codex"]);
    assert!(output.status.success(), "{}", text(&output.stderr));
    assert!(text(&output.stdout).contains("Codex: removed Stave"));
    let config = fs::read_to_string(home.path().join(".codex/config.toml")).unwrap();
    assert!(!config.contains("stave"), "{config}");
}

#[test]
fn install_with_no_flag_sets_up_every_app_found() {
    let home = tempfile::tempdir().unwrap();
    fs::create_dir_all(home.path().join(".config/Claude")).unwrap();
    let output = stave(home.path(), &["mcp", "install"]);
    assert!(output.status.success(), "{}", text(&output.stderr));
    let said = text(&output.stdout);
    assert!(said.contains("Claude Desktop: added Stave"), "{said}");
    assert!(!said.contains("Codex"), "{said}");
    assert!(
        home.path()
            .join(".config/Claude/claude_desktop_config.json")
            .is_file()
    );
}

#[test]
fn install_with_no_app_found_says_which_apps_it_knows() {
    let home = tempfile::tempdir().unwrap();
    let output = stave(home.path(), &["mcp", "install"]);
    assert_eq!(output.status.code(), Some(1));
    let said = text(&output.stderr);
    assert!(
        said.contains("Claude Desktop, Claude Code or Codex"),
        "{said}"
    );
}

#[test]
fn an_app_that_fails_fails_the_command_but_not_the_others() {
    let home = tempfile::tempdir().unwrap();
    // No `claude` on the path: Claude Code fails, Codex is still set up.
    let output = stave(home.path(), &["mcp", "install", "--claude-code", "--codex"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(text(&output.stderr).contains("Claude Code:"));
    assert!(text(&output.stdout).contains("Codex: added Stave"));
    assert!(home.path().join(".codex/config.toml").is_file());
}

#[test]
fn mcp_help_lists_install_and_uninstall() {
    let home = tempfile::tempdir().unwrap();
    let output = stave(home.path(), &["mcp", "--help"]);
    assert!(output.status.success(), "{}", text(&output.stderr));
    let said = text(&output.stdout);
    assert!(said.contains("install"), "{said}");
    assert!(said.contains("uninstall"), "{said}");
}

/// Bare `stave mcp` is what every app is set up to start: it parses (no
/// usage error) and runs the server, which here finds standard input
/// closed, with no AI app on the other end, and says so.
#[test]
fn bare_mcp_parses() {
    let home = tempfile::tempdir().unwrap();
    let output = stave(home.path(), &["mcp"]);
    let said = text(&output.stderr);
    assert!(!said.contains("Usage:"), "{said}");
    assert!(said.contains("talking to the AI app"), "{said}");
}
