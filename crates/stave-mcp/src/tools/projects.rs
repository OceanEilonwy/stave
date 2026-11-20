//! Projects: `list_projects`, `open_project`, `new_project`.
//!
//! A project folder is given as a full path and used as the path really
//! leads (symbolic links followed); everything after works inside that
//! folder only.
//!
//! A project's versions are turned on only when asked: `open_project`
//! says when they are off, and turns them on when called again with
//! `start_versions`, recording the folder's files as the first version.
//! `new_project` counts as applying a change, so it follows the merge
//! policy: refused under `off`, and otherwise the new project starts with
//! versions on and its first version recorded as the agent's.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::CallToolResult;
use rmcp::{Peer, RoleServer, tool, tool_router};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;
use stave_app::Opened;
use stave_app::script::{parse_script, run};
use stave_history::{MergePolicy, merge_policy};

use super::looking::count_words;
use super::{answer, blocking, finish};
use crate::agent_line;
use crate::{McpError, OpenProject, StaveServer};

/// The message of the version that records a project's files as they
/// were when its versions were turned on.
const FIRST_VERSION: &str = "The project as it was when versions were turned on";

/// The message of a new project's first version.
const NEW_PROJECT_VERSION: &str = "Made the project";

/// How many recent projects are listed, as the window's Open Recent menu.
const RECENT_LIMIT: usize = 8;

/// The name the window records its projects under in the recent list.
const RECENT_APPLICATION: &str = "Stave";

/// The namespace of the desktop bookmark spec's elements.
const BOOKMARK_NAMESPACE: &str = "http://www.freedesktop.org/standards/desktop-bookmarks";

/// `open_project`'s arguments.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct OpenArguments {
    /// The project folder's full path, starting with `/`.
    pub path: String,
    /// Turn the project's versions on when they are off, so changes can
    /// be suggested and applied. Ask the person first.
    #[serde(default)]
    pub start_versions: bool,
}

/// `new_project`'s arguments.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct NewArguments {
    /// The full path of the folder to make the project in: an empty
    /// folder, or one that doesn't exist yet.
    pub folder: String,
    /// The steps that describe the project, one a line, as `stave edit
    /// --script` reads them: `new "<title>" <width> <height> <depth>`,
    /// then `import std:<module>`, `stock <name> <species> <form>
    /// <thickness> ...`, `tools ...`, `create`; then, to add to it,
    /// `select region <name>`, `choose <module>.<Entry>`, `set <setting> =
    /// <value>`, `add`.
    pub steps: String,
}

/// A project the window opened lately.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RecentProject {
    /// Its folder.
    pub folder: PathBuf,
    /// The folder's name.
    pub name: String,
}

#[tool_router(router = projects_router, vis = "pub(crate)")]
impl StaveServer {
    /// The projects opened lately in the Stave window.
    #[tool(
        description = "List the Stave projects opened lately in the Stave window, newest first. \
                       Open one with open_project.",
        annotations(read_only_hint = true)
    )]
    async fn list_projects(&self) -> CallToolResult {
        let projects = recent_projects(&self.options().recent_projects);
        let mut text = String::new();
        if projects.is_empty() {
            text.push_str(
                "No Stave projects were opened lately. Ask for the project's folder and call \
                 open_project with its full path.",
            );
        } else {
            text.push_str("Stave projects opened lately, newest first:\n");
            for project in &projects {
                let _ = writeln!(text, "  {}: {}", project.name, project.folder.display());
            }
        }
        answer(text, json!({ "projects": projects }))
    }

    /// Opens a project for this session.
    #[tool(
        description = "Open a Stave project folder for this session, given its full path. Says \
                       how many parts it has, what problems it has, and whether its versions \
                       are on (with how many versions and suggestions). Versions are needed to \
                       suggest changes; when they are off, ask the person, then call again with \
                       start_versions: true. The other tools work on the project opened last.",
        annotations(read_only_hint = false, destructive_hint = false)
    )]
    async fn open_project(
        &self,
        Parameters(arguments): Parameters<OpenArguments>,
    ) -> CallToolResult {
        finish(
            self.open_folder(arguments.path, arguments.start_versions)
                .await,
        )
    }

    /// Makes a new project in an empty folder.
    #[tool(
        description = "Make a new Stave project in an empty folder (or one that doesn't exist \
                       yet) from steps, one a line, the way a person builds it in the window: \
                       new \"<title>\" <width> <height> <depth>, import std:<module>, stock \
                       <name> <species> solid <thickness> max_width <w> max_length <l>, create, \
                       then select region <name>, choose <module>.<Entry>, set <setting> = \
                       <value>, add. Refuses a folder that isn't empty, and refuses when the \
                       person's settings let only them make changes. Opens the new project, \
                       with its versions on.",
        annotations(read_only_hint = false, destructive_hint = false)
    )]
    async fn new_project(
        &self,
        Parameters(arguments): Parameters<NewArguments>,
        peer: Peer<RoleServer>,
    ) -> CallToolResult {
        finish(self.make_project(arguments, &peer).await)
    }
}

impl StaveServer {
    async fn open_folder(
        &self,
        given: String,
        start_versions: bool,
    ) -> Result<CallToolResult, McpError> {
        let folder = project_folder(&given)?;
        let (summary, text) = blocking(move || {
            // A folder inside a bigger repository is refused before
            // anything else: its versions could never be kept.
            let mut on = agent_line::versions_on(&folder)?;
            let opened = open(&folder)?;
            if !on && start_versions {
                agent_line::start(&folder, &agent_line::person(), FIRST_VERSION)?;
                on = true;
            }
            let mut summary = Summary::of(&folder, &opened);
            summary.versions = Versions::read(&folder, on)?;
            let text = summary.text();
            Ok((summary, text))
        })
        .await?;
        self.set_open_project(OpenProject {
            folder: summary.folder.clone(),
        })
        .await;
        Ok(answer(text, json!(summary)))
    }

    async fn make_project(
        &self,
        arguments: NewArguments,
        peer: &Peer<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let folder = new_folder(&arguments.folder)?;
        // Making a project counts as applying a change: the person's own
        // default decides, as the new folder has no settings of its own.
        match merge_policy(&folder, &self.options().config_dir) {
            MergePolicy::Off => return Err(McpError::NewProjectNotAllowed),
            MergePolicy::Ask | MergePolicy::Auto => {}
        }
        let steps = parse_script(&arguments.steps)?;
        let actor = agent_line::actor(&self.agent(peer).await);
        let (summary, transcript) = blocking(move || {
            let made = first_missing(&folder);
            std::fs::create_dir_all(&folder).map_err(|source| McpError::Io {
                path: folder.clone(),
                source,
            })?;
            let mut transcript = Vec::new();
            let fine = run(&folder, &steps, true, &mut transcript);
            let transcript = String::from_utf8_lossy(&transcript).into_owned();
            match fine {
                Ok(true) => {}
                Ok(false) => {
                    empty_again(&folder, made.as_deref());
                    return Err(McpError::StepFailed(transcript));
                }
                Err(error) => {
                    empty_again(&folder, made.as_deref());
                    return Err(error.into());
                }
            }
            let opened = folder
                .canonicalize()
                .map_err(|source| McpError::Io {
                    path: folder.clone(),
                    source,
                })
                .and_then(|real| open(&real).map(|opened| (real, opened)));
            let opened = opened.and_then(|(real, opened)| {
                agent_line::start(&real, &actor, NEW_PROJECT_VERSION)?;
                let mut summary = Summary::of(&real, &opened);
                summary.versions = Versions::read(&real, true)?;
                Ok(summary)
            });
            match opened {
                Ok(summary) => Ok((summary, transcript)),
                Err(McpError::NotAProject(_)) => {
                    empty_again(&folder, made.as_deref());
                    Err(McpError::NeverCreated)
                }
                Err(error) => {
                    empty_again(&folder, made.as_deref());
                    Err(error)
                }
            }
        })
        .await?;
        self.set_open_project(OpenProject {
            folder: summary.folder.clone(),
        })
        .await;
        let text = format!(
            "Made a new project in {}.\n\n{transcript}\n{}",
            summary.folder.display(),
            summary.text()
        );
        let mut structured = json!(summary);
        if let Some(fields) = structured.as_object_mut() {
            fields.insert("transcript".to_owned(), json!(transcript));
        }
        Ok(answer(text, structured))
    }
}

/// What `open_project` says about a project.
#[derive(Debug, Clone, Serialize)]
struct Summary {
    /// The project's title, or the folder's name when it has none yet.
    title: String,
    /// The folder.
    folder: PathBuf,
    /// Whether the design could be placed.
    placed: bool,
    /// How many parts.
    parts: usize,
    /// How many pieces of hardware.
    hardware: usize,
    /// How many errors.
    errors: usize,
    /// How many warnings.
    warnings: usize,
    /// Each problem on one line: `error[code]: message (file:line:column)`.
    problems: Vec<String>,
    /// Whether its versions are on, and how many there are.
    #[serde(flatten)]
    versions: Versions,
}

impl Summary {
    fn of(folder: &Path, opened: &Opened) -> Self {
        let warnings = opened
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.severity == stave_lang::Severity::Warning)
            .count();
        let problems = stave_app::view::problems(opened, folder)
            .into_iter()
            .map(|problem| {
                let mut line = format!(
                    "{}[{}]: {}",
                    problem.severity.word(),
                    problem.code,
                    problem.message
                );
                if let Some(location) = problem.location {
                    let _ = write!(line, " ({location})");
                }
                line
            })
            .collect();
        let design = opened.design.as_ref();
        Self {
            title: design
                .map(|design| design.project.title.clone())
                .filter(|title| !title.is_empty())
                .unwrap_or_else(|| folder_name(folder)),
            folder: folder.to_path_buf(),
            placed: design.is_some(),
            parts: design.map_or(0, |design| design.parts().len()),
            hardware: design.map_or(0, |design| design.hardware().len()),
            errors: opened.error_count(),
            warnings,
            problems,
            versions: Versions::OFF,
        }
    }

    fn text(&self) -> String {
        let mut text = format!("Opened \"{}\" in {}.\n", self.title, self.folder.display());
        let checks = format!(
            "{} and {}",
            count_words(self.errors, "error", "errors"),
            count_words(self.warnings, "warning", "warnings")
        );
        if self.placed {
            let _ = writeln!(
                text,
                "It has {} and {}, and checks with {checks}.",
                count_words(self.parts, "part", "parts"),
                count_words(self.hardware, "piece of hardware", "pieces of hardware"),
            );
        } else {
            let _ = writeln!(text, "It can't be placed yet: it checks with {checks}.");
        }
        if !self.problems.is_empty() {
            text.push_str("Problems:\n");
            for (number, problem) in self.problems.iter().enumerate() {
                let _ = writeln!(text, "  {}. {problem}", number.saturating_add(1));
            }
        }
        text.push_str(
            "Call check for the problems in full with their fixes, and describe for the parts.",
        );
        text.push('\n');
        match self.versions.versions {
            VersionsState::On => {
                let at_least = if self.versions.revisions_capped {
                    "at least "
                } else {
                    ""
                };
                let _ = write!(
                    text,
                    "Versions are on: {at_least}{} and {} waiting.",
                    count_words(self.versions.revisions, "version", "versions"),
                    count_words(self.versions.suggestions, "suggestion", "suggestions"),
                );
            }
            VersionsState::Off => text.push_str(&McpError::VersionsOff.to_string()),
        }
        text
    }
}

/// Whether a project's versions are on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
enum VersionsState {
    /// Off: changes can be tried, not suggested.
    Off,
    /// On.
    On,
}

/// What `open_project` says about a project's versions.
#[derive(Debug, Clone, Serialize)]
struct Versions {
    /// On or off.
    versions: VersionsState,
    /// How many versions main has, counted up to [`VERSIONS_COUNTED`].
    revisions: usize,
    /// Whether main has more versions than [`VERSIONS_COUNTED`], so
    /// `revisions` is only a lower bound.
    revisions_capped: bool,
    /// How many suggestions are waiting.
    suggestions: usize,
}

/// The most versions `open_project` counts.
const VERSIONS_COUNTED: usize = 1000;

impl Versions {
    const OFF: Self = Self {
        versions: VersionsState::Off,
        revisions: 0,
        revisions_capped: false,
        suggestions: 0,
    };

    /// The versions of the project in `folder`, when `on`.
    fn read(folder: &Path, on: bool) -> Result<Self, McpError> {
        if !on {
            return Ok(Self::OFF);
        }
        let history = agent_line::history(folder)?;
        // Counting one past the cap tells whether there are more.
        let counted = history.versions(VERSIONS_COUNTED.saturating_add(1))?.len();
        Ok(Self {
            versions: VersionsState::On,
            revisions: counted.min(VERSIONS_COUNTED),
            revisions_capped: counted > VERSIONS_COUNTED,
            suggestions: history.suggestions()?.len(),
        })
    }
}

/// Opens the project in `folder`.
pub(crate) fn open(folder: &Path) -> Result<Opened, McpError> {
    stave_app::open(folder).map_err(|error| load_error(folder, error))
}

/// The error for a project that can't be loaded.
pub(crate) fn load_error(folder: &Path, error: stave_app::LoadError) -> McpError {
    match error {
        stave_app::LoadError::NoStaveFiles { .. } => McpError::NotAProject(folder.to_path_buf()),
        stave_app::LoadError::NotAFolder { .. } => McpError::NotAFolder(folder.to_path_buf()),
        error @ (stave_app::LoadError::Unreadable { .. }
        | stave_app::LoadError::UnreadableFile { .. }
        | stave_app::LoadError::Sources(_)) => McpError::Load {
            folder: folder.to_path_buf(),
            source: error,
        },
    }
}

/// `given` as a path, when it is a full path without `..` steps.
fn plain_full_path(given: &str) -> Result<&Path, McpError> {
    let path = Path::new(given);
    if !path.is_absolute() {
        return Err(McpError::NotAbsolute(given.to_owned()));
    }
    if path
        .components()
        .any(|component| component == std::path::Component::ParentDir)
    {
        return Err(McpError::ParentStep(given.to_owned()));
    }
    Ok(path)
}

/// The folder `given` names, as a full path with every link followed:
/// the project is the folder the path really leads to, and everything
/// after stays inside that folder.
fn project_folder(given: &str) -> Result<PathBuf, McpError> {
    let path = plain_full_path(given)?;
    match path.canonicalize() {
        Ok(folder) if folder.is_dir() => Ok(folder),
        Ok(_) | Err(_) => Err(McpError::NotAFolder(path.to_path_buf())),
    }
}

/// The folder a new project goes into: a full path to an empty folder,
/// or to one that doesn't exist yet (its parent folders are made).
fn new_folder(given: &str) -> Result<PathBuf, McpError> {
    let path = plain_full_path(given)?;
    let Ok(metadata) = std::fs::symlink_metadata(path) else {
        return Ok(path.to_path_buf());
    };
    if metadata.is_symlink() {
        return Err(McpError::Link(path.to_path_buf()));
    }
    if !metadata.is_dir() {
        // A file is in the way.
        return Err(McpError::FolderNotEmpty(path.to_path_buf()));
    }
    let mut entries = std::fs::read_dir(path).map_err(|source| McpError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    if entries.next().is_some() {
        return Err(McpError::FolderNotEmpty(path.to_path_buf()));
    }
    Ok(path.to_path_buf())
}

/// The outermost folder on the way to `folder` (`folder` itself
/// included) that doesn't exist yet, so a failed `new_project` can remove
/// every folder it made. `None` when `folder` already exists.
fn first_missing(folder: &Path) -> Option<PathBuf> {
    folder
        .ancestors()
        .take_while(|ancestor| std::fs::symlink_metadata(ancestor).is_err())
        .last()
        .map(Path::to_path_buf)
}

/// Puts a folder that was empty (or not there) before a failed
/// `new_project` back as it was: `made` is the outermost folder
/// `new_project` made (see [`first_missing`]) and is removed whole;
/// without it `folder` already existed and is emptied. Best effort: what
/// can't be removed stays.
fn empty_again(folder: &Path, made: Option<&Path>) {
    if let Some(made) = made {
        let _ = std::fs::remove_dir_all(made);
        return;
    }
    let Ok(entries) = std::fs::read_dir(folder) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let _ = match entry.file_type() {
            Ok(kind) if kind.is_dir() => std::fs::remove_dir_all(&path),
            Ok(_) | Err(_) => std::fs::remove_file(&path),
        };
    }
}

fn folder_name(folder: &Path) -> String {
    folder.file_name().map_or_else(
        || folder.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    )
}

/// The project folders the window opened lately, newest first, that are
/// still there: read from the desktop's recent list (`recently-used.xbel`,
/// the desktop bookmark spec), where the window records each project it
/// opens under the application name "Stave". A missing or unreadable list
/// is an empty one.
pub fn recent_projects(list: &Path) -> Vec<RecentProject> {
    let Ok(text) = std::fs::read_to_string(list) else {
        return Vec::new();
    };
    let Ok(document) = roxmltree::Document::parse(&text) else {
        return Vec::new();
    };
    let mut found: Vec<(&str, PathBuf)> = document
        .descendants()
        .filter(|node| node.has_tag_name("bookmark"))
        .filter(|bookmark| {
            bookmark.descendants().any(|node| {
                node.has_tag_name((BOOKMARK_NAMESPACE, "application"))
                    && node.attribute("name") == Some(RECENT_APPLICATION)
            })
        })
        .filter_map(|bookmark| {
            let folder = file_uri_path(bookmark.attribute("href")?)?;
            Some((bookmark.attribute("modified").unwrap_or(""), folder))
        })
        .filter(|(_, folder)| folder.is_dir())
        .collect();
    // Newest first; the times are ISO 8601 in UTC, so they sort as text.
    found.sort_by(|a, b| b.0.cmp(a.0).then_with(|| a.1.cmp(&b.1)));
    let mut projects: Vec<RecentProject> = Vec::new();
    for (_, folder) in found {
        if projects.iter().any(|project| project.folder == folder) {
            continue;
        }
        projects.push(RecentProject {
            name: folder_name(&folder),
            folder,
        });
        if projects.len() == RECENT_LIMIT {
            break;
        }
    }
    projects
}

/// The path of a `file://` URI, percent-decoded; `None` for any other
/// URI or a broken escape.
fn file_uri_path(uri: &str) -> Option<PathBuf> {
    use std::os::unix::ffi::OsStringExt as _;
    let rest = uri.strip_prefix("file://")?;
    // `file://host/path`: only the local host is ours.
    let path = rest.strip_prefix("localhost").unwrap_or(rest);
    if !path.starts_with('/') {
        return None;
    }
    let mut bytes = Vec::with_capacity(path.len());
    let mut chars = path.bytes();
    while let Some(byte) = chars.next() {
        if byte == b'%' {
            let high = hex_digit(chars.next()?)?;
            let low = hex_digit(chars.next()?)?;
            bytes.push(high << 4 | low);
        } else {
            bytes.push(byte);
        }
    }
    Some(PathBuf::from(std::ffi::OsString::from_vec(bytes)))
}

fn hex_digit(byte: u8) -> Option<u8> {
    char::from(byte)
        .to_digit(16)
        .and_then(|digit| u8::try_from(digit).ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_uris_are_decoded() {
        assert_eq!(
            file_uri_path("file:///home/me/My%20Chest"),
            Some(PathBuf::from("/home/me/My Chest"))
        );
        assert_eq!(
            file_uri_path("file://localhost/tmp/a"),
            Some(PathBuf::from("/tmp/a"))
        );
        assert_eq!(file_uri_path("https://example.com/a"), None);
        assert_eq!(file_uri_path("file:///bad%2"), None);
        assert_eq!(file_uri_path("file:///bad%zz"), None);
    }
}
