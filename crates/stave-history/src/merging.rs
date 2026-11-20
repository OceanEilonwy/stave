//! The steps that move main: a save (the window's fork merged into main)
//! and applying a suggestion; and catching the window up with main.

use std::collections::BTreeSet;

use jj_lib::conflict_labels::ConflictLabels;
use jj_lib::conflicts::{
    ConflictMarkerStyle, ConflictMaterializeOptions, materialize_merge_result_to_bytes,
};
use jj_lib::files::{FileMergeHunkLevel, MergeOptions, MergeResult, merge_hunks};
use jj_lib::merge::{Merge, SameChange};
use jj_lib::repo::Repo as _;

use crate::actor::{Actor, Description};
use crate::error::HistoryError;
use crate::history::{History, block_on};
use crate::paths::{ProjectFiles, RelativePath};
use crate::store::{self, MAIN, MOVED_MAIN_ATTRIBUTE, SUGGESTION_ATTRIBUTE, WINDOW};
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
    /// Nothing was written and the window is still behind main: these
    /// files, which main changed, are unsaved in the window (or changed in
    /// the folder while catching up). The next save merges them.
    Waiting {
        /// The files standing in the way, sorted.
        files: Vec<RelativePath>,
    },
}

/// What the writer [`History::catch_up`] hands its files to did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Written {
    /// Every file was written and removed as asked.
    All,
    /// Nothing was written: these files changed (on disk, or unsaved in
    /// the window) since the folder was read, so writing would lose that
    /// change.
    HeldBack {
        /// The files that changed meanwhile, sorted.
        files: Vec<RelativePath>,
    },
}

/// Text merged three ways, line by line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergedText {
    /// The merged text; where both sides changed the same lines it holds
    /// conflict markers with every side in full.
    pub contents: Vec<u8>,
    /// Whether it holds conflict markers.
    pub conflicted: bool,
}

/// Merges the change from `before` to `yours` with the change from `before`
/// to `theirs`, as a save or a catch-up merges a file: where both changed
/// the same lines, both are kept between conflict markers.
pub fn merge_text(before: &[u8], yours: &[u8], theirs: &[u8]) -> MergedText {
    let options = MergeOptions {
        hunk_level: FileMergeHunkLevel::Line,
        same_change: SameChange::Accept,
    };
    let inputs = Merge::from_vec(vec![yours, before, theirs]);
    match merge_hunks(&inputs, &options) {
        MergeResult::Resolved(contents) => MergedText {
            contents: contents.to_vec(),
            conflicted: false,
        },
        MergeResult::Conflict(_) => {
            let labels = ConflictLabels::from_vec(vec![
                "yours".to_owned(),
                "before".to_owned(),
                "theirs".to_owned(),
            ]);
            let materialize = ConflictMaterializeOptions {
                marker_style: ConflictMarkerStyle::Snapshot,
                marker_len: None,
                merge: options,
            };
            MergedText {
                contents: materialize_merge_result_to_bytes(&inputs, &labels, &materialize)
                    .to_vec(),
                conflicted: true,
            }
        }
    }
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
    /// Saving files identical to the window's last version (main's, when in
    /// step) records nothing and returns main.
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
            let repo = self.head_with_git().await?;
            let store = repo.store().clone();
            let main = store::commit(&store, &store::main_id(&repo)?).await?;
            let base = store::commit(&store, &store::window_id(&repo)?).await?;
            let tree = store::tree_over(&store, &base.tree(), files).await?;
            // Nothing changed since the window's base: nothing to record,
            // wherever main is (the window catches up separately).
            if tree.tree_ids() == base.tree_ids() {
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
            self.export_main(&repo, &mut transaction)?;
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
    /// `folder_files` are the folder's files as the caller read them, and
    /// `unsaved` the files the window holds unsaved. Everything is decided
    /// and written under the main lock, so main can't move in between:
    ///
    /// - when main changed a file in `unsaved` since the window was last in
    ///   step, nothing is written and this returns [`CatchUp::Waiting`];
    /// - otherwise main's changes are merged with `folder_files` (where both
    ///   changed the same lines the file gets conflict markers) and handed
    ///   to `write`, which writes them into the folder. Only once it wrote
    ///   them all is the window recorded as in step with main; when it fails
    ///   or holds back, the window stays where it was.
    ///
    /// # Errors
    ///
    /// [`HistoryError::Store`] when the repository can't be read or written;
    /// whatever `write` returns.
    pub fn catch_up<E: From<HistoryError>>(
        &mut self,
        folder_files: &ProjectFiles,
        unsaved: &BTreeSet<RelativePath>,
        write: impl FnOnce(&CatchUp) -> Result<Written, E>,
    ) -> Result<CatchUp, E> {
        let _lock = self.lock_main()?;
        let (repo, main, decided) = block_on(async {
            let repo = self.head_with_git().await?;
            let store = repo.store().clone();
            let main = store::commit(&store, &store::main_id(&repo)?).await?;
            let window = store::commit(&store, &store::window_id(&repo)?).await?;
            if window.id() == main.id() {
                return Ok((repo, main, None));
            }
            let window_tree = window.tree();
            let main_tree = main.tree();
            let mut waiting = Vec::new();
            for path in unsaved {
                if !store::same_value(&window_tree, &main_tree, path).await? {
                    waiting.push(path.clone());
                }
            }
            if !waiting.is_empty() {
                return Ok((repo, main, Some(CatchUp::Waiting { files: waiting })));
            }
            let folder_tree = store::tree_over(&store, &window_tree, folder_files).await?;
            let merged = store::merge_trees(window.tree(), folder_tree, main_tree).await?;
            let (merged_files, conflicted) = store::files_of(&merged).await?;
            let removed: Vec<RelativePath> = folder_files
                .paths()
                .filter(|path| !merged_files.contains(path))
                .cloned()
                .collect();
            let mut files = ProjectFiles::new();
            for (path, contents) in merged_files {
                if folder_files.get(&path) == Some(contents.as_slice()) {
                    continue;
                }
                // A file the folder doesn't show (one `.gitignore` excludes,
                // tracked all the same) is written only when main changed it.
                if !folder_files.contains(&path)
                    && store::same_value(&window_tree, &merged, &path).await?
                {
                    continue;
                }
                files.insert(path, contents);
            }
            let decided = if files.is_empty() && removed.is_empty() {
                CatchUp::UpToDate
            } else {
                CatchUp::Write {
                    files,
                    removed,
                    conflicted,
                }
            };
            Ok::<_, HistoryError>((repo, main, Some(decided)))
        })?;
        let Some(decided) = decided else {
            return Ok(CatchUp::UpToDate);
        };
        match &decided {
            CatchUp::Waiting { .. } => return Ok(decided),
            CatchUp::UpToDate => {}
            CatchUp::Write { .. } => match write(&decided)? {
                Written::All => {}
                Written::HeldBack { files } => return Ok(CatchUp::Waiting { files }),
            },
        }
        block_on(async {
            let mut transaction = Self::transaction(&repo, None, "catch up");
            store::set_bookmark(transaction.repo_mut(), WINDOW, Some(main.id()));
            transaction
                .commit("catch up with main")
                .await
                .map_err(HistoryError::store)?;
            Ok::<(), HistoryError>(())
        })?;
        Ok(decided)
    }

    /// Applies a suggestion to main: records `merged` as a revision on main,
    /// made by `actor`, and removes the suggestion.
    ///
    /// The caller has replayed the suggestion's patches on the files of
    /// `base` (main as [`History::main`] reported it when the caller read
    /// them) and checked the result clean; `merged` is that result. If main
    /// moved since `base`, nothing is recorded: the result was checked on a
    /// main that is gone, so the caller reads main again, replays and checks
    /// on it, and tries again.
    ///
    /// # Errors
    ///
    /// [`HistoryError::UnknownSuggestion`]; [`HistoryError::UnknownRevision`]
    /// when `base` names no revision; [`HistoryError::SuggestionConflicted`]
    /// when the suggestion still holds clashes;
    /// [`HistoryError::MainMovedSince`] when main moved since `base`;
    /// [`HistoryError::SettingsProtected`] when `merged` changes a file in
    /// `.stave/`.
    pub fn apply(
        &mut self,
        id: &SuggestionId,
        actor: &Actor,
        base: &RevisionId,
        merged: &ProjectFiles,
    ) -> Result<RevisionId, HistoryError> {
        let _lock = self.lock_main()?;
        let new_main = block_on(async {
            let repo = self.head_with_git().await?;
            let store = repo.store().clone();
            let line = suggestions::line(&repo, id).await?;
            let conflicted = store::conflicted_paths(&line.tip.tree()).await?;
            if !conflicted.is_empty() {
                return Err(HistoryError::SuggestionConflicted {
                    id: id.clone(),
                    files: conflicted,
                });
            }
            let main = store::commit(&store, &store::main_id(&repo)?).await?;
            let base = store::commit(&store, &self.resolve(&repo, base)?).await?;
            let tree = store::tree_over(&store, &base.tree(), merged).await?;
            store::refuse_settings_changes(&base.tree(), &tree).await?;
            // The caller checked `merged` against `base`; on any other main
            // the result was never checked, so it is not recorded.
            if base.id() != main.id() {
                return Err(HistoryError::MainMovedSince {
                    main: RevisionId(store::hex(main.id())),
                });
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
            transaction.set_attribute(SUGGESTION_ATTRIBUTE.to_owned(), id.as_str().to_owned());
            let applied =
                store::record(transaction.repo_mut(), main.id(), tree, &description, actor).await?;
            store::set_bookmark(transaction.repo_mut(), MAIN, Some(applied.id()));
            store::set_bookmark(transaction.repo_mut(), &suggestion_bookmark(id), None);
            self.export_main(&repo, &mut transaction)?;
            transaction
                .commit(action)
                .await
                .map_err(HistoryError::store)?;
            Ok(applied.id().clone())
        })?;
        Ok(RevisionId(store::hex(&new_main)))
    }
}
