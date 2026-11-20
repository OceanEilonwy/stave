//! `docs/ai.md` shows only commands the real `stave` accepts: every line of
//! a fenced block that starts with `stave ` is run as the real binary with
//! `--help` added, so its subcommands and flags are parsed (an unknown one
//! fails) and nothing is done. It runs in a temporary home with an empty
//! `PATH`, so no AI app's settings are ever touched.

use std::path::Path;
use std::process::Command;

/// The `stave …` command lines in the fenced blocks of `text`, each split
/// into words after `stave`, with a trailing `# comment` dropped.
fn commands(text: &str) -> Vec<Vec<String>> {
    let mut found = Vec::new();
    let mut fenced = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            fenced = !fenced;
            continue;
        }
        if !fenced {
            continue;
        }
        let command = trimmed.split(" #").next().unwrap_or_default().trim();
        if let Some(rest) = command.strip_prefix("stave ") {
            found.push(rest.split_whitespace().map(str::to_owned).collect());
        } else if command == "stave" {
            found.push(Vec::new());
        }
    }
    found
}

#[test]
fn every_stave_command_in_the_ai_guide_parses() {
    let guide = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/ai.md");
    let text = std::fs::read_to_string(&guide).unwrap();
    let commands = commands(&text);
    for needed in [
        &["mcp", "install"][..],
        &["mcp", "uninstall"][..],
        &["mcp"][..],
    ] {
        assert!(
            commands.iter().any(|words| words == needed),
            "docs/ai.md never shows `stave {}`",
            needed.join(" ")
        );
    }
    let home = tempfile::tempdir().unwrap();
    for words in &commands {
        let output = Command::new(env!("CARGO_BIN_EXE_stave"))
            .args(words)
            .arg("--help")
            .env("HOME", home.path())
            .env("PATH", home.path().join("bin"))
            .env_remove("XDG_CONFIG_HOME")
            .env_remove("CODEX_HOME")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "`stave {}` in docs/ai.md is not a command stave accepts:\n{}",
            words.join(" "),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn a_flag_stave_doesnt_have_is_caught() {
    let commands = commands("```\nstave mcp install --chatgpt   # no such app\n```\n");
    assert_eq!(commands, vec![vec!["mcp", "install", "--chatgpt"]]);
    let output = Command::new(env!("CARGO_BIN_EXE_stave"))
        .args(&commands[0])
        .arg("--help")
        .output()
        .unwrap();
    assert!(!output.status.success());
}
