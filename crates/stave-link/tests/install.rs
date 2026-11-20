//! Installing and uninstalling Stave in the AI apps, each in a temporary
//! home: the real `~/.config/Claude`, `~/.codex` and `claude` are never
//! touched. Claude Code is played by a small `claude` script on a
//! temporary `PATH` that keeps its servers in a file and logs its calls.

use std::fs;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};

use stave_link::{Host, Places, found_hosts_in, install_in, uninstall_in};

/// A temporary home with no AI app in it, and an empty `PATH`.
struct Home {
    directory: tempfile::TempDir,
    bin: PathBuf,
}

impl Home {
    fn new() -> Home {
        let directory = tempfile::tempdir().unwrap();
        let bin = directory.path().join("bin");
        fs::create_dir(&bin).unwrap();
        Home { directory, bin }
    }

    fn path(&self) -> &Path {
        self.directory.path()
    }

    fn places(&self) -> Places {
        Places::new(self.path().to_path_buf(), vec![self.bin.clone()])
    }

    fn claude_desktop_config(&self) -> PathBuf {
        self.path()
            .join(".config/Claude/claude_desktop_config.json")
    }

    fn codex_config(&self) -> PathBuf {
        self.path().join(".codex/config.toml")
    }

    /// Puts a fake `claude` on the `PATH`: `mcp get|add|remove` over a
    /// file of server names, every call logged.
    fn fake_claude(&self) {
        let script = r#"#!/bin/sh
here="$(dirname "$0")"
echo "$@" >> "$here/claude.log"
servers="$here/servers"
touch "$servers"
case "$1 $2" in
  "mcp get") grep -qx "$3" "$servers" ;;
  "mcp add")
    name="$5"
    if grep -qx "$name" "$servers"; then echo "already exists" >&2; exit 1; fi
    echo "$name" >> "$servers" ;;
  "mcp remove")
    name="$5"
    grep -qx "$name" "$servers" || { echo "not found" >&2; exit 1; }
    grep -vx "$name" "$servers" > "$servers.new"; mv "$servers.new" "$servers" ;;
  *) exit 2 ;;
esac
"#;
        let path = self.bin.join("claude");
        fs::write(&path, script).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    }

    fn claude_log(&self) -> String {
        fs::read_to_string(self.bin.join("claude.log")).unwrap_or_default()
    }

    fn claude_servers(&self) -> String {
        fs::read_to_string(self.bin.join("servers")).unwrap_or_default()
    }
}

const STAVE: &str = "/opt/stave/bin/stave";

fn json(path: &Path) -> serde_json::Value {
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

#[test]
fn installs_claude_desktop_keeping_other_servers() {
    let home = Home::new();
    let config = home.claude_desktop_config();
    fs::create_dir_all(config.parent().unwrap()).unwrap();
    fs::write(
        &config,
        r#"{
  "globalShortcut": "Ctrl+Space",
  "mcpServers": {
    "files": { "command": "npx", "args": ["files-server", "/home"] }
  }
}
"#,
    )
    .unwrap();
    let results = install_in(&home.places(), &[Host::ClaudeDesktop], Path::new(STAVE));
    assert_eq!(results.len(), 1);
    let installed = results.into_iter().next().unwrap().unwrap();
    assert_eq!(installed.host, Host::ClaudeDesktop);
    assert!(
        installed.how.contains("Claude Desktop"),
        "{}",
        installed.how
    );
    let written = json(&config);
    assert_eq!(written["globalShortcut"], "Ctrl+Space");
    assert_eq!(written["mcpServers"]["files"]["command"], "npx");
    assert_eq!(
        written["mcpServers"]["files"]["args"],
        serde_json::json!(["files-server", "/home"])
    );
    assert_eq!(written["mcpServers"]["stave"]["command"], STAVE);
    assert_eq!(
        written["mcpServers"]["stave"]["args"],
        serde_json::json!(["mcp"])
    );
}

#[test]
fn installs_claude_desktop_without_a_config_file() {
    let home = Home::new();
    fs::create_dir_all(home.path().join(".config/Claude")).unwrap();
    let results = install_in(&home.places(), &[Host::ClaudeDesktop], Path::new(STAVE));
    assert!(results[0].is_ok(), "{:?}", results[0]);
    let written = json(&home.claude_desktop_config());
    assert_eq!(written["mcpServers"]["stave"]["command"], STAVE);
}

#[test]
fn a_config_that_does_not_read_is_left_alone() {
    let home = Home::new();
    let config = home.claude_desktop_config();
    fs::create_dir_all(config.parent().unwrap()).unwrap();
    fs::write(&config, "{ not json").unwrap();
    let results = install_in(&home.places(), &[Host::ClaudeDesktop], Path::new(STAVE));
    let error = results.into_iter().next().unwrap().unwrap_err();
    assert!(
        error.to_string().contains("claude_desktop_config.json"),
        "{error}"
    );
    assert_eq!(fs::read_to_string(&config).unwrap(), "{ not json");
}

#[test]
fn installs_codex_keeping_other_tables() {
    let home = Home::new();
    let config = home.codex_config();
    fs::create_dir_all(config.parent().unwrap()).unwrap();
    let before = r#"# My Codex settings
model = "o4"

[mcp_servers.files]
command = "npx"
args = ["files-server"]

[profiles.quiet]
approval_policy = "never"
"#;
    fs::write(&config, before).unwrap();
    let results = install_in(&home.places(), &[Host::Codex], Path::new(STAVE));
    let installed = results.into_iter().next().unwrap().unwrap();
    assert_eq!(installed.host, Host::Codex);
    let written = fs::read_to_string(&config).unwrap();
    // Everything that was there is still there, comments included.
    for line in before.lines() {
        assert!(written.contains(line), "lost {line:?} from:\n{written}");
    }
    let table: toml_edit::DocumentMut = written.parse().unwrap();
    assert_eq!(
        table["mcp_servers"]["stave"]["command"].as_str(),
        Some(STAVE)
    );
    let args = table["mcp_servers"]["stave"]["args"].as_array().unwrap();
    assert_eq!(
        args.iter().map(|value| value.as_str()).collect::<Vec<_>>(),
        [Some("mcp")]
    );
}

#[test]
fn installs_claude_code_through_its_command() {
    let home = Home::new();
    home.fake_claude();
    let results = install_in(&home.places(), &[Host::ClaudeCode], Path::new(STAVE));
    let installed = results.into_iter().next().unwrap().unwrap();
    assert_eq!(installed.host, Host::ClaudeCode);
    assert!(
        home.claude_log()
            .contains(&format!("mcp add --scope user stave -- {STAVE} mcp")),
        "{}",
        home.claude_log()
    );
    assert_eq!(home.claude_servers(), "stave\n");
}

#[test]
fn claude_code_without_its_command_is_an_error() {
    let home = Home::new();
    let results = install_in(&home.places(), &[Host::ClaudeCode], Path::new(STAVE));
    let error = results.into_iter().next().unwrap().unwrap_err();
    assert!(error.to_string().contains("claude"), "{error}");
}

#[test]
fn uninstall_removes_only_stave() {
    let home = Home::new();
    home.fake_claude();
    let desktop = home.claude_desktop_config();
    fs::create_dir_all(desktop.parent().unwrap()).unwrap();
    fs::write(
        &desktop,
        r#"{"mcpServers": {"files": {"command": "npx", "args": []}}, "theme": "dark"}"#,
    )
    .unwrap();
    let codex = home.codex_config();
    fs::create_dir_all(codex.parent().unwrap()).unwrap();
    fs::write(
        &codex,
        "model = \"o4\"\n\n[mcp_servers.files]\ncommand = \"npx\"\n",
    )
    .unwrap();
    let all = [Host::ClaudeDesktop, Host::ClaudeCode, Host::Codex];
    for result in install_in(&home.places(), &all, Path::new(STAVE)) {
        result.unwrap();
    }
    for result in uninstall_in(&home.places(), &all) {
        let removed = result.unwrap();
        assert!(removed.how.contains("removed"), "{}", removed.how);
    }
    let written = json(&desktop);
    assert_eq!(written["theme"], "dark");
    assert_eq!(written["mcpServers"]["files"]["command"], "npx");
    assert!(written["mcpServers"].get("stave").is_none());
    let table: toml_edit::DocumentMut = fs::read_to_string(&codex).unwrap().parse().unwrap();
    assert_eq!(table["model"].as_str(), Some("o4"));
    assert_eq!(
        table["mcp_servers"]["files"]["command"].as_str(),
        Some("npx")
    );
    assert!(table["mcp_servers"].get("stave").is_none());
    assert_eq!(home.claude_servers(), "");
    // A second uninstall finds nothing to remove, and says so.
    for result in uninstall_in(&home.places(), &all) {
        let removed = result.unwrap();
        assert!(removed.how.contains("wasn't set up"), "{}", removed.how);
    }
}

#[test]
fn install_twice_is_idempotent() {
    let home = Home::new();
    home.fake_claude();
    fs::create_dir_all(home.path().join(".config/Claude")).unwrap();
    fs::create_dir_all(home.path().join(".codex")).unwrap();
    let all = [Host::ClaudeDesktop, Host::ClaudeCode, Host::Codex];
    for result in install_in(&home.places(), &all, Path::new(STAVE)) {
        result.unwrap();
    }
    let desktop = fs::read(home.claude_desktop_config()).unwrap();
    let codex = fs::read(home.codex_config()).unwrap();
    for result in install_in(&home.places(), &all, Path::new(STAVE)) {
        result.unwrap();
    }
    assert_eq!(fs::read(home.claude_desktop_config()).unwrap(), desktop);
    assert_eq!(fs::read(home.codex_config()).unwrap(), codex);
    assert_eq!(home.claude_servers(), "stave\n");
}

#[test]
fn install_again_with_a_moved_stave_updates_the_command() {
    let home = Home::new();
    fs::create_dir_all(home.path().join(".codex")).unwrap();
    let places = home.places();
    install_in(&places, &[Host::Codex], Path::new(STAVE))
        .remove(0)
        .unwrap();
    install_in(&places, &[Host::Codex], Path::new("/usr/bin/stave"))
        .remove(0)
        .unwrap();
    let table: toml_edit::DocumentMut = fs::read_to_string(home.codex_config())
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(
        table["mcp_servers"]["stave"]["command"].as_str(),
        Some("/usr/bin/stave")
    );
}

#[test]
fn found_hosts_are_the_apps_in_the_home_and_on_the_path() {
    let home = Home::new();
    assert_eq!(found_hosts_in(&home.places()), []);
    fs::create_dir_all(home.path().join(".config/Claude")).unwrap();
    assert_eq!(found_hosts_in(&home.places()), [Host::ClaudeDesktop]);
    home.fake_claude();
    fs::create_dir_all(home.path().join(".codex")).unwrap();
    assert_eq!(
        found_hosts_in(&home.places()),
        [Host::ClaudeDesktop, Host::ClaudeCode, Host::Codex]
    );
}
