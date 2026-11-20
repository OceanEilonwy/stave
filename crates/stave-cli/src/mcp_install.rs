//! `stave mcp install` and `stave mcp uninstall`: set Stave up in the AI
//! apps (Claude Desktop, Claude Code, Codex), or take it out, through
//! `stave-link`.
//!
//! ```text
//! stave mcp install                  # every app found on this computer
//! stave mcp install --codex          # only the apps named
//! stave mcp uninstall --claude-desktop
//! ```
//!
//! Each app's line says what was done (on standard output) or why it
//! couldn't be (on standard error). Exit codes: 0 when every app named
//! (or found) was done, 1 when one failed or none was found.

use anyhow::{Context as _, Result};
use clap::{Args, Subcommand};
use stave_link::{Host, Installed, LinkError};

/// What `stave mcp` does besides running the server.
#[derive(Debug, Subcommand)]
pub(crate) enum McpCommand {
    /// Set Stave up in the AI apps on this computer, so they can open,
    /// check and change your projects. With no flag, every app found.
    Install(HostFlags),
    /// Take Stave out of the AI apps. With no flag, every app found.
    Uninstall(HostFlags),
}

/// Which apps.
#[derive(Debug, Args)]
pub(crate) struct HostFlags {
    /// Claude Desktop.
    #[arg(long)]
    claude_desktop: bool,
    /// Claude Code (runs `claude mcp add`).
    #[arg(long)]
    claude_code: bool,
    /// Codex.
    #[arg(long)]
    codex: bool,
}

impl HostFlags {
    /// The apps named, or every app found when none is.
    fn hosts(&self) -> Vec<Host> {
        let named: Vec<Host> = Host::ALL
            .into_iter()
            .filter(|host| match host {
                Host::ClaudeDesktop => self.claude_desktop,
                Host::ClaudeCode => self.claude_code,
                Host::Codex => self.codex,
            })
            .collect();
        if named.is_empty() {
            stave_link::found_hosts()
        } else {
            named
        }
    }
}

/// Runs `stave mcp install` or `stave mcp uninstall`: `true` when every
/// app was done.
pub(crate) fn run(command: &McpCommand) -> Result<bool> {
    let (flags, installing) = match command {
        McpCommand::Install(flags) => (flags, true),
        McpCommand::Uninstall(flags) => (flags, false),
    };
    let hosts = flags.hosts();
    if hosts.is_empty() {
        eprintln!(
            "No AI app was found. Stave works with Claude Desktop, Claude Code or Codex: \
             install one, or name it (--claude-desktop, --claude-code, --codex)."
        );
        return Ok(false);
    }
    let results = if installing {
        let stave = std::env::current_exe()
            .and_then(|path| path.canonicalize())
            .context("can't find where this stave program is")?;
        stave_link::install(&hosts, &stave)
    } else {
        stave_link::uninstall(&hosts)
    };
    Ok(report(&hosts, results))
}

/// Prints each app's result: `true` when none failed.
fn report(hosts: &[Host], results: Vec<Result<Installed, LinkError>>) -> bool {
    let mut all_done = true;
    for (host, result) in hosts.iter().zip(results) {
        match result {
            Ok(installed) => println!("{}: {}", installed.host, installed.how),
            Err(error) => {
                all_done = false;
                eprintln!("{host}: {error}");
            }
        }
    }
    all_done
}
