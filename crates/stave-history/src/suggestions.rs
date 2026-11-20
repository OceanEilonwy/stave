//! Suggestions: an agent's lines of revisions, one bookmark each.

use jj_lib::backend::CommitId;
use jj_lib::commit::Commit;
use jj_lib::object_id::ObjectId as _;
use jj_lib::repo::{ReadonlyRepo, Repo as _};

use crate::actor::{Actor, ActorKind, Description};
use crate::error::HistoryError;
use crate::history::{History, block_on};
use crate::paths::{ProjectFiles, RelativePath};
use crate::store::{self, SUGGESTION_PREFIX};
use crate::{RevisionId, SuggestionId};
use jj_lib::rewrite::rebase_commit;

/// The bookmark holding a suggestion's latest revision.
pub(crate) fn suggestion_bookmark(id: &SuggestionId) -> String {
    format!("{SUGGESTION_PREFIX}{id}")
}

/// A suggestion's line of revisions, read from the repository.
pub(crate) struct Line {
    /// The latest revision.
    pub tip: Commit,
    /// The revision on main the line starts from.
    pub base: CommitId,
    /// The agent who made the first revision.
    pub agent: Option<Actor>,
    /// The first revision's title.
    pub title: String,
    /// Every revision's patches, oldest first.
    pub patches: Vec<String>,
    /// Every revision on the line, newest first.
    pub commits: Vec<Commit>,
}

/// Reads suggestion `id`'s line: from its bookmark back through the
/// revisions that carry its id.
pub(crate) async fn line(repo: &ReadonlyRepo, id: &SuggestionId) -> Result<Line, HistoryError> {
    let unknown = || HistoryError::UnknownSuggestion(id.clone());
    let tip_id = store::bookmark(repo, &suggestion_bookmark(id)).ok_or_else(unknown)?;
    let store = repo.store();
    let tip = store::commit(store, &tip_id).await?;
    let mut newest_first = Vec::new();
    let mut commits = Vec::new();
    let mut next = tip.clone();
    let base = loop {
        let description = Description::parse(next.description());
        if description.suggestion.as_deref() != Some(id.as_str()) {
            break next.id().clone();
        }
        let parent = next.parent_ids().first().cloned();
        newest_first.push(description);
        commits.push(next.clone());
        match parent {
            Some(parent) => next = store::commit(store, &parent).await?,
            None => break store.root_commit_id().clone(),
        }
    };
    let first = newest_first.last().ok_or_else(unknown)?;
    let agent = first.actor.clone();
    let title = first.message.clone();
    let patches = newest_first
        .iter()
        .rev()
        .flat_map(|description| description.patches.iter().cloned())
        .collect();
    Ok(Line {
        tip,
        base,
        agent,
        title,
        patches,
        commits,
    })
}

/// A suggestion: an agent's line of revisions, waiting to be applied or
/// discarded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suggestion {
    /// Its id.
    pub id: SuggestionId,
    /// The agent who made it.
    pub agent: Actor,
    /// What it does, in the agent's words (its first revision's title).
    pub title: String,
    /// The patches that made it, in order.
    pub patches: Vec<String>,
    /// The revision on main it starts from.
    pub base: RevisionId,
    /// Files where it clashes with main since a rebase; it can't be applied
    /// until they are resolved.
    pub conflicted: Vec<RelativePath>,
}

impl History {
    /// Records a suggestion by `agent`: `files` are the whole project after
    /// `patch`, which checked clean. With `onto`, the revision goes on the
    /// end of that suggestion's line; otherwise a new suggestion starts from
    /// main's latest revision, titled `title`. Main is never touched.
    ///
    /// # Errors
    ///
    /// [`HistoryError::UnknownSuggestion`] when `onto` names none;
    /// [`HistoryError::SettingsProtected`] when `files` change a file in
    /// `.stave/` (the project's settings, which only the person changes).
    pub fn suggest(
        &mut self,
        agent: &Actor,
        onto: Option<&SuggestionId>,
        files: &ProjectFiles,
        patch: &str,
        title: &str,
    ) -> Result<SuggestionId, HistoryError> {
        let _lock = self.lock_main()?;
        block_on(async {
            let repo = self.head().await?;
            let store = repo.store().clone();
            let (id, parent) = match onto {
                Some(id) => {
                    let tip = store::bookmark(&repo, &suggestion_bookmark(id))
                        .ok_or_else(|| HistoryError::UnknownSuggestion(id.clone()))?;
                    (id.clone(), tip)
                }
                None => {
                    let id = SuggestionId(self.settings.get_rng().new_change_id(6).hex());
                    (id, store::main_id(&repo)?)
                }
            };
            let parent_tree = store::commit(&store, &parent).await?.tree();
            let tree = store::tree_over(&store, &parent_tree, files).await?;
            store::refuse_settings_changes(&parent_tree, &tree).await?;
            let description = Description {
                message: title.to_owned(),
                actor: Some(agent.clone()),
                suggestion: Some(id.as_str().to_owned()),
                patches: vec![patch.to_owned()],
                ..Description::default()
            };
            let action = format!("suggest: {title}");
            let mut transaction = Self::transaction(&repo, Some(agent), &action);
            let revision =
                store::record(transaction.repo_mut(), &parent, tree, &description, agent).await?;
            store::set_bookmark(
                transaction.repo_mut(),
                &suggestion_bookmark(&id),
                Some(revision.id()),
            );
            transaction
                .commit(action)
                .await
                .map_err(HistoryError::store)?;
            Ok(id)
        })
    }

    /// The files of a suggestion's latest revision. Files whose changes
    /// clash hold conflict markers.
    ///
    /// # Errors
    ///
    /// [`HistoryError::UnknownSuggestion`] when there is none.
    pub fn suggestion_files(&self, id: &SuggestionId) -> Result<ProjectFiles, HistoryError> {
        block_on(async {
            let repo = self.head().await?;
            let line = line(&repo, id).await?;
            let (files, _conflicted) = store::files_of(&line.tip.tree()).await?;
            Ok(files)
        })
    }

    /// Every suggestion, sorted by id.
    ///
    /// # Errors
    ///
    /// [`HistoryError::Store`] when the repository can't be read.
    pub fn suggestions(&self) -> Result<Vec<Suggestion>, HistoryError> {
        block_on(async {
            let repo = self.head().await?;
            let ids: Vec<SuggestionId> = repo
                .view()
                .local_bookmarks()
                // A bookmark moved two ways at once (outside Stave; Stave
                // moves them under the lock) names no single line, so that
                // suggestion is left out rather than hiding the others.
                .filter(|(_name, target)| target.as_normal().is_some())
                .filter_map(|(name, _target)| {
                    name.as_str()
                        .strip_prefix(SUGGESTION_PREFIX)
                        .map(|id| SuggestionId(id.to_owned()))
                })
                .collect();
            let mut suggestions = Vec::with_capacity(ids.len());
            for id in ids {
                let line = line(&repo, &id).await?;
                let conflicted = store::conflicted_paths(&line.tip.tree()).await?;
                let agent = line.agent.unwrap_or_else(|| Actor {
                    kind: ActorKind::Agent,
                    name: line.tip.author().name.clone(),
                    id: String::new(),
                });
                suggestions.push(Suggestion {
                    id,
                    agent,
                    title: line.title,
                    patches: line.patches,
                    base: RevisionId(store::hex(&line.base)),
                    conflicted,
                });
            }
            suggestions.sort_by(|left, right| left.id.cmp(&right.id));
            Ok(suggestions)
        })
    }

    /// Moves a suggestion's line onto main's latest revision, for when its
    /// patches no longer fit main. Text that clashes stays on the
    /// suggestion, with conflict markers; returns those files (empty when
    /// the rebase was clean). Main is never touched.
    ///
    /// # Errors
    ///
    /// [`HistoryError::UnknownSuggestion`] when there is none.
    pub fn rebase_suggestion(
        &mut self,
        id: &SuggestionId,
    ) -> Result<Vec<RelativePath>, HistoryError> {
        let _lock = self.lock_main()?;
        block_on(async {
            let repo = self.head().await?;
            let line = line(&repo, id).await?;
            let main = store::main_id(&repo)?;
            if line.base == main {
                return store::conflicted_paths(&line.tip.tree()).await;
            }
            let Some(oldest) = line.commits.last() else {
                return Err(HistoryError::UnknownSuggestion(id.clone()));
            };
            let action = format!("rebase suggestion: {}", line.title);
            let mut transaction = Self::transaction(&repo, None, &action);
            rebase_commit(transaction.repo_mut(), oldest.clone(), vec![main])
                .await
                .map_err(HistoryError::store)?;
            // The rest of the line follows, and its bookmark with it.
            transaction
                .repo_mut()
                .rebase_descendants()
                .await
                .map_err(HistoryError::store)?;
            let repo = transaction
                .commit(action)
                .await
                .map_err(HistoryError::store)?;
            let line = self::line(&repo, id).await?;
            store::conflicted_paths(&line.tip.tree()).await
        })
    }

    /// Replaces the files of a suggestion's latest revision, as when its
    /// clashes are resolved. Main is never touched.
    ///
    /// # Errors
    ///
    /// [`HistoryError::UnknownSuggestion`] when there is none.
    pub fn write_suggestion_files(
        &mut self,
        id: &SuggestionId,
        files: &ProjectFiles,
    ) -> Result<(), HistoryError> {
        let _lock = self.lock_main()?;
        block_on(async {
            let repo = self.head().await?;
            let line = line(&repo, id).await?;
            let store = repo.store().clone();
            let tip_tree = line.tip.tree();
            let tree = store::tree_over(&store, &tip_tree, files).await?;
            store::refuse_settings_changes(&tip_tree, &tree).await?;
            let action = format!("resolve suggestion: {}", line.title);
            let mut transaction = Self::transaction(&repo, None, &action);
            let resolved = transaction
                .repo_mut()
                .rewrite_commit(&line.tip)
                .set_tree(tree)
                .write()
                .await
                .map_err(HistoryError::store)?;
            transaction
                .repo_mut()
                .rebase_descendants()
                .await
                .map_err(HistoryError::store)?;
            store::set_bookmark(
                transaction.repo_mut(),
                &suggestion_bookmark(id),
                Some(resolved.id()),
            );
            transaction
                .commit(action)
                .await
                .map_err(HistoryError::store)?;
            Ok(())
        })
    }

    /// Discards a suggestion: its line is abandoned and its bookmark
    /// removed. It can be brought back with [`History::undo`].
    ///
    /// # Errors
    ///
    /// [`HistoryError::UnknownSuggestion`] when there is none.
    pub fn discard(&mut self, id: &SuggestionId, actor: &Actor) -> Result<(), HistoryError> {
        let _lock = self.lock_main()?;
        block_on(async {
            let repo = self.head().await?;
            let line = line(&repo, id).await?;
            let action = format!("discard suggestion: {}", line.title);
            let mut transaction = Self::transaction(&repo, Some(actor), &action);
            store::set_bookmark(transaction.repo_mut(), &suggestion_bookmark(id), None);
            for commit in &line.commits {
                transaction.repo_mut().record_abandoned_commit(commit);
            }
            transaction
                .repo_mut()
                .rebase_descendants()
                .await
                .map_err(HistoryError::store)?;
            transaction
                .commit(action)
                .await
                .map_err(HistoryError::store)?;
            Ok(())
        })
    }
}
