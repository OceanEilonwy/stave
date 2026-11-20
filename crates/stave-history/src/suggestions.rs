//! Suggestions: an agent's lines of revisions, one bookmark each.

use jj_lib::backend::CommitId;
use jj_lib::commit::Commit;
use jj_lib::object_id::ObjectId as _;
use jj_lib::repo::{ReadonlyRepo, Repo as _};

use crate::SuggestionId;
use crate::actor::{Actor, Description};
use crate::error::HistoryError;
use crate::history::{History, block_on};
use crate::paths::ProjectFiles;
use crate::store::{self, SUGGESTION_PREFIX};

/// The bookmark holding a suggestion's latest revision.
pub(crate) fn suggestion_bookmark(id: &SuggestionId) -> String {
    format!("{SUGGESTION_PREFIX}{id}")
}

/// A suggestion's line of revisions, read from the repository.
pub(crate) struct Line {
    /// The latest revision.
    pub tip: Commit,
    /// The revision on main the line starts from.
    #[expect(dead_code, reason = "undo and the suggestions list use it, next")]
    pub base: CommitId,
    /// The agent who made the first revision.
    #[expect(dead_code, reason = "undo and the suggestions list use it, next")]
    pub agent: Option<Actor>,
    /// The first revision's title.
    pub title: String,
    /// Every revision's patches, oldest first.
    pub patches: Vec<String>,
}

/// Reads suggestion `id`'s line: from its bookmark back through the
/// revisions that carry its id.
pub(crate) async fn line(repo: &ReadonlyRepo, id: &SuggestionId) -> Result<Line, HistoryError> {
    let unknown = || HistoryError::UnknownSuggestion(id.clone());
    let tip_id = store::bookmark(repo, &suggestion_bookmark(id)).ok_or_else(unknown)?;
    let store = repo.store();
    let tip = store::commit(store, &tip_id).await?;
    let mut newest_first = Vec::new();
    let mut next = tip.clone();
    let base = loop {
        let description = Description::parse(next.description());
        if description.suggestion.as_deref() != Some(id.as_str()) {
            break next.id().clone();
        }
        let parent = next.parent_ids().first().cloned();
        newest_first.push(description);
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
    })
}

impl History {
    /// Records a suggestion by `agent`: `files` are the whole project after
    /// `patch`, which checked clean. With `onto`, the revision goes on the
    /// end of that suggestion's line; otherwise a new suggestion starts from
    /// main's latest revision, titled `title`. Main is never touched.
    ///
    /// # Errors
    ///
    /// [`HistoryError::UnknownSuggestion`] when `onto` names none.
    pub fn suggest(
        &mut self,
        agent: &Actor,
        onto: Option<&SuggestionId>,
        files: &ProjectFiles,
        patch: &str,
        title: &str,
    ) -> Result<SuggestionId, HistoryError> {
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
                    (id, store::main_id(&repo))
                }
            };
            let tree = store::tree_from_files(&store, files).await?;
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
}
