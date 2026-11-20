//! The steps that move main: a save (the window's fork merged into main)
//! and applying a suggestion; and catching the window up with main.

use jj_lib::repo::Repo as _;

use crate::actor::{Actor, Description};
use crate::error::HistoryError;
use crate::history::{History, block_on};
use crate::paths::{ProjectFiles, RelativePath};
use crate::store::{self, MAIN, MOVED_MAIN_ATTRIBUTE, WINDOW};
use crate::suggestions::{self, suggestion_bookmark};
use crate::{RevisionId, SuggestionId};

/// What catching the window up with main asks the window to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CatchUp {
    /// Main hasn't moved since the window was last in step: nothing to do.
    UpToDate,
    /// Main moved: write these files into the folder (and its buffers).
    Write {
        /// Every file whose contents change, with its new contents.
        files: ProjectFiles,
        /// Files to delete from the folder.
        removed: Vec<RelativePath>,
        /// Files among `files` where the window's own changes clash with
        /// main's; they hold conflict markers, every side in full.
        conflicted: Vec<RelativePath>,
    },
}

impl History {
    /// Saves the window's files: records them as a revision by `actor` and
    /// merges them into main. Never refused for problems in the design.
    ///
    /// When main hasn't moved since the window was last in step, the new
    /// revision goes straight onto main. When it has (someone else applied
    /// a change), the files are first recorded on the window's own fork,
    /// then merged with main into a new revision on main, so main stays one
    /// line; a clash is recorded in that revision and reaches the folder
    /// through the next [`History::catch_up`], which the window then runs.
    /// Saving files identical to main's records nothing and returns main.
    ///
    /// # Errors
    ///
    /// [`HistoryError::Store`] when the repository can't be written.
    pub fn save(
        &mut self,
        files: &ProjectFiles,
        actor: &Actor,
        message: &str,
    ) -> Result<RevisionId, HistoryError> {
        let _lock = self.lock_main()?;
        let new_main = block_on(async {
            let repo = self.head().await?;
            let store = repo.store().clone();
            let main = store::commit(&store, &store::main_id(&repo)).await?;
            let base = store::commit(&store, &store::window_id(&repo)).await?;
            let tree = store::tree_from_files(&store, files).await?;
            if base.id() == main.id() && tree.tree_ids() == main.tree_ids() {
                return Ok(main.id().clone());
            }
            let description = Description {
                message: message.to_owned(),
                actor: Some(actor.clone()),
                ..Description::default()
            };
            let mut transaction =
                Self::transaction(&repo, Some(actor), &format!("save: {message}"));
            transaction.set_attribute(MOVED_MAIN_ATTRIBUTE.to_owned(), "true".to_owned());
            let (new_main, window) = if base.id() == main.id() {
                let saved =
                    store::record(transaction.repo_mut(), main.id(), tree, &description, actor)
                        .await?;
                (saved.id().clone(), saved.id().clone())
            } else {
                let own = store::record(
                    transaction.repo_mut(),
                    base.id(),
                    tree.clone(),
                    &description,
                    actor,
                )
                .await?;
                let merged = store::merge_trees(base.tree(), tree, main.tree()).await?;
                let merged = store::record(
                    transaction.repo_mut(),
                    main.id(),
                    merged,
                    &description,
                    actor,
                )
                .await?;
                (merged.id().clone(), own.id().clone())
            };
            store::set_bookmark(transaction.repo_mut(), MAIN, Some(&new_main));
            store::set_bookmark(transaction.repo_mut(), WINDOW, Some(&window));
            transaction
                .commit(format!("save: {message}"))
                .await
                .map_err(HistoryError::store)?;
            Ok::<_, HistoryError>(new_main)
        })?;
        Ok(RevisionId(store::hex(&new_main)))
    }

    /// Catches the window up with main after someone else moved it.
    ///
    /// `folder_files` are the window's files as they are now, unsaved edits
    /// included. Main's changes since the window was last in step are merged
    /// with them; where both changed the same lines the file gets conflict
    /// markers. The window is then in step with main, so it writes what this
    /// returns.
    ///
    /// # Errors
    ///
    /// [`HistoryError::Store`] when the repository can't be read or written.
    pub fn catch_up(&mut self, folder_files: &ProjectFiles) -> Result<CatchUp, HistoryError> {
        let _lock = self.lock_main()?;
        block_on(async {
            let repo = self.head().await?;
            let store = repo.store().clone();
            let main = store::commit(&store, &store::main_id(&repo)).await?;
            let window = store::commit(&store, &store::window_id(&repo)).await?;
            if window.id() == main.id() {
                return Ok(CatchUp::UpToDate);
            }
            let folder_tree = store::tree_from_files(&store, folder_files).await?;
            let merged = store::merge_trees(window.tree(), folder_tree, main.tree()).await?;
            let (merged_files, conflicted) = store::files_of(&merged).await?;

            let mut transaction = Self::transaction(&repo, None, "catch up");
            store::set_bookmark(transaction.repo_mut(), WINDOW, Some(main.id()));
            transaction
                .commit("catch up with main")
                .await
                .map_err(HistoryError::store)?;

            let removed: Vec<RelativePath> = folder_files
                .paths()
                .filter(|path| !merged_files.contains(path))
                .cloned()
                .collect();
            let files: ProjectFiles = merged_files
                .into_iter()
                .filter(|(path, contents)| folder_files.get(path) != Some(contents.as_slice()))
                .collect();
            if files.is_empty() && removed.is_empty() {
                return Ok(CatchUp::UpToDate);
            }
            Ok(CatchUp::Write {
                files,
                removed,
                conflicted,
            })
        })
    }

    /// Applies a suggestion to main: records `merged` as a revision on main,
    /// made by `actor`, and removes the suggestion.
    ///
    /// The caller has replayed the suggestion's patches on main and checked
    /// the result clean; `merged` is that result, built on the main this
    /// handle last reported ([`History::main`]). If main moved since, the
    /// change from that main to `merged` is merged onto main as it is now.
    ///
    /// # Errors
    ///
    /// [`HistoryError::UnknownSuggestion`]; [`HistoryError::SuggestionConflicted`]
    /// when the suggestion still holds clashes; [`HistoryError::MainMoved`]
    /// when main moved and the changes clash.
    pub fn apply(
        &mut self,
        id: &SuggestionId,
        actor: &Actor,
        merged: &ProjectFiles,
    ) -> Result<RevisionId, HistoryError> {
        let _lock = self.lock_main()?;
        let new_main = block_on(async {
            let repo = self.head().await?;
            let store = repo.store().clone();
            let line = suggestions::line(&repo, id).await?;
            let conflicted = store::conflicted_paths(&line.tip.tree()).await?;
            if !conflicted.is_empty() {
                return Err(HistoryError::SuggestionConflicted {
                    id: id.clone(),
                    files: conflicted,
                });
            }
            let main = store::commit(&store, &store::main_id(&repo)).await?;
            let mut tree = store::tree_from_files(&store, merged).await?;
            if let Some(observed) = self.observed_main()
                && &observed != main.id()
            {
                let observed = store::commit(&store, &observed).await?;
                tree = store::merge_trees(observed.tree(), tree, main.tree()).await?;
                let clashes = store::conflicted_paths(&tree).await?;
                if !clashes.is_empty() {
                    return Err(HistoryError::MainMoved { files: clashes });
                }
            }
            let description = Description {
                message: line.title.clone(),
                actor: Some(actor.clone()),
                applies: Some(id.as_str().to_owned()),
                patches: line.patches.clone(),
                ..Description::default()
            };
            let action = format!("apply suggestion: {}", line.title);
            let mut transaction = Self::transaction(&repo, Some(actor), &action);
            transaction.set_attribute(MOVED_MAIN_ATTRIBUTE.to_owned(), "true".to_owned());
            let applied =
                store::record(transaction.repo_mut(), main.id(), tree, &description, actor).await?;
            store::set_bookmark(transaction.repo_mut(), MAIN, Some(applied.id()));
            store::set_bookmark(transaction.repo_mut(), &suggestion_bookmark(id), None);
            transaction
                .commit(action)
                .await
                .map_err(HistoryError::store)?;
            Ok(applied.id().clone())
        })?;
        self.observe_main(new_main.clone());
        Ok(RevisionId(store::hex(&new_main)))
    }
}
