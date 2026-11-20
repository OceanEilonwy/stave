//! Claude Code: its own command, `claude mcp add --scope user stave --
//! <stave> mcp`, run only when `claude` is on the path. The user's entry
//! as it stands is read from `~/.claude.json`: when it already runs this
//! `stave`, nothing is run. Otherwise installing first removes it (`claude
//! mcp remove --scope user stave`), so a moved `stave` is picked up; that
//! removal failing just means there was none. When the add then fails,
//! the entry that was removed is put back. Nothing is decided from
//! `claude mcp get`, which also finds a project's or a folder's own
//! `stave` entry, and every command runs in the home folder, so a
//! project's `.mcp.json` doesn't come into it.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::{
    Host, Installed, LinkError, Places, SERVER_ARGUMENT, SERVER_NAME, home_variable, utf8,
};

/// The command.
pub(crate) const PROGRAM: &str = "claude";

/// The arguments that remove the user's entry.
const REMOVE: [&str; 5] = ["mcp", "remove", "--scope", "user", SERVER_NAME];

/// Adds Stave as a server for the user (every project).
pub(crate) fn install(places: &Places, stave: &Path) -> Result<Installed, LinkError> {
    let program = program(places)?;
    let stave = utf8(stave)?;
    let before = user_entry(places);
    if before
        .as_ref()
        .is_some_and(|(command, arguments)| command == stave && arguments == &[SERVER_ARGUMENT])
    {
        return Ok(Installed {
            host: Host::ClaudeCode,
            how: "Stave was already set up in Claude Code".to_owned(),
        });
    }
    // An older entry goes first; when there is none this fails, and that
    // is fine.
    let removed = run(places, &program, &REMOVE)?.is_ok();
    let arguments = [
        "mcp",
        "add",
        "--scope",
        "user",
        SERVER_NAME,
        "--",
        stave,
        SERVER_ARGUMENT,
    ];
    if let Err(mut failed) = run(places, &program, &arguments)? {
        if removed {
            let put_back = before.is_some_and(|(command, old)| {
                let mut again = vec!["mcp", "add", "--scope", "user", SERVER_NAME, "--"];
                again.push(&command);
                again.extend(old.iter().map(String::as_str));
                matches!(run(places, &program, &again), Ok(Ok(())))
            });
            failed.output.push_str(if put_back {
                " (the Stave entry Claude Code had before was put back)"
            } else {
                " (the Stave entry Claude Code had before was removed; run the setup again)"
            });
        }
        return Err(failed.error());
    }
    Ok(Installed {
        host: Host::ClaudeCode,
        how: format!(
            "ran `{}`; Claude Code can use Stave in its next session",
            command_line(&arguments)
        ),
    })
}

/// Removes the user's Stave server, when there is one.
pub(crate) fn uninstall(places: &Places) -> Result<Installed, LinkError> {
    let program = program(places)?;
    if run(places, &program, &REMOVE)?.is_err() {
        return Ok(Installed {
            host: Host::ClaudeCode,
            how: "Stave wasn't set up in Claude Code".to_owned(),
        });
    }
    Ok(Installed {
        host: Host::ClaudeCode,
        how: format!(
            "removed Stave from Claude Code (`{}`)",
            command_line(&REMOVE)
        ),
    })
}

/// The user's `stave` entry as `~/.claude.json` holds it: its command
/// and arguments, when it is there and reads.
fn user_entry(places: &Places) -> Option<(String, Vec<String>)> {
    let text = std::fs::read_to_string(places.home.join(".claude.json")).ok()?;
    let settings: serde_json::Value = serde_json::from_str(&text).ok()?;
    let entry = settings.get("mcpServers")?.get(SERVER_NAME)?;
    let command = entry.get("command")?.as_str()?.to_owned();
    let arguments = match entry.get("args") {
        Some(arguments) => arguments
            .as_array()?
            .iter()
            .map(|argument| argument.as_str().map(ToOwned::to_owned))
            .collect::<Option<Vec<String>>>()?,
        None => Vec::new(),
    };
    Some((command, arguments))
}

/// Where `claude` is.
fn program(places: &Places) -> Result<PathBuf, LinkError> {
    places.find_program(PROGRAM).ok_or(LinkError::NoCommand {
        host: Host::ClaudeCode,
        program: PROGRAM,
    })
}

/// A run of the command that exited with a failure.
struct Failed {
    command: String,
    output: String,
}

impl Failed {
    fn error(self) -> LinkError {
        LinkError::Failed {
            command: self.command,
            output: self.output,
        }
    }
}

/// Runs the command in, and with, the home of `places`: `Err` when it
/// can't be run at all, `Ok(Err)` when it ran and failed.
fn run(
    places: &Places,
    program: &Path,
    arguments: &[&str],
) -> Result<Result<(), Failed>, LinkError> {
    let (name, home) = home_variable(places);
    let output = Command::new(program)
        .args(arguments)
        .current_dir(&places.home)
        .env(name, home)
        .output()
        .map_err(|source| LinkError::Run {
            program: program.to_path_buf(),
            source,
        })?;
    if output.status.success() {
        return Ok(Ok(()));
    }
    let said = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    let said = if said.is_empty() {
        String::from_utf8_lossy(&output.stdout).trim().to_owned()
    } else {
        said
    };
    Ok(Err(Failed {
        command: command_line(arguments),
        output: if said.is_empty() {
            output.status.to_string()
        } else {
            said
        },
    }))
}

/// The command line as a person would type it.
fn command_line(arguments: &[&str]) -> String {
    std::iter::once(PROGRAM)
        .chain(arguments.iter().copied())
        .collect::<Vec<_>>()
        .join(" ")
}
