//! Stave as an MCP server (`stave mcp`): the AI app a person already uses
//! (Claude Desktop, Claude Code, Codex) opens their projects, checks them,
//! looks up the language and tries changes written as patches, all
//! through the same pipeline the window and `stave check` use.
//!
//! ```text
//! AI app ──stdio, JSON-RPC──▶ StaveServer ──▶ tools/{projects,looking,changing}
//!                                              └──▶ stave-app (open, check, try a patch)
//! ```
//!
//! One server serves one connection, and so one agent: the AI app named
//! by its `clientInfo` plus a short id ([`Agent`]). The server holds the
//! project the agent opened ([`OpenProject`]) and reads its files from
//! disk afresh for every tool, so it always sees what the window last
//! saved. Every tool answers with one text that stands on its own (Claude
//! Code and Codex show only that) and the same facts as structured JSON.
//!
//! Nothing here changes a project except `new_project`, which writes a
//! new project into an empty folder. Paths are never trusted:
//! [`project_path`] refuses one that leads out of the project folder.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::model::{Implementation, ServerCapabilities, ServerConfig};
use rmcp::{Peer, RoleServer, ServerHandler, ServiceExt as _, tool_handler};
use tokio::sync::Mutex;

mod error;
pub mod tools;

pub use error::McpError;

/// Where the server finds what lives outside a project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerOptions {
    /// Stave's settings folder, `~/.config/stave`.
    pub config_dir: PathBuf,
    /// Stave's cache folder, `~/.cache/stave`: disposable scratch space.
    pub cache_dir: PathBuf,
    /// The desktop's list of recently used files, where the window
    /// records the projects it opened: `~/.local/share/recently-used.xbel`.
    pub recent_projects: PathBuf,
}

impl ServerOptions {
    /// The folders of the person running Stave, from the XDG base
    /// directory variables, falling back to `$HOME/.config`,
    /// `$HOME/.cache` and `$HOME/.local/share` as the spec says.
    ///
    /// # Errors
    ///
    /// [`McpError::NoHome`] when `HOME` is needed and not set.
    pub fn from_environment() -> Result<Self, McpError> {
        let base = |variable: &str, fallback: &str| -> Result<PathBuf, McpError> {
            match std::env::var_os(variable).map(PathBuf::from) {
                Some(path) if path.is_absolute() => Ok(path),
                Some(_) | None => std::env::var_os("HOME")
                    .map(|home| PathBuf::from(home).join(fallback))
                    .ok_or(McpError::NoHome),
            }
        };
        Ok(Self {
            config_dir: base("XDG_CONFIG_HOME", ".config")?.join("stave"),
            cache_dir: base("XDG_CACHE_HOME", ".cache")?.join("stave"),
            recent_projects: base("XDG_DATA_HOME", ".local/share")?.join("recently-used.xbel"),
        })
    }
}

/// The project an agent opened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenProject {
    /// Its folder, as a full path with no symbolic links.
    pub folder: PathBuf,
}

/// The agent on the other end of a connection: the AI app, as it names
/// itself in `clientInfo`, and a short id that tells two connections of
/// the same app apart.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Agent {
    /// The app's name: "Claude Desktop", "Codex".
    pub name: String,
    /// Four hex digits, different for every connection this process
    /// serves and, through the process id, for most processes.
    pub id: String,
}

impl Agent {
    /// The agent named `name`, with the next id.
    fn new(name: &str) -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let count = NEXT.fetch_add(1, Ordering::Relaxed);
        let id = std::process::id().wrapping_add(count) & 0xffff;
        let name = name.trim();
        Self {
            name: if name.is_empty() {
                "AI app".to_owned()
            } else {
                name.to_owned()
            },
            id: format!("{id:04x}"),
        }
    }
}

/// What a connection holds between tool calls.
#[derive(Debug, Default)]
struct Session {
    /// The project the agent opened, if any.
    open: Option<OpenProject>,
    /// The agent, once its app said who it is.
    agent: Option<Agent>,
}

/// The MCP server for one connection.
#[derive(Debug, Clone)]
pub struct StaveServer {
    options: Arc<ServerOptions>,
    session: Arc<Mutex<Session>>,
    tool_router: ToolRouter<Self>,
}

impl StaveServer {
    /// A server with no project open.
    pub fn new(options: ServerOptions) -> Self {
        Self {
            options: Arc::new(options),
            session: Arc::new(Mutex::new(Session::default())),
            tool_router: tools::router(),
        }
    }

    /// Where the server finds what lives outside a project.
    pub fn options(&self) -> &ServerOptions {
        &self.options
    }

    /// The project the agent opened, if any.
    pub async fn current_project(&self) -> Option<OpenProject> {
        self.session.lock().await.open.clone()
    }

    /// The project the agent opened.
    ///
    /// # Errors
    ///
    /// [`McpError::NoProject`] before `open_project`.
    pub async fn require_project(&self) -> Result<OpenProject, McpError> {
        self.current_project().await.ok_or(McpError::NoProject)
    }

    /// Makes `project` the open project.
    pub async fn set_open_project(&self, project: OpenProject) {
        self.session.lock().await.open = Some(project);
    }

    /// The agent on the other end, named from the app's `clientInfo` the
    /// first time it is asked for.
    pub async fn agent(&self, peer: &Peer<RoleServer>) -> Agent {
        let mut session = self.session.lock().await;
        session
            .agent
            .get_or_insert_with(|| {
                let name = peer
                    .peer_info()
                    .map(|info| info.client_info.name.clone())
                    .unwrap_or_default();
                Agent::new(&name)
            })
            .clone()
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for StaveServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("stave", env!("CARGO_PKG_VERSION")))
            .with_instructions(INSTRUCTIONS)
    }
}

/// What the AI app is told about the server when it connects.
const INSTRUCTIONS: &str = "Stave is a woodworking design tool. A project is a folder of .stave \
files that describe parts, stock, joints and hardware by their relationships. Open a project \
with open_project (or make one with new_project), read it with describe and check, and change \
it by writing a patch in the .stave grammar: look the grammar up with grammar and the standard \
kinds with std_lookup, then try the patch with try_patch, which checks it on a scratch copy and \
writes nothing. A problem's fixes are patches too (fixes_for).";

/// Serves one connection over standard input and output until the AI app
/// closes it: what `stave mcp` runs.
///
/// # Errors
///
/// When the connection can't be set up or stops unexpectedly.
pub async fn serve_stdio(options: ServerOptions) -> Result<(), McpError> {
    let running = StaveServer::new(options)
        .serve(rmcp::transport::stdio())
        .await
        .map_err(Box::new)?;
    running.waiting().await?;
    Ok(())
}

/// [`serve_stdio`] on a runtime of its own, for a caller that has none
/// (the `stave` command).
///
/// # Errors
///
/// As [`serve_stdio`], and [`McpError::Runtime`] when the runtime can't
/// be started.
pub fn serve_stdio_blocking(options: ServerOptions) -> Result<(), McpError> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(McpError::Runtime)?
        .block_on(serve_stdio(options))
}

/// The full path of `given`, a path inside the open project written
/// relative to its folder (`carcass.stave`, `hardware/hinges.stave`).
///
/// # Errors
///
/// [`McpError::OutsideProject`] for an absolute path, a path with `..`
/// or one that a symbolic link leads out of the folder;
/// [`McpError::Io`] when the folder itself can't be read.
pub fn project_path(open: &OpenProject, given: &str) -> Result<PathBuf, McpError> {
    let outside = || McpError::OutsideProject(given.to_owned());
    let mut relative = PathBuf::new();
    for component in Path::new(given).components() {
        match component {
            Component::Normal(part) => relative.push(part),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(outside());
            }
        }
    }
    let folder = open.folder.canonicalize().map_err(|source| McpError::Io {
        path: open.folder.clone(),
        source,
    })?;
    let joined = folder.join(&relative);
    // The deepest part of the path that exists, with every link in it
    // followed, must still be inside the folder.
    let mut existing = joined.as_path();
    while std::fs::symlink_metadata(existing).is_err() {
        existing = existing.parent().ok_or_else(outside)?;
    }
    let real = existing.canonicalize().map_err(|_| outside())?;
    if real.starts_with(&folder) {
        Ok(joined)
    } else {
        Err(outside())
    }
}
