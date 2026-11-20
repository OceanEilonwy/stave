//! `docs/ai.md` shows only commands the real `stave` accepts: every line of
//! a fenced block that starts with `stave ` is run as the real binary with
//! `--help` added, so its subcommands and flags are parsed (an unknown one
//! fails) and nothing is done. It runs in a temporary home with an empty
//! `PATH`, so no AI app's settings are ever touched. Its "The tools" table
//! names exactly the tools `stave-mcp` serves.

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

/// The tool names in the first column of the guide's "The tools" table,
/// sorted, each without its arguments.
fn tools_in_table(text: &str) -> Vec<String> {
    let text = format!("\n{text}");
    let section = text
        .split("\n## The tools\n")
        .nth(1)
        .expect("docs/ai.md has a section \"The tools\"");
    let section = section.split("\n## ").next().unwrap_or_default();
    let mut names = Vec::new();
    for row in section.lines().filter(|line| line.starts_with("| `")) {
        let cell = row.split('|').nth(1).unwrap_or_default();
        for (index, quoted) in cell.split('`').enumerate() {
            if index % 2 == 1 {
                let name = quoted.split('(').next().unwrap_or_default();
                names.push(name.to_owned());
            }
        }
    }
    names.sort();
    names
}

/// The tools `stave-mcp` serves: each `async fn` right after a `#[tool(`
/// attribute in its `tools` modules, sorted.
fn tools_in_server() -> Vec<String> {
    let tools = Path::new(env!("CARGO_MANIFEST_DIR")).join("../stave-mcp/src/tools");
    let mut names = Vec::new();
    for entry in std::fs::read_dir(&tools).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|extension| extension != "rs") {
            continue;
        }
        let source = std::fs::read_to_string(&path).unwrap();
        let mut in_tool = false;
        for line in source.lines() {
            let line = line.trim();
            if line.starts_with("#[tool(") || line == "#[tool]" {
                in_tool = true;
            } else if in_tool && let Some(rest) = line.strip_prefix("async fn ") {
                let name = rest.split(['(', '<']).next().unwrap_or_default();
                names.push(name.to_owned());
                in_tool = false;
            }
        }
    }
    names.sort();
    names
}

#[test]
fn the_ai_guides_tool_table_lists_exactly_the_servers_tools() {
    let guide = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/ai.md");
    let text = std::fs::read_to_string(&guide).unwrap();
    let served = tools_in_server();
    assert!(served.len() >= 18, "found only {served:?} in stave-mcp");
    assert_eq!(
        tools_in_table(&text),
        served,
        "docs/ai.md's \"The tools\" table and stave-mcp's tools differ"
    );
}

#[test]
fn a_tool_the_server_doesnt_have_is_caught() {
    let text = "## The tools\n\n| Tool | What |\n|---|---|\n\
                | `check` | ok |\n| `list_suggestions`, `cut_list(project?)` | no |\n\n## Next\n";
    assert_eq!(
        tools_in_table(text),
        ["check", "cut_list", "list_suggestions"]
    );
    assert!(!tools_in_server().contains(&"cut_list".to_owned()));
}
