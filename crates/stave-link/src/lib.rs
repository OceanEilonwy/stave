//! Links Stave to the AI apps people already use, and to the running Stave
//! window (`docs/superpowers/specs/2026-11-20-ai-integration-design.md`,
//! "Installing", and `show_in_stave` under "Preview").
//!
//! ```text
//! install(hosts, stave) ──▶ Claude Desktop  ~/.config/Claude/claude_desktop_config.json  mcpServers.stave
//!                       ├─▶ Claude Code     claude mcp add --scope user stave -- <stave> mcp
//!                       └─▶ Codex           ~/.codex/config.toml                         [mcp_servers.stave]
//! show_in_stave(folder, patch) ──D-Bus──▶ io.github.OceanEilonwy.Stave  app.show-project (ss)
//! ```
//!
//! Every app gets the same server entry: the `stave` program with the
//! argument `mcp`. Installing keeps everything else in the app's settings
//! (other servers, other tables, comments in Codex's TOML), installing
//! twice changes nothing, and uninstalling removes Stave's entry alone.
//! Where the settings live comes from [`Places`]: [`Places::from_environment`]
//! reads `HOME` (and `XDG_CONFIG_HOME`, `CODEX_HOME`, `PATH`), and a test
//! builds one over a temporary folder.
//!
//! Nothing here goes on the network: the only other process spoken to is
//! the `claude` program, and the Stave window over the session bus.

#![forbid(unsafe_code)]

use std::ffi::OsString;
use std::fmt;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};

mod claude_code;
mod claude_desktop;
mod codex;
mod files;
mod show;

pub use show::{
    APPLICATION_ID, OBJECT_PATH, SHOW_PROJECT_ACTION, SHOW_PROJECT_PARAMETER, Shown,
    show_in_application, show_in_stave,
};

/// The server's name in every app's settings.
pub const SERVER_NAME: &str = "stave";

/// The argument that runs Stave as a server: `stave mcp`.
pub const SERVER_ARGUMENT: &str = "mcp";

/// An AI app Stave can be set up in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Host {
    /// Claude Desktop, through its settings file.
    ClaudeDesktop,
    /// Claude Code, through its `claude` command.
    ClaudeCode,
    /// Codex, through its settings file.
    Codex,
}

impl Host {
    /// Every app, in the order they are set up and reported.
    pub const ALL: [Host; 3] = [Host::ClaudeDesktop, Host::ClaudeCode, Host::Codex];

    /// The app's own name: "Claude Desktop".
    pub fn name(self) -> &'static str {
        match self {
            Host::ClaudeDesktop => "Claude Desktop",
            Host::ClaudeCode => "Claude Code",
            Host::Codex => "Codex",
        }
    }
}

impl fmt::Display for Host {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// What installing or uninstalling did in one app.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Installed {
    /// The app.
    pub host: Host,
    /// What was done, in plain words: "added Stave to Claude Desktop's
    /// settings in …; restart Claude Desktop to use it".
    pub how: String,
}

/// Why Stave couldn't be set up in (or taken out of) an app, or couldn't
/// reach the window.
#[derive(Debug, thiserror::Error)]
pub enum LinkError {
    /// `HOME` isn't set, so the apps' settings can't be found.
    #[error("HOME isn't set, so Stave can't find the AI apps' settings")]
    NoHome,
    /// A path that the settings file can't hold: not UTF-8.
    #[error("the path '{}' isn't UTF-8, so it can't be written into a settings file", .0.display())]
    NotUtf8(PathBuf),
    /// The `stave` to set up isn't a full path, so the app would run
    /// whatever `stave` its own working folder leads to.
    #[error("the path '{}' isn't a full path, so it can't be written into a settings file", .0.display())]
    NotAbsolute(PathBuf),
    /// A settings file couldn't be read.
    #[error("can't read '{}': {source}", path.display())]
    Read {
        /// The file.
        path: PathBuf,
        /// Why.
        source: std::io::Error,
    },
    /// A settings file couldn't be written.
    #[error("can't write '{}': {source}", path.display())]
    Write {
        /// The file.
        path: PathBuf,
        /// Why.
        source: std::io::Error,
    },
    /// A settings file isn't what its app writes; it is left alone.
    #[error("'{}' doesn't read as the app's settings ({message}); it was left as it is", path.display())]
    Settings {
        /// The file.
        path: PathBuf,
        /// What is wrong with it.
        message: String,
    },
    /// The app's command isn't on `PATH`.
    #[error("can't find the '{program}' command for {host}; is {host} installed?")]
    NoCommand {
        /// The app.
        host: Host,
        /// The command: `claude`.
        program: &'static str,
    },
    /// The app's command couldn't be run.
    #[error("can't run '{}': {source}", program.display())]
    Run {
        /// The command.
        program: PathBuf,
        /// Why.
        source: std::io::Error,
    },
    /// The app's command ran and failed.
    #[error("'{command}' failed: {output}")]
    Failed {
        /// The command line as run.
        command: String,
        /// What it said.
        output: String,
    },
    /// The folder to show isn't a path the window can be sent: not
    /// absolute, or not UTF-8.
    #[error("can't send the folder '{}' to Stave: {reason}", folder.display())]
    Folder {
        /// The folder.
        folder: PathBuf,
        /// Why.
        reason: &'static str,
    },
    /// There is no session bus to reach the window over.
    #[error("can't reach the desktop's session bus: {0}")]
    NoSessionBus(zbus::Error),
    /// The window was reached and the call failed.
    #[error("Stave's window didn't take the request: {0}")]
    Bus(#[from] zbus::Error),
}

/// Where the apps keep their settings, and where commands are looked up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Places {
    /// The user's settings folder: `~/.config`.
    config: PathBuf,
    /// Codex's own folder: `~/.codex`.
    codex_home: PathBuf,
    /// The folders searched for a command, in order.
    search_path: Vec<PathBuf>,
    /// The home folder, handed on to the commands run.
    home: PathBuf,
}

impl Places {
    /// The places under `home` (`home/.config`, `home/.codex`), with
    /// commands looked up in `search_path`.
    pub fn new(home: PathBuf, search_path: Vec<PathBuf>) -> Places {
        Places {
            config: home.join(".config"),
            codex_home: home.join(".codex"),
            search_path,
            home,
        }
    }

    /// The places of the user running Stave: under `HOME`, with
    /// `XDG_CONFIG_HOME` and `CODEX_HOME` when they are set to absolute
    /// paths, and commands looked up on `PATH`.
    ///
    /// # Errors
    ///
    /// [`LinkError::NoHome`] when `HOME` isn't set (or is empty).
    pub fn from_environment() -> Result<Places, LinkError> {
        let home = std::env::var_os("HOME")
            .filter(|home| !home.is_empty())
            .map(PathBuf::from)
            .ok_or(LinkError::NoHome)?;
        let search_path = std::env::var_os("PATH")
            .map(|path| std::env::split_paths(&path).collect())
            .unwrap_or_default();
        let mut places = Places::new(home, search_path);
        if let Some(config) = absolute_variable("XDG_CONFIG_HOME") {
            places.config = config;
        }
        if let Some(codex_home) = absolute_variable("CODEX_HOME") {
            places.codex_home = codex_home;
        }
        Ok(places)
    }

    /// Claude Desktop's settings folder.
    fn claude_desktop_folder(&self) -> PathBuf {
        self.config.join("Claude")
    }

    /// Claude Desktop's settings file.
    fn claude_desktop_config(&self) -> PathBuf {
        self.claude_desktop_folder()
            .join("claude_desktop_config.json")
    }

    /// Codex's settings file.
    fn codex_config(&self) -> PathBuf {
        self.codex_home.join("config.toml")
    }

    /// Where a command is found: the first executable file of that name
    /// in the search path. Folders on the path that aren't absolute (`.`,
    /// `bin`, an empty entry) are skipped: what they find depends on the
    /// folder a program happens to run in, so another app could later run
    /// a different program than the one found.
    pub fn find_program(&self, name: &str) -> Option<PathBuf> {
        self.search_path
            .iter()
            .filter(|folder| folder.is_absolute())
            .map(|folder| folder.join(name))
            .find(|candidate| {
                candidate.metadata().is_ok_and(|metadata| {
                    metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
                })
            })
    }
}

/// An environment variable holding an absolute path.
fn absolute_variable(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
}

/// Sets Stave up in each app, running `stave` as `stave mcp`: one result
/// per app, in the order given. Uses [`Places::from_environment`].
pub fn install(hosts: &[Host], stave: &Path) -> Vec<Result<Installed, LinkError>> {
    match Places::from_environment() {
        Ok(places) => install_in(&places, hosts, stave),
        Err(_no_home) => every_host_fails(hosts),
    }
}

/// Takes Stave out of each app: one result per app, in the order given.
/// Uses [`Places::from_environment`].
pub fn uninstall(hosts: &[Host]) -> Vec<Result<Installed, LinkError>> {
    match Places::from_environment() {
        Ok(places) => uninstall_in(&places, hosts),
        Err(_no_home) => every_host_fails(hosts),
    }
}

/// The apps found on this computer, in [`Host::ALL`]'s order. Uses
/// [`Places::from_environment`]; none when `HOME` isn't set.
pub fn found_hosts() -> Vec<Host> {
    Places::from_environment()
        .map(|places| found_hosts_in(&places))
        .unwrap_or_default()
}

/// [`install`] with the apps' settings in `places`.
pub fn install_in(
    places: &Places,
    hosts: &[Host],
    stave: &Path,
) -> Vec<Result<Installed, LinkError>> {
    if !stave.is_absolute() {
        return hosts
            .iter()
            .map(|_| Err(LinkError::NotAbsolute(stave.to_path_buf())))
            .collect();
    }
    hosts
        .iter()
        .map(|host| match host {
            Host::ClaudeDesktop => claude_desktop::install(places, stave),
            Host::ClaudeCode => claude_code::install(places, stave),
            Host::Codex => codex::install(places, stave),
        })
        .collect()
}

/// [`uninstall`] with the apps' settings in `places`.
pub fn uninstall_in(places: &Places, hosts: &[Host]) -> Vec<Result<Installed, LinkError>> {
    hosts
        .iter()
        .map(|host| match host {
            Host::ClaudeDesktop => claude_desktop::uninstall(places),
            Host::ClaudeCode => claude_code::uninstall(places),
            Host::Codex => codex::uninstall(places),
        })
        .collect()
}

/// [`found_hosts`] with the apps' settings in `places`: Claude Desktop
/// when its settings folder is there, Claude Code when `claude` is on the
/// path, Codex when its folder is there or `codex` is on the path.
pub fn found_hosts_in(places: &Places) -> Vec<Host> {
    Host::ALL
        .into_iter()
        .filter(|host| match host {
            Host::ClaudeDesktop => places.claude_desktop_folder().is_dir(),
            Host::ClaudeCode => places.find_program(claude_code::PROGRAM).is_some(),
            Host::Codex => places.codex_home.is_dir() || places.find_program("codex").is_some(),
        })
        .collect()
}

/// The same failure for every app: the places couldn't be found (only
/// [`LinkError::NoHome`] stops [`Places::from_environment`]).
fn every_host_fails(hosts: &[Host]) -> Vec<Result<Installed, LinkError>> {
    hosts.iter().map(|_| Err(LinkError::NoHome)).collect()
}

/// A path as text for a settings file.
fn utf8(path: &Path) -> Result<&str, LinkError> {
    path.to_str()
        .ok_or_else(|| LinkError::NotUtf8(path.to_path_buf()))
}

/// The environment a command is run with: the home of `places`.
fn home_variable(places: &Places) -> (&'static str, OsString) {
    ("HOME", places.home.clone().into_os_string())
}
