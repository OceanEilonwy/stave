//! Claude Code: its own command, `claude mcp add --scope user stave --
//! <stave> mcp`, run only when `claude` is on the path. Installing first
//! removes the user's entry (`claude mcp remove --scope user stave`), so
//! a moved `stave` is picked up; that removal failing just means there
//! was none. Nothing is decided from `claude mcp get`, which also finds
//! a project's or a folder's own `stave` entry, and every command runs
//! in the home folder, so a project's `.mcp.json` doesn't come into it.

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
    // An older entry goes first; when there is none this fails, and that
    // is fine.
    let _none_before = run(places, &program, &REMOVE)?;
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
    run(places, &program, &arguments)?.map_err(|failed| failed.error())?;
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
