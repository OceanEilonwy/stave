//! Revisions: `suggest`, `list_suggestions`, `list_versions`,
//! `apply_suggestion`, `discard_suggestion`, `undo`.
//!
//! An agent's changes go into its own line, never straight into the
//! project: `suggest` records a patch that fits as a suggestion (main is
//! not touched). `apply_suggestion` is the merge: it re-tries the
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
    Actor, ActorKind, HistoryError, MergePolicy, SuggestionId, UndoScope, merge_policy,
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
                       suggestion given as onto) and recorded only when they fit. The project \
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
            let files = agent_line::line_files(&history, onto.as_ref())?;
            let after =
                match agent_line::replay(folder, &files, std::slice::from_ref(&arguments.patch))? {
                    Replay::Fits { files, .. } => files,
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
            let id = history.suggest(&actor, onto.as_ref(), &after, &arguments.patch, title)?;
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
            let suggestion = history
                .suggestions()?
                .into_iter()
                .find(|suggestion| suggestion.id == id)
                .ok_or_else(|| HistoryError::UnknownSuggestion(id.clone()))?;
            if !suggestion.conflicted.is_empty() {
                return Err(HistoryError::SuggestionConflicted {
                    id,
                    files: suggestion.conflicted,
                }
                .into());
            }

            // Re-try the patches on main as it is now.
            let base = history.main()?;
            let main_files = history.files_at(&base)?;
            let (base, merged) = match agent_line::replay(folder, &main_files, &suggestion.patches)?
            {
                Replay::Fits {
                    files, errors: 0, ..
                } => (base, files),
                Replay::Fits { problems, .. } => return Err(McpError::NotClean(problems)),
                Replay::Refused { title, problems } => {
                    // A patch no longer fits: move the line onto main and
                    // check its files, merged as text, instead.
                    let clashes = history.rebase_suggestion(&id)?;
                    if !clashes.is_empty() {
                        return Err(McpError::Clashes {
                            files: clashes.iter().map(ToString::to_string).collect(),
                        });
                    }
                    let base = history.main()?;
                    let files = history.suggestion_files(&id)?;
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
            let main = history.apply(&id, &actor, &base, &merged)?;
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
