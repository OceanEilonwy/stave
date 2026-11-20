//! Conflicts: `list_conflicts`, `try_resolution`, `resolve_conflict`.
//!
//! A clash sits either in the project's own files (the window caught up
//! with a change someone else applied and the two changed the same lines)
//! or in a suggestion (its line was moved onto main and its text clashed).
//! Each tool works on one of the two: the project's files by default, a
//! suggestion when its id is given.
//!
//! ```text
//! list_conflicts   ─▶ each conflict: sides, base, origin, change, errors
//! try_resolution   ─▶ stave_app::conflicts::try_resolution (writes nothing)
//! resolve_conflict ─▶ project: editing::write_all + History::save as the agent
//!                     suggestion: History::write_suggestion_files
//! ```
//!
//! Settling a clash in the project's own files moves main, so the result
//! must check with no errors (with other clashes left, add no errors to
//! any side of them) and the merge policy must not be `off`.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::CallToolResult;
use rmcp::{Peer, RoleServer, tool, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};
use stave_app::Opened;
use stave_app::conflicts::{self, SideChange};
use stave_app::editing::{CommitError, FileEdit, WriteEdits as _, write_all};
use stave_app::scratch::DiskWriter;
use stave_history::{
    Actor, ActorKind, History, MergePolicy, ProjectFiles, RelativePath, Suggestion, SuggestionId,
    Version, VersionsStatus, merge_policy,
};
use stave_lang::conflict::{Conflict, Resolution, find_conflicts};
use stave_lang::patch::TextEdit;
use stave_lang::render_diagnostics;
use stave_modules::{DiskFiles, FileSource, MemoryFiles};

use super::looking::{count_words, problem_json};
use super::projects::load_error;
use super::{answer, blocking, finish};
use crate::{Agent, McpError, OpenProject, StaveServer, project_path};

/// `list_conflicts`'s arguments.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListArguments {
    /// A suggestion's id, to list the clashes in that suggestion; leave
    /// it out for the clashes in the project's own files.
    #[serde(default)]
    pub suggestion: Option<String>,
}

/// How to settle one clash.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ResolutionArgument {
    /// Keep this side, by its number in list_conflicts (from 1), and drop
    /// the others: `{"keep": 2}`.
    Keep(usize),
    /// Keep every side, one after the other: `"keep_both"`.
    KeepBoth,
    /// Write this text in place of the whole clash, marker lines
    /// included: `{"text": "param box_length = 320mm"}`.
    Text(String),
}

/// `try_resolution`'s and `resolve_conflict`'s arguments.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ResolveArguments {
    /// The file, relative to the project folder, as list_conflicts names
    /// it: `box.stave`.
    pub file: String,
    /// The clash's number in that file, from 1, as list_conflicts numbers
    /// it.
    pub conflict: usize,
    /// How to settle it: `{"keep": 1}`, `"keep_both"` or
    /// `{"text": "..."}`.
    pub resolution: ResolutionArgument,
    /// A suggestion's id, when the clash is in that suggestion; leave it
    /// out for a clash in the project's own files.
    #[serde(default)]
    pub suggestion: Option<String>,
}

#[tool_router(router = conflicts_router, vis = "pub(crate)")]
impl StaveServer {
    /// Every clash, described.
    #[tool(
        description = "List the clashes (conflict markers) in the open project's own files, or \
                       in one suggestion when its id is given. For each: the file, its number \
                       in the file and its lines, the text both sides changed from (the base), \
                       and each side's text, where it came from (the person using Stave, an AI \
                       app's suggestion, or outside Stave), what it changes from the base as \
                       - and + lines, and how many errors the project has with that side \
                       chosen. Settle one with try_resolution, then resolve_conflict.",
        annotations(read_only_hint = true)
    )]
    async fn list_conflicts(
        &self,
        Parameters(arguments): Parameters<ListArguments>,
    ) -> CallToolResult {
        finish(self.conflicts_listed(arguments).await)
    }

    /// A resolution, tried.
    #[tool(
        description = "Try settling one clash on a scratch copy: keep one side ({\"keep\": n}, \
                       sides counted from 1), keep both (\"keep_both\") or write new text in \
                       its place ({\"text\": \"...\"}). Says how many errors the project would \
                       then have, with the problems, and whether clashes are left. Writes \
                       nothing.",
        annotations(read_only_hint = true)
    )]
    async fn try_resolution(
        &self,
        Parameters(arguments): Parameters<ResolveArguments>,
    ) -> CallToolResult {
        finish(self.resolution_tried(arguments).await)
    }

    /// A resolution, written.
    #[tool(
        description = "Settle one clash and write the result, as try_resolution tries it. In \
                       the project's own files the result must check with no errors (with \
                       other clashes left: it must add no errors to any side of them); it is \
                       written to the file, only if the file hasn't changed since it was read, \
                       and recorded as a version made by this AI app. A Stave window holding \
                       the file with unsaved changes keeps its own text. In a suggestion it is \
                       written to the suggestion, which can be applied once no clash is left.",
        annotations(read_only_hint = false, destructive_hint = true)
    )]
    async fn resolve_conflict(
        &self,
        peer: Peer<RoleServer>,
        Parameters(arguments): Parameters<ResolveArguments>,
    ) -> CallToolResult {
        let agent = self.agent(&peer).await;
        finish(self.resolution_written(arguments, agent).await)
    }
}

impl StaveServer {
    async fn conflicts_listed(&self, arguments: ListArguments) -> Result<CallToolResult, McpError> {
        let project = self.require_project().await?;
        blocking(move || list(&project, arguments.suggestion.as_deref())).await
    }

    async fn resolution_tried(
        &self,
        arguments: ResolveArguments,
    ) -> Result<CallToolResult, McpError> {
        let project = self.require_project().await?;
        blocking(move || {
            let place = Place::read(&project, arguments.suggestion.as_deref())?;
            let target = Target::find(&place, &project, &arguments)?;
            let tried = target.try_on(&place)?;
            let mut text = format!(
                "Settling clash {} in {} that way: {}.\n",
                arguments.conflict,
                target.shown,
                verdict(&tried.opened, &tried.text)
            );
            text.push_str(&problems_text(&tried.opened));
            text.push_str("Nothing was written.");
            Ok(answer(
                text,
                tried_json(&place.folder, &tried.opened, &tried.text, &target.shown),
            ))
        })
        .await
    }

    async fn resolution_written(
        &self,
        arguments: ResolveArguments,
        agent: Agent,
    ) -> Result<CallToolResult, McpError> {
        let project = self.require_project().await?;
        let config_dir = self.options().config_dir.clone();
        blocking(move || {
            let actor = Actor {
                kind: ActorKind::Agent,
                name: agent.name,
                id: agent.id,
            };
            let place = Place::read(&project, arguments.suggestion.as_deref())?;
            let target = Target::find(&place, &project, &arguments)?;
            match &place.suggestion {
                Some(suggestion) => {
                    write_to_suggestion(&place, suggestion, &target, arguments.conflict)
                }
                None => write_to_project(&place, &target, &actor, &config_dir, arguments.conflict),
            }
        })
        .await
    }
}

/// The files a tool works on: the project's own, or a suggestion's.
struct Place {
    /// The project folder.
    folder: PathBuf,
    /// The `.stave` files, by path relative to the folder (`/`-separated),
    /// with their text.
    texts: BTreeMap<String, String>,
    /// The suggestion, when the files are a suggestion's, with all of its
    /// files (for writing back) and the same `.stave` files in memory at
    /// their full paths (for checking).
    suggestion: Option<SuggestionPlace>,
    /// The project's versions, when they are on.
    history: Option<History>,
}

/// A suggestion's files.
struct SuggestionPlace {
    suggestion: Suggestion,
    files: ProjectFiles,
    memory: MemoryFiles,
}

impl Place {
    /// The files of the project, or of `suggestion` when given.
    fn read(project: &OpenProject, suggestion: Option<&str>) -> Result<Self, McpError> {
        let folder = project.folder.clone();
        let history = match History::status(&folder)? {
            VersionsStatus::On => Some(History::open(&folder)?),
            VersionsStatus::Off | VersionsStatus::Nested { .. } => None,
        };
        let Some(id) = suggestion else {
            let mut texts = BTreeMap::new();
            collect_stave_files(&folder, &folder, &mut texts)?;
            return Ok(Self {
                folder,
                texts,
                suggestion: None,
                history,
            });
        };
        let id = SuggestionId::new(id.trim());
        let history = history.ok_or(stave_history::HistoryError::Off)?;
        let suggestion = history
            .suggestions()?
            .into_iter()
            .find(|suggestion| suggestion.id == id)
            .ok_or_else(|| stave_history::HistoryError::UnknownSuggestion(id.clone()))?;
        let files = history.suggestion_files(&id)?;
        let mut texts = BTreeMap::new();
        let mut memory = MemoryFiles::new();
        for (path, bytes) in &files {
            if path.as_path().extension().is_none_or(|end| end != "stave") {
                continue;
            }
            let shown = slash_path(path.as_path());
            let text =
                String::from_utf8(bytes.to_vec()).map_err(|_| McpError::NotText(shown.clone()))?;
            memory.insert(folder.join(path.as_path()), text.clone());
            texts.insert(shown, text);
        }
        Ok(Self {
            folder,
            texts,
            suggestion: Some(SuggestionPlace {
                suggestion,
                files,
                memory,
            }),
            history: Some(history),
        })
    }

    /// Where the checker reads the files from.
    fn source(&self) -> &dyn FileSource {
        match &self.suggestion {
            Some(suggestion) => &suggestion.memory,
            None => &DiskFiles,
        }
    }
}

/// Adds every `.stave` file under `directory` (not in hidden folders, not
/// through links) to `texts`, by its path relative to `folder`.
fn collect_stave_files(
    folder: &Path,
    directory: &Path,
    texts: &mut BTreeMap<String, String>,
) -> Result<(), McpError> {
    let io = |source| McpError::Io {
        path: directory.to_owned(),
        source,
    };
    for entry in std::fs::read_dir(directory).map_err(io)? {
        let entry = entry.map_err(io)?;
        let path = entry.path();
        if entry.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        let kind = entry.file_type().map_err(io)?;
        if kind.is_dir() {
            collect_stave_files(folder, &path, texts)?;
        } else if kind.is_file() && path.extension().is_some_and(|end| end == "stave") {
            let relative = path.strip_prefix(folder).unwrap_or(&path);
            let shown = slash_path(relative);
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            texts.insert(shown, text);
        }
    }
    Ok(())
}

/// A relative path written with `/`.
fn slash_path(path: &Path) -> String {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

/// One clash a tool settles.
struct Target {
    /// The file as the tools name it.
    shown: String,
    /// The same, checked to be a path inside the project.
    relative: RelativePath,
    /// Its full path.
    path: PathBuf,
    /// The clash's index in the file, from 0.
    index: usize,
    /// How to settle it.
    resolution: Resolution,
    /// The file's text now.
    text: String,
}

impl Target {
    fn find(
        place: &Place,
        project: &OpenProject,
        arguments: &ResolveArguments,
    ) -> Result<Self, McpError> {
        // The path must stay inside the project, whichever files these are.
        let path = project_path(project, &arguments.file)?;
        let relative = RelativePath::new(Path::new(&arguments.file))
            .map_err(|_| McpError::OutsideProject(arguments.file.clone()))?;
        let shown = slash_path(relative.as_path());
        let text = place
            .texts
            .get(&shown)
            .cloned()
            .ok_or_else(|| McpError::NoConflicts(shown.clone()))?;
        let found = find_conflicts(&text);
        if found.is_empty() {
            return Err(McpError::NoConflicts(shown));
        }
        let no_such_clash = || McpError::NoSuchClash {
            file: shown.clone(),
            number: arguments.conflict,
            count: found.len(),
        };
        let index = arguments
            .conflict
            .checked_sub(1)
            .ok_or_else(no_such_clash)?;
        let conflict = found.get(index).ok_or_else(no_such_clash)?;
        let resolution = match &arguments.resolution {
            ResolutionArgument::Keep(number) => {
                let count = conflict.sides.len();
                let side = number.checked_sub(1).filter(|side| *side < count).ok_or(
                    McpError::NoSuchSide {
                        number: *number,
                        count,
                    },
                )?;
                Resolution::Keep(side)
            }
            ResolutionArgument::KeepBoth => Resolution::KeepBoth,
            ResolutionArgument::Text(text) => Resolution::Text(text.clone()),
        };
        Ok(Self {
            shown,
            relative,
            path,
            index,
            resolution,
            text,
        })
    }

    /// The clash settled on a scratch copy, and the project checked.
    fn try_on(&self, place: &Place) -> Result<conflicts::Tried, McpError> {
        conflicts::try_resolution(
            &place.folder,
            place.source(),
            &self.path,
            self.index,
            &self.resolution,
        )
        .map_err(|error| match error {
            conflicts::ConflictError::Load(error) => load_error(&place.folder, error),
            error @ (conflicts::ConflictError::Read { .. }
            | conflicts::ConflictError::Resolve(_)) => McpError::Conflict(error),
        })
    }
}

/// `list_conflicts`.
fn list(project: &OpenProject, suggestion: Option<&str>) -> Result<CallToolResult, McpError> {
    let place = Place::read(project, suggestion)?;
    let latest = place
        .history
        .as_ref()
        .and_then(|history| history.versions(1).ok())
        .and_then(|versions| versions.into_iter().next());
    let side_errors: Vec<usize> = conflicts::check_sides(&place.folder, place.source())
        .map_err(|error| load_error(&place.folder, error))?
        .iter()
        .map(Opened::error_count)
        .collect();
    let context = Origins {
        latest: latest.as_ref(),
        suggestion: place.suggestion.as_ref().map(|place| &place.suggestion),
    };
    let mut text = String::new();
    let mut listed = Vec::new();
    for (file, file_text) in &place.texts {
        for found in conflicts::conflicts(file_text) {
            let (words, json) = describe(file, file_text, &found, &context, &side_errors);
            text.push_str(&words);
            listed.push(json);
        }
    }
    let whose = match &place.suggestion {
        Some(suggestion) => format!("suggestion {}", suggestion.suggestion.id),
        None => "the project's own files".to_owned(),
    };
    if listed.is_empty() {
        text = format!("There are no clashes in {whose}.\n");
    } else {
        text.insert_str(
            0,
            &format!(
                "{} in {whose}:\n",
                count_words(listed.len(), "clash", "clashes")
            ),
        );
        text.push_str(
            "Settle one with try_resolution (keep a side by its number, keep_both, or new \
             text), then resolve_conflict.\n",
        );
    }
    // Suggestions that clash, when listing the project's own files.
    let mut clashing = Vec::new();
    if place.suggestion.is_none()
        && let Some(history) = &place.history
    {
        for suggestion in history.suggestions()? {
            if suggestion.conflicted.is_empty() {
                continue;
            }
            let files: Vec<String> = suggestion
                .conflicted
                .iter()
                .map(|path| slash_path(path.as_path()))
                .collect();
            let _ = writeln!(
                text,
                "Suggestion {} (\"{}\") has clashes in {}: call list_conflicts with \
                 suggestion: \"{}\" to see them.",
                suggestion.id,
                suggestion.title,
                files.join(", "),
                suggestion.id
            );
            clashing.push(json!({
                "id": suggestion.id.as_str(),
                "title": suggestion.title,
                "files": files,
            }));
        }
    }
    Ok(answer(
        text,
        json!({
            "suggestion": place.suggestion.as_ref().map(|place| place.suggestion.id.as_str()),
            "conflicts": listed,
            "suggestions_with_conflicts": clashing,
        }),
    ))
}

/// What tells where each side of a clash came from.
struct Origins<'a> {
    /// Main's latest version, when versions are on.
    latest: Option<&'a Version>,
    /// The suggestion, when the files are a suggestion's.
    suggestion: Option<&'a Suggestion>,
}

impl Origins<'_> {
    /// Where the side labelled `label` came from, in plain words. The
    /// labels are the ones Stave's versions write: `yours` (the window's
    /// files), `theirs` (main), and, when a suggestion is moved onto main,
    /// `… (rebased revision)` (the suggestion) and `… (rebase
    /// destination)` (main).
    fn of(&self, label: &str) -> String {
        if label == "yours" {
            return "the person using Stave (their work in the Stave window)".to_owned();
        }
        if label.ends_with("(rebased revision)")
            && let Some(suggestion) = self.suggestion
        {
            return format!(
                "{}'s suggestion \"{}\"",
                actor_words(&suggestion.agent),
                suggestion.title
            );
        }
        if label == "theirs" || label.ends_with("(rebase destination)") {
            return match self.latest {
                Some(version) => format!(
                    "the project as it is now; its latest version is \"{}\" by {}",
                    version.message,
                    actor_words(&version.actor)
                ),
                None => "the project as it is now".to_owned(),
            };
        }
        if label.is_empty() {
            "outside Stave".to_owned()
        } else {
            format!("outside Stave (marked '{label}')")
        }
    }
}

/// An actor in plain words.
fn actor_words(actor: &Actor) -> String {
    match actor.kind {
        ActorKind::User => {
            if actor.name.is_empty() {
                "the person using Stave".to_owned()
            } else {
                format!("{} (the person using Stave)", actor.name)
            }
        }
        ActorKind::Agent => format!("{} {}", actor.name, actor.id).trim().to_owned(),
    }
}

/// One clash in words and as JSON.
fn describe(
    file: &str,
    text: &str,
    found: &conflicts::FileConflict,
    origins: &Origins<'_>,
    side_errors: &[usize],
) -> (String, Value) {
    let number = found.index.saturating_add(1);
    let section = |range: &std::ops::Range<usize>| text.get(range.clone()).unwrap_or("");
    let mut words = format!(
        "{file}, clash {number} (lines {}-{}):\n",
        found.lines.start(),
        found.lines.end()
    );
    let base = found
        .conflict
        .base
        .as_ref()
        .map(|base| section(&base.range));
    match base {
        Some("") => words.push_str("  before: nothing (both sides added lines)\n"),
        Some(base) => {
            words.push_str("  before:\n");
            push_indented(&mut words, base, "    ");
        }
        None => words.push_str("  before: not recorded\n"),
    }
    let mut sides = Vec::new();
    for (index, side) in found.conflict.sides.iter().enumerate() {
        let origin = origins.of(&side.label);
        let errors = side_errors.get(index).copied();
        let change = side_change_text(text, &found.conflict, index);
        let checks = match errors {
            Some(0) => "the project checks with no errors with this side".to_owned(),
            Some(count) => format!(
                "the project has {} with this side",
                count_words(count, "error", "errors")
            ),
            None => "not checked".to_owned(),
        };
        // The plain origin only: the raw marker label (jj's revision
        // words for a suggestion) stays in the JSON, and `origin` already
        // quotes it for a clash made outside Stave.
        let _ = writeln!(
            words,
            "  side {}, from {origin}; {checks}:",
            index.saturating_add(1)
        );
        push_indented(&mut words, &change, "    ");
        sides.push(json!({
            "number": index.saturating_add(1),
            "label": side.label,
            "origin": origin,
            "text": section(&side.range),
            "change": change,
            "errors": errors,
        }));
    }
    // Only the resolutions, not the window's translated words for them,
    // so the same project gives the same JSON in every locale.
    let actions: Vec<Value> = found
        .actions
        .iter()
        .map(|action| resolution_json(&action.resolution))
        .collect();
    let json = json!({
        "file": file,
        "number": number,
        "lines": [found.lines.start(), found.lines.end()],
        "base": base,
        "sides": sides,
        "resolutions": actions,
    });
    (words, json)
}

/// A side's change from the base as text: `- `/`+ ` lines, or the whole
/// side when the clash doesn't record its base.
fn side_change_text(text: &str, conflict: &Conflict, side: usize) -> String {
    match conflicts::side_change(text, conflict, side) {
        SideChange::Against { lines } if lines.is_empty() => "(no change)\n".to_owned(),
        SideChange::Against { lines } => lines,
        SideChange::Whole { text } => text.lines().map(|line| format!("+ {line}\n")).collect(),
    }
}

/// A resolution as the tools take it.
fn resolution_json(resolution: &Resolution) -> Value {
    match resolution {
        Resolution::Keep(side) => json!({ "keep": side.saturating_add(1) }),
        Resolution::KeepBoth => json!("keep_both"),
        Resolution::Text(text) => json!({ "text": text }),
    }
}

/// Appends `text`'s lines to `out`, each after `indent`.
fn push_indented(out: &mut String, text: &str, indent: &str) {
    for line in text.lines() {
        out.push_str(indent);
        out.push_str(line);
        out.push('\n');
    }
}

/// What a tried resolution leaves, in a few words.
fn verdict(opened: &Opened, text: &str) -> String {
    let errors = opened.error_count();
    let left = find_conflicts(text).len();
    let mut words = if errors == 0 {
        "the project checks with no errors".to_owned()
    } else {
        format!(
            "the project would have {}",
            count_words(errors, "error", "errors")
        )
    };
    if left > 0 {
        let _ = write!(
            words,
            ", and the file still has {}",
            count_words(left, "clash", "clashes")
        );
    }
    words
}

/// The errors of `opened` as `stave check` prints them.
fn problems_text(opened: &Opened) -> String {
    let errors: Vec<_> = opened
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.is_error())
        .cloned()
        .collect();
    render_diagnostics(&opened.sources, &errors)
}

/// A tried resolution as JSON.
fn tried_json(folder: &Path, opened: &Opened, text: &str, file: &str) -> Value {
    let problems: Vec<Value> = opened
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.is_error())
        .map(|diagnostic| problem_json(&opened.sources, folder, diagnostic))
        .collect();
    json!({
        "file": file,
        "errors": opened.error_count(),
        "conflicts_left": find_conflicts(text).len(),
        "problems": problems,
        "text": text,
    })
}

/// `resolve_conflict` in a suggestion: the suggestion's files with the
/// clash settled, written to the suggestion.
fn write_to_suggestion(
    place: &Place,
    suggestion: &SuggestionPlace,
    target: &Target,
    number: usize,
) -> Result<CallToolResult, McpError> {
    let tried = target.try_on(place)?;
    let mut history = History::open(&place.folder)?;
    let mut files = suggestion.files.clone();
    files.insert(target.relative.clone(), tried.text.clone().into_bytes());
    history.write_suggestion_files(&suggestion.suggestion.id, &files)?;
    let mut text = format!(
        "Settled clash {number} in {} in suggestion {}: {}.\n",
        target.shown,
        suggestion.suggestion.id,
        verdict(&tried.opened, &tried.text)
    );
    text.push_str(&problems_text(&tried.opened));
    let left: usize = history
        .suggestions()?
        .into_iter()
        .find(|found| found.id == suggestion.suggestion.id)
        .map_or(0, |found| found.conflicted.len());
    if left == 0 {
        text.push_str("The suggestion has no clashes left.");
    } else {
        let _ = write!(
            text,
            "The suggestion still has clashes in {}.",
            count_words(left, "file", "files")
        );
    }
    let mut json = tried_json(&place.folder, &tried.opened, &tried.text, &target.shown);
    if let Value::Object(map) = &mut json {
        map.insert(
            "suggestion".to_owned(),
            json!(suggestion.suggestion.id.as_str()),
        );
        map.insert("files_with_conflicts_left".to_owned(), json!(left));
    }
    Ok(answer(text, json))
}

/// How many clashes the project's own files hold with `target`'s file
/// reading `text`.
fn clashes_left(place: &Place, target: &Target, text: &str) -> usize {
    place
        .texts
        .iter()
        .map(|(file, file_text)| {
            if *file == target.shown {
                find_conflicts(text).len()
            } else {
                find_conflicts(file_text).len()
            }
        })
        .sum()
}

/// Whether a tried resolution of the project's own files may be written:
/// `None` when it may, else why not.
///
/// With no clash left the project must check with no errors. With clashes
/// left, every clash still left is an `error[conflict]`, so the project
/// can't check clean until the last is settled; instead each side of the
/// clashes left (that side chosen in every one, as `list_conflicts`
/// checks them) must have no more errors with the resolution than it had
/// before it, so settling one clash never adds a problem to any way of
/// settling the rest.
fn refusal(
    place: &Place,
    target: &Target,
    tried: &conflicts::Tried,
    left: usize,
) -> Result<Option<McpError>, McpError> {
    if left == 0 {
        let errors = tried.opened.error_count();
        return Ok((errors > 0).then(|| McpError::NotClean {
            errors: count_words(errors, "error", "errors"),
            problems: problems_text(&tried.opened),
        }));
    }
    let load = |error| load_error(&place.folder, error);
    let before = conflicts::check_sides(&place.folder, &DiskFiles).map_err(load)?;
    let resolved = conflicts::Replaced::new(&DiskFiles, &target.path, &tried.text);
    let after = conflicts::check_sides(&place.folder, &resolved).map_err(load)?;
    for (side, opened) in after.iter().enumerate() {
        let had = before
            .get(side)
            .or(before.last())
            .map_or(0, Opened::error_count);
        let has = opened.error_count();
        if has > had {
            return Ok(Some(McpError::NotClean {
                errors: format!(
                    "{} with side {} chosen in the clashes left ({} before)",
                    count_words(has, "error", "errors"),
                    side.saturating_add(1),
                    had
                ),
                problems: problems_text(opened),
            }));
        }
    }
    Ok(None)
}

/// `resolve_conflict` in the project's own files: written to the file
/// through the one writer when the result checks clean (or, with clashes
/// left, adds no errors) and the policy allows, then recorded as a version
/// by the agent when versions are on.
fn write_to_project(
    place: &Place,
    target: &Target,
    actor: &Actor,
    config_dir: &Path,
    number: usize,
) -> Result<CallToolResult, McpError> {
    if merge_policy(&place.folder, config_dir) == MergePolicy::Off {
        return Err(McpError::PolicyOff);
    }
    let tried = target.try_on(place)?;
    let left = clashes_left(place, target, &tried.text);
    if let Some(refused) = refusal(place, target, &tried, left)? {
        return Err(refused);
    }
    // Through the one writer, only over the text the clash was settled
    // from: anything saved since (the window saving) is kept.
    let edit = FileEdit {
        path: target.path.clone(),
        created: false,
        before: target.text.clone(),
        edits: vec![TextEdit {
            range: 0..target.text.len(),
            text: tried.text.clone(),
        }],
        after: tried.text.clone(),
    };
    if DiskWriter.can_write(&edit).is_err() {
        return Err(McpError::ChangedMeanwhile(target.shown.clone()));
    }
    write_all(std::slice::from_ref(&edit), &mut DiskWriter).map_err(|error| match error {
        CommitError::Write { path, source } => McpError::Io { path, source },
        CommitError::NothingChosen | CommitError::DoesNotFit => {
            McpError::ChangedMeanwhile(target.shown.clone())
        }
    })?;
    let mut text = format!(
        "Settled clash {number} in {}: {}.\n",
        target.shown,
        verdict(&tried.opened, &tried.text)
    );
    if left > 0 {
        let _ = writeln!(
            text,
            "The project's own files still have {}: list them with list_conflicts.",
            count_words(left, "clash", "clashes")
        );
    }
    let mut not_recorded = None;
    let recorded = match History::status(&place.folder)? {
        VersionsStatus::On => {
            let message = format!("Settle the clash in {}", target.shown);
            match record(&place.folder, actor, &message) {
                Ok(revision) => {
                    let _ = writeln!(text, "Recorded as a version of the project, \"{message}\".");
                    Some(revision)
                }
                Err(error) => {
                    // The file is written already: say so, so the clash
                    // isn't settled again over the new text.
                    let _ = writeln!(
                        text,
                        "The file was written, but it couldn't be recorded as a version: \
                         {error}. Don't settle this clash again; the next save in Stave \
                         records it."
                    );
                    not_recorded = Some(error.to_string());
                    None
                }
            }
        }
        VersionsStatus::Off | VersionsStatus::Nested { .. } => {
            text.push_str("Versions are off for this project, so no version was recorded.\n");
            None
        }
    };
    let mut json = tried_json(&place.folder, &tried.opened, &tried.text, &target.shown);
    if let Value::Object(map) = &mut json {
        map.insert("version".to_owned(), json!(recorded));
        map.insert("conflicts_left_in_project".to_owned(), json!(left));
        if let Some(reason) = not_recorded {
            map.insert("not_recorded".to_owned(), json!(reason));
        }
    }
    Ok(answer(text, json))
}

/// Records the project folder as a version by `actor`.
fn record(folder: &Path, actor: &Actor, message: &str) -> Result<String, McpError> {
    let mut history = History::open(folder)?;
    let files = history.read_folder()?;
    let revision = history.save(&files, actor, message)?;
    Ok(revision.as_str().to_owned())
}
