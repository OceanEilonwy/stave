//! Revisions: `suggest`, `list_suggestions`, `list_versions`,
//! `apply_suggestion`, `discard_suggestion`, `undo`.
//!
//! An agent's changes go into its own line, never straight into the
//! project: `suggest` records a patch that fits and checks clean as a
//! suggestion (main is not touched). `apply_suggestion` is the merge: it re-tries the
//! suggestion's patches, in order, on main's latest files, and records the
//! result on main only when the design then checks with no errors; when a
//! patch no longer fits, the suggestion's line is moved onto main instead
//! and its files, merged as text, are checked the same way. Applying
//! follows the project's merge policy (`.stave/agents.toml`, then the
//! person's `agents.toml`): under `off` only the person applies, from the
//! window. The window, not the agent, brings the project folder up to
//! date with main.

use std::fmt::Write as _;

use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::CallToolResult;
use rmcp::{Peer, RoleServer, tool, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};
use stave_history::{
    Actor, ActorKind, History, HistoryError, MergePolicy, ProjectFiles, RevisionId, SuggestionId,
    UndoScope, merge_policy,
};

use super::looking::count_words;
use super::{answer, blocking, finish};
use crate::agent_line::{self, Replay};
use crate::{McpError, StaveServer};

/// How many versions `list_versions` lists when not told.
const VERSIONS_SHOWN: usize = 20;

/// The most versions `list_versions` lists.
const VERSIONS_MOST: usize = 200;

/// How many hex digits of a version's id are shown.
const SHORT_ID: usize = 12;

/// `suggest`'s arguments.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct SuggestArguments {
    /// One or more patches in the .stave patch grammar, as for try_patch.
    pub patch: String,
    /// What the suggestion does, in a few plain words: "Add a shoe shelf".
    pub title: String,
    /// The id of a suggestion to add this patch to, on top of its own
    /// patches; without it a new suggestion starts from the project's main
    /// line.
    #[serde(default)]
    pub onto: Option<String>,
}

/// `list_versions`' arguments.
#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct VersionsArguments {
    /// How many versions to list, newest first (20 when not given, at most
    /// 200).
    #[serde(default)]
    pub limit: Option<usize>,
}

/// The arguments of the tools that act on one suggestion.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct SuggestionArguments {
    /// The suggestion's id, from suggest or list_suggestions.
    pub id: String,
}

#[tool_router(router = revisions_router, vis = "pub(crate)")]
impl StaveServer {
    /// Records a patch that fits as a suggestion.
    #[tool(
        description = "Record one or more patches as a suggestion for the open project, once \
                       they fit: they are tried on the project's main line (or on top of the \
                       suggestion given as onto) and recorded only when they fit and the design \
                       then checks with no errors. The project \
                       itself doesn't change: the person sees the suggestion in Stave, and \
                       apply_suggestion puts it in. Needs the project's versions on.",
        annotations(read_only_hint = false, destructive_hint = false)
    )]
    async fn suggest(
        &self,
        Parameters(arguments): Parameters<SuggestArguments>,
        peer: Peer<RoleServer>,
    ) -> CallToolResult {
        finish(self.record_suggestion(arguments, &peer).await)
    }

    /// The suggestions waiting.
    #[tool(
        description = "List the open project's suggestions waiting to be applied or \
                       discarded, with who made each, what it does and any clashes.",
        annotations(read_only_hint = true)
    )]
    async fn list_suggestions(&self) -> CallToolResult {
        finish(self.suggestions().await)
    }

    /// The project's versions.
    #[tool(
        description = "List the open project's versions, newest first: who made each \
                       (the person or an AI app), what it did and when.",
        annotations(read_only_hint = true)
    )]
    async fn list_versions(
        &self,
        Parameters(arguments): Parameters<VersionsArguments>,
    ) -> CallToolResult {
        finish(self.versions(arguments.limit).await)
    }

    /// Applies a suggestion to the project.
    #[tool(
        description = "Put a suggestion into the open project: its patches are tried again on \
                       the project as it is now and recorded as a new version only when the \
                       design then checks with no errors. Refused when the project's settings \
                       let only the person apply suggestions. Undo takes it back.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false
        )
    )]
    async fn apply_suggestion(
        &self,
        Parameters(arguments): Parameters<SuggestionArguments>,
        peer: Peer<RoleServer>,
    ) -> CallToolResult {
        finish(self.apply(arguments.id, &peer).await)
    }

    /// Discards a suggestion.
    #[tool(
        description = "Discard one of the open project's suggestions. The project doesn't \
                       change; undo brings the suggestion back.",
        annotations(read_only_hint = false, destructive_hint = false)
    )]
    async fn discard_suggestion(
        &self,
        Parameters(arguments): Parameters<SuggestionArguments>,
        peer: Peer<RoleServer>,
    ) -> CallToolResult {
        finish(self.discard(arguments.id, &peer).await)
    }

    /// Undoes this agent's last change.
    #[tool(
        description = "Undo this AI app's own last change to the open project (a suggestion, \
                       an apply or a discard). Refused when the person or another app has \
                       since changed the project on top of it.",
        annotations(read_only_hint = false, destructive_hint = true)
    )]
    async fn undo(&self, peer: Peer<RoleServer>) -> CallToolResult {
        finish(self.undo_own(&peer).await)
    }
}

impl StaveServer {
    async fn record_suggestion(
        &self,
        arguments: SuggestArguments,
        peer: &Peer<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let project = self.require_project().await?;
        let actor = agent_line::actor(&self.agent(peer).await);
        blocking(move || {
            let folder = &project.folder;
            let mut history = agent_line::history(folder)?;
            let onto = arguments.onto.map(SuggestionId::new);
            let (base, files) = agent_line::line_files(&history, onto.as_ref())?;
            let after =
                match agent_line::replay(folder, &files, std::slice::from_ref(&arguments.patch))? {
                    Replay::Fits {
                        files, errors: 0, ..
                    } => files,
                    // apply_suggestion needs a design with no errors, so
                    // a suggestion that leaves any could never go in.
                    Replay::Fits { problems, .. } => {
                        return Err(McpError::SuggestionNotClean(problems));
                    }
                    Replay::Refused { title, problems } => {
                        return Err(McpError::SuggestionRefused(format!(
                            "{title}: refused\n{problems}"
                        )));
                    }
                };
            let changed: Vec<String> = after
                .iter()
                .filter(|(path, contents)| files.get(path) != Some(*contents))
                .map(|(path, _)| path.to_string())
                .collect();
            let title = arguments.title.trim();
            let id = history.suggest(
                &actor,
                onto.as_ref(),
                &base,
                &after,
                &arguments.patch,
                title,
            )?;
            let mut text = match &onto {
                Some(onto) => format!("Added \"{title}\" to suggestion {onto}."),
                None => format!("Suggested \"{title}\" as suggestion {id}."),
            };
            text.push_str(" It fits");
            if changed.is_empty() {
                text.push_str(" and changes no file.");
            } else {
                let _ = write!(text, " and changes {}.", changed.join(", "));
            }
            text.push_str(
                " The project itself is unchanged: the person sees the suggestion in Stave, and \
                 apply_suggestion puts it in.",
            );
            Ok(answer(
                text,
                json!({ "id": id.as_str(), "title": title, "changed": changed }),
            ))
        })
        .await
    }

    async fn suggestions(&self) -> Result<CallToolResult, McpError> {
        let project = self.require_project().await?;
        blocking(move || {
            let history = agent_line::history(&project.folder)?;
            let suggestions = history.suggestions()?;
            let mut text = String::new();
            if suggestions.is_empty() {
                text.push_str("No suggestions are waiting.");
            } else {
                text.push_str("Suggestions waiting:\n");
            }
            let mut listed = Vec::with_capacity(suggestions.len());
            for suggestion in &suggestions {
                let _ = write!(
                    text,
                    "  {}: \"{}\" by {}, {}",
                    suggestion.id,
                    suggestion.title,
                    who(&suggestion.agent),
                    count_words(suggestion.patches.len(), "patch", "patches"),
                );
                let conflicted: Vec<String> = suggestion
                    .conflicted
                    .iter()
                    .map(ToString::to_string)
                    .collect();
                if !conflicted.is_empty() {
                    let _ = write!(
                        text,
                        "; clashes with the project in {}, to resolve before it can be applied",
                        conflicted.join(", ")
                    );
                }
                text.push('\n');
                listed.push(json!({
                    "id": suggestion.id.as_str(),
                    "title": suggestion.title,
                    "by": actor_json(&suggestion.agent),
                    "patches": suggestion.patches,
                    "base": suggestion.base.as_str(),
                    "conflicted": conflicted,
                }));
            }
            Ok(answer(
                text.trim_end().to_owned(),
                json!({ "suggestions": listed }),
            ))
        })
        .await
    }

    async fn versions(&self, limit: Option<usize>) -> Result<CallToolResult, McpError> {
        let project = self.require_project().await?;
        let limit = limit.unwrap_or(VERSIONS_SHOWN).clamp(1, VERSIONS_MOST);
        blocking(move || {
            let history = agent_line::history(&project.folder)?;
            let versions = history.versions(limit)?;
            let mut text = if versions.is_empty() {
                "The project has no versions yet.".to_owned()
            } else {
                "Versions, newest first:\n".to_owned()
            };
            let mut listed = Vec::with_capacity(versions.len());
            for version in &versions {
                let short = version
                    .id
                    .as_str()
                    .get(..SHORT_ID)
                    .unwrap_or(version.id.as_str());
                let _ = writeln!(
                    text,
                    "  {}  {short}  {}: {}",
                    version.time,
                    who(&version.actor),
                    version.message
                );
                listed.push(json!({
                    "id": version.id.as_str(),
                    "by": actor_json(&version.actor),
                    "message": version.message,
                    "time": version.time,
                }));
            }
            Ok(answer(
                text.trim_end().to_owned(),
                json!({ "versions": listed }),
            ))
        })
        .await
    }

    async fn apply(&self, id: String, peer: &Peer<RoleServer>) -> Result<CallToolResult, McpError> {
        let project = self.require_project().await?;
        let actor = agent_line::actor(&self.agent(peer).await);
        let config_dir = self.options().config_dir.clone();
        blocking(move || {
            let folder = &project.folder;
            let mut history = agent_line::history(folder)?;
            match merge_policy(folder, &config_dir) {
                MergePolicy::Off => return Err(McpError::ApplyNotAllowed),
                MergePolicy::Ask | MergePolicy::Auto => {}
            }
            let id = SuggestionId::new(id);
            // Main may move while the result is checked; it is then checked
            // again on main as it is now, a few times at most.
            let mut attempts = APPLY_ATTEMPTS;
            let (suggestion, main) = loop {
                attempts = attempts.saturating_sub(1);
                match apply_once(folder, &mut history, &id, &actor) {
                    Err(McpError::History(HistoryError::MainMovedSince { .. })) if attempts > 0 => {
                    }
                    result => break result?,
                }
            };
            let short = main.as_str().get(..SHORT_ID).unwrap_or(main.as_str());
            let text = format!(
                "Applied \"{}\": it is now in the project as version {short}. Stave brings the \
                 open project up to date; undo takes it back.",
                suggestion.title
            );
            Ok(answer(
                text,
                json!({
                    "applied": id.as_str(),
                    "title": suggestion.title,
                    "main": main.as_str(),
                }),
            ))
        })
        .await
    }
    async fn discard(
        &self,
        id: String,
        peer: &Peer<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let project = self.require_project().await?;
        let actor = agent_line::actor(&self.agent(peer).await);
        blocking(move || {
            let mut history = agent_line::history(&project.folder)?;
            let id = SuggestionId::new(id);
            history.discard(&id, &actor)?;
            Ok(answer(
                format!(
                    "Discarded suggestion {id}. The project is unchanged; undo brings it back."
                ),
                json!({ "discarded": id.as_str() }),
            ))
        })
        .await
    }

    async fn undo_own(&self, peer: &Peer<RoleServer>) -> Result<CallToolResult, McpError> {
        let project = self.require_project().await?;
        let actor = agent_line::actor(&self.agent(peer).await);
        blocking(move || {
            let mut history = agent_line::history(&project.folder)?;
            let undone = history.undo(&actor, UndoScope::Own)?;
            Ok(answer(
                format!("Undid this app's last change: {undone}."),
                json!({ "undone": undone }),
            ))
        })
        .await
    }
}

/// Who made a change, in words: the person's name, or the app's.
/// A suggestion's base and latest files, read after it was moved onto
/// main.
///
/// The base is the revision the suggestion was moved onto, not main as it
/// is now: main may have moved since the rebase let go of the lock, and
/// [`History::apply`] merges the suggestion onto a later main only when it
/// is given the older base (given main itself, it lays the files over main
/// and undoes whatever landed in between). The base is read before the
/// files, so a second rebase between the two reads can only leave the
/// files newer than the base, which merges cleanly, never older.
/// How many times applying reads main again when it moved meanwhile.
const APPLY_ATTEMPTS: u32 = 3;

/// One try at applying suggestion `id`: its patches replayed and checked
/// on main as it is now, then recorded on that same main.
fn apply_once(
    folder: &std::path::Path,
    history: &mut History,
    id: &SuggestionId,
    actor: &Actor,
) -> Result<(stave_history::Suggestion, RevisionId), McpError> {
    let suggestion = history
        .suggestions()?
        .into_iter()
        .find(|suggestion| &suggestion.id == id)
        .ok_or_else(|| HistoryError::UnknownSuggestion(id.clone()))?;
    if !suggestion.conflicted.is_empty() {
        return Err(HistoryError::SuggestionConflicted {
            id: id.clone(),
            files: suggestion.conflicted,
        }
        .into());
    }

    // Re-try the patches on main as it is now.
    let base = history.main()?;
    let main_files = history.files_at(&base)?;
    let (base, merged) = match agent_line::replay(folder, &main_files, &suggestion.patches)? {
        Replay::Fits {
            files, errors: 0, ..
        } => (base, files),
        Replay::Fits { problems, .. } => return Err(McpError::NotClean(problems)),
        Replay::Refused { title, problems } => {
            // A patch no longer fits: move the line onto main and
            // check its files, merged as text, instead.
            let clashes = history.rebase_suggestion(id, actor)?;
            if !clashes.is_empty() {
                return Err(McpError::Clashes {
                    files: clashes.iter().map(ToString::to_string).collect(),
                });
            }
            let (base, files) = rebased_line(history, id)?;
            match agent_line::replay(folder, &files, &[])? {
                Replay::Fits {
                    files, errors: 0, ..
                } => (base, files),
                Replay::Fits {
                    problems: merged, ..
                } => {
                    return Err(McpError::NotClean(format!(
                        "{title} no longer fits:\n{problems}\nThe suggestion's files, \
                             moved onto the project, check with:\n{merged}"
                    )));
                }
                Replay::Refused { problems, .. } => {
                    return Err(McpError::NotClean(problems));
                }
            }
        }
    };
    let main = history.apply(id, actor, &base, &merged)?;
    Ok((suggestion, main))
}

fn rebased_line(
    history: &History,
    id: &SuggestionId,
) -> Result<(RevisionId, ProjectFiles), McpError> {
    let base = history
        .suggestions()?
        .into_iter()
        .find(|suggestion| &suggestion.id == id)
        .ok_or_else(|| HistoryError::UnknownSuggestion(id.clone()))?;
    let files = history.files_at(&base.tip)?;
    Ok((base.base, files))
}

fn who(actor: &Actor) -> String {
    match actor.kind {
        ActorKind::User => actor.name.clone(),
        ActorKind::Agent => format!("{} (an AI app)", actor.name),
    }
}

fn actor_json(actor: &Actor) -> Value {
    let kind = match actor.kind {
        ActorKind::User => "person",
        ActorKind::Agent => "agent",
    };
    json!({ "kind": kind, "name": actor.name, "id": actor.id })
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use stave_history::{Actor, ActorKind, History, ProjectFiles, RelativePath};

    use super::rebased_line;

    fn person() -> Actor {
        Actor {
            kind: ActorKind::User,
            name: "Ann".to_owned(),
            id: "window".to_owned(),
        }
    }

    fn agent() -> Actor {
        Actor {
            kind: ActorKind::Agent,
            name: "Helper".to_owned(),
            id: "helper-1".to_owned(),
        }
    }

    fn relative(path: &str) -> RelativePath {
        RelativePath::new(Path::new(path)).unwrap()
    }

    fn text(files: &ProjectFiles, path: &str) -> String {
        String::from_utf8(files.get(&relative(path)).unwrap().to_vec()).unwrap()
    }

    fn scratch_folder(name: &str) -> PathBuf {
        let folder =
            std::env::temp_dir().join(format!("stave-mcp-unit-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).unwrap();
        folder.canonicalize().unwrap()
    }

    /// Saves main's files with `path` replaced by `contents`, through a
    /// handle of its own, as the window or another agent would.
    fn main_changes(folder: &Path, path: &str, contents: &str) {
        let mut history = History::open(folder).unwrap();
        let mut files = history.files_at(&history.main().unwrap()).unwrap();
        files.insert(relative(path), contents.as_bytes().to_vec());
        history
            .save(&files, &person(), "Someone else's change")
            .unwrap();
    }

    /// Main moves between the rebase and the apply: the files checked on
    /// the rebased line are not recorded on a main they were never checked
    /// on, so the change that landed is kept and the caller tries again.
    #[test]
    fn apply_after_a_rebase_onto_a_main_that_moved_is_refused() {
        let folder = scratch_folder("rebased_line");
        for name in ["a", "b", "c"] {
            std::fs::write(folder.join(format!("{name}.stave")), format!("{name}\n")).unwrap();
        }
        let mut history = History::start(&folder).unwrap();
        let first = history.read_folder().unwrap();
        history.save(&first, &person(), "First version").unwrap();

        let mut suggested = first.clone();
        suggested.insert(relative("a.stave"), b"a, suggested\n".to_vec());
        let id = history
            .suggest(
                &agent(),
                None,
                &history.main().unwrap(),
                &suggested,
                "patch",
                "Change a",
            )
            .unwrap();

        main_changes(&folder, "b.stave", "b, before the rebase\n");
        assert!(history.rebase_suggestion(&id, &agent()).unwrap().is_empty());
        let rebased_onto = history.main().unwrap();
        main_changes(&folder, "c.stave", "c, after the rebase\n");

        let (base, files) = rebased_line(&history, &id).unwrap();
        assert_eq!(base, rebased_onto, "the base is where it was rebased onto");
        assert_ne!(base, history.main().unwrap(), "main moved since");

        let main = history.main().unwrap();
        let error = history.apply(&id, &agent(), &base, &files).unwrap_err();
        assert!(
            matches!(error, stave_history::HistoryError::MainMovedSince { .. }),
            "{error:?}"
        );
        assert_eq!(history.main().unwrap(), main, "nothing recorded");
        let kept = history.files_at(&main).unwrap();
        assert_eq!(text(&kept, "c.stave"), "c, after the rebase\n");
        let _ = std::fs::remove_dir_all(&folder);
    }
}
