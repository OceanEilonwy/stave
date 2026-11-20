//! The jj side: settings, trees built from project files and files read back
//! out of trees, commits and bookmarks. Nothing here is public; no jj type
//! leaves the crate.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use futures::StreamExt as _;
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use jj_lib::backend::{CommitId, CopyId, MillisSinceEpoch, Signature, Timestamp, TreeValue};
use jj_lib::commit::Commit;
use jj_lib::config::StackedConfig;
use jj_lib::conflicts::{
    ConflictMarkerStyle, ConflictMaterializeOptions, MaterializedTreeValue,
    materialize_merge_result_to_bytes, materialize_tree_value,
};
use jj_lib::matchers::EverythingMatcher;
use jj_lib::merge::Merge;
use jj_lib::merged_tree::MergedTree;
use jj_lib::merged_tree_builder::MergedTreeBuilder;
use jj_lib::object_id::ObjectId as _;
use jj_lib::op_store::RefTarget;
use jj_lib::ref_name::RefNameBuf;
use jj_lib::repo::{MutableRepo, ReadonlyRepo, Repo as _};
use jj_lib::repo_path::RepoPathBuf;
use jj_lib::settings::UserSettings;
use jj_lib::store::Store;

use crate::actor::{Actor, Description};
use crate::error::HistoryError;
use crate::paths::{ProjectFiles, RelativePath};

/// The bookmark main moves on.
pub(crate) const MAIN: &str = "main";
/// The bookmark marking the revision the window's folder was last in step
/// with: the base for merging a save or catching up.
pub(crate) const WINDOW: &str = "stave-window";
/// The start of every suggestion's bookmark name.
pub(crate) const SUGGESTION_PREFIX: &str = "stave-suggestion-";

/// The operation attribute naming who made an operation.
pub(crate) const ACTOR_ATTRIBUTE: &str = "stave-actor";
/// The operation attribute describing an operation in plain words.
pub(crate) const ACTION_ATTRIBUTE: &str = "stave-action";
/// The operation attribute an undo carries: the operation it reverted.
pub(crate) const UNDOES_ATTRIBUTE: &str = "stave-undoes";
/// The operation attribute set when an operation moved main.
pub(crate) const MOVED_MAIN_ATTRIBUTE: &str = "stave-moved-main";
/// The operation attribute naming the suggestion an operation changed.
pub(crate) const SUGGESTION_ATTRIBUTE: &str = "stave-suggestion";

/// The settings Stave runs jj with: jj's defaults only, nothing read from
/// the user's own jj configuration.
pub(crate) fn settings() -> Result<UserSettings, HistoryError> {
    UserSettings::from_config_and_home_dir(StackedConfig::with_defaults(), None)
        .map_err(HistoryError::store)
}

/// A bookmark's commit, when it points at exactly one.
pub(crate) fn bookmark(repo: &ReadonlyRepo, name: &str) -> Option<CommitId> {
    let name = RefNameBuf::from(name);
    repo.view()
        .get_local_bookmark(name.as_ref())
        .as_normal()
        .cloned()
}

/// A bookmark's commit in a transaction's repository, when it points at
/// exactly one.
pub(crate) fn bookmark_in(repo: &MutableRepo, name: &str) -> Option<CommitId> {
    let name = RefNameBuf::from(name);
    repo.view()
        .get_local_bookmark(name.as_ref())
        .as_normal()
        .cloned()
}

/// The target for a bookmark pointing at `id`.
pub(crate) fn target(id: &CommitId) -> RefTarget {
    RefTarget::normal(id.clone())
}

/// The commit a bookmark points at: `None` when it is absent, an error when
/// it points at more than one (moved two ways at once outside Stave).
fn single_bookmark(repo: &ReadonlyRepo, name: &str) -> Result<Option<CommitId>, HistoryError> {
    let name_buf = RefNameBuf::from(name);
    let target = repo.view().get_local_bookmark(name_buf.as_ref());
    if target.is_absent() {
        return Ok(None);
    }
    match target.as_normal() {
        Some(id) => Ok(Some(id.clone())),
        None => Err(HistoryError::store(std::io::Error::other(format!(
            "the line \"{name}\" was moved two ways at once"
        )))),
    }
}

/// Main's commit: the bookmark, or the root commit before the first save.
pub(crate) fn main_id(repo: &ReadonlyRepo) -> Result<CommitId, HistoryError> {
    Ok(single_bookmark(repo, MAIN)?.unwrap_or_else(|| repo.store().root_commit_id().clone()))
}

/// The window's base: its bookmark, or main when it has none yet.
pub(crate) fn window_id(repo: &ReadonlyRepo) -> Result<CommitId, HistoryError> {
    match single_bookmark(repo, WINDOW)? {
        Some(id) => Ok(id),
        None => main_id(repo),
    }
}

/// Reads a commit.
pub(crate) async fn commit(store: &Arc<Store>, id: &CommitId) -> Result<Commit, HistoryError> {
    store
        .get_commit_async(id)
        .await
        .map_err(HistoryError::store)
}

/// The jj path for a project path.
fn repo_path(path: &RelativePath) -> Result<RepoPathBuf, HistoryError> {
    RepoPathBuf::from_internal_string(path.to_slash_string()).map_err(HistoryError::store)
}

/// The `.gitignore` files among a project's files, each with the folder it
/// sits in, deepest folders first, as git consults them.
fn gitignores(files: &ProjectFiles) -> Vec<(PathBuf, Gitignore)> {
    let mut found: Vec<(PathBuf, Gitignore)> = files
        .iter()
        .filter(|(path, _contents)| path.as_path().file_name() == Some(".gitignore".as_ref()))
        .map(|(path, contents)| {
            let folder = path
                .as_path()
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_default();
            let mut builder = GitignoreBuilder::new("");
            for line in String::from_utf8_lossy(contents).lines() {
                // A line git couldn't read is passed over, as git does.
                let _ignored = builder.add_line(None, line);
            }
            let matcher = builder.build().unwrap_or_else(|_| Gitignore::empty());
            (folder, matcher)
        })
        .collect();
    found.sort_by_key(|(folder, _matcher)| std::cmp::Reverse(folder.components().count()));
    found
}

/// Whether the `.gitignore` files in `ignores` exclude `path`.
fn is_ignored(ignores: &[(PathBuf, Gitignore)], path: &RelativePath) -> bool {
    for (folder, matcher) in ignores {
        let Ok(inside) = path.as_path().strip_prefix(folder) else {
            continue;
        };
        match matcher.matched_path_or_any_parents(inside, false) {
            ignore::Match::Ignore(_) => return true,
            ignore::Match::Whitelist(_) => return false,
            ignore::Match::None => {}
        }
    }
    false
}

/// Writes `files`, the project's files as Stave reads them, as a tree built
/// over `base`, the revision they were made from.
///
/// Files only change where `files` say so: each keeps `base`'s executable
/// bit, and what `files` never holds is kept from `base` as it was:
/// symbolic links, submodules, files `.gitignore` excludes (tracked in git
/// all the same) and anything under a path Stave doesn't read. Every other
/// file of `base` missing from `files` was deleted.
pub(crate) async fn tree_over(
    store: &Arc<Store>,
    base: &MergedTree,
    files: &ProjectFiles,
) -> Result<MergedTree, HistoryError> {
    let ignores = gitignores(files);
    let mut executable = BTreeSet::new();
    let mut builder = MergedTreeBuilder::new(base.clone());
    let mut entries = base.entries();
    while let Some((path, value)) = entries.next().await {
        let value = value.map_err(HistoryError::store)?;
        let Ok(relative) = RelativePath::new(Path::new(path.as_internal_file_string())) else {
            continue;
        };
        let is_file = match value.as_resolved() {
            Some(Some(TreeValue::File {
                executable: true, ..
            })) => {
                executable.insert(relative.clone());
                true
            }
            Some(Some(TreeValue::File { .. })) => true,
            // Not project files: Stave neither reads nor writes them.
            Some(Some(TreeValue::Symlink(_) | TreeValue::GitSubmodule(_) | TreeValue::Tree(_)))
            | Some(None) => false,
            // A clash is handed out as a file with conflict markers.
            None => true,
        };
        if is_file && !files.contains(&relative) && !is_ignored(&ignores, &relative) {
            builder.set_or_remove(path, None);
        }
    }
    for (relative, contents) in files.iter() {
        let path = repo_path(relative)?;
        let mut reader = contents;
        let id = store
            .write_file(&path, &mut reader)
            .await
            .map_err(HistoryError::store)?;
        builder.set_or_remove(
            path,
            Some(TreeValue::File {
                id,
                executable: executable.contains(relative),
                copy_id: CopyId::placeholder(),
            }),
        );
    }
    builder.write_tree().await.map_err(HistoryError::store)
}

/// Whether `left` and `right` hold the same thing at `path`.
pub(crate) async fn same_value(
    left: &MergedTree,
    right: &MergedTree,
    path: &RelativePath,
) -> Result<bool, HistoryError> {
    let path = repo_path(path)?;
    let left = left.path_value(&path).await.map_err(HistoryError::store)?;
    let right = right.path_value(&path).await.map_err(HistoryError::store)?;
    Ok(left == right)
}

/// Refuses a change from `base` to `changed` that touches the project's
/// settings in `.stave/`, which only the person using Stave may change
/// (an agent setting `merge_policy = "auto"` would raise its own rights).
pub(crate) async fn refuse_settings_changes(
    base: &MergedTree,
    changed: &MergedTree,
) -> Result<(), HistoryError> {
    let mut differences = base.diff(changed, &EverythingMatcher);
    while let Some(entry) = differences.next().await {
        let Ok(relative) = RelativePath::new(Path::new(entry.path.as_internal_file_string()))
        else {
            continue;
        };
        if relative.is_settings() {
            return Err(HistoryError::SettingsProtected { path: relative });
        }
    }
    Ok(())
}

/// A tree's files, with clashing files written out with snapshot-style
/// conflict markers (every side in full), and the list of those files.
/// Symbolic links and submodules are not project files and are left out,
/// and so is any path that is not a [`RelativePath`] (inside `.git` or
/// `.stave/history`, or not text), so nothing handed out can be written
/// into a repository's own folder.
pub(crate) async fn files_of(
    tree: &MergedTree,
) -> Result<(ProjectFiles, Vec<RelativePath>), HistoryError> {
    let store = tree.store();
    let options = ConflictMaterializeOptions {
        marker_style: ConflictMarkerStyle::Snapshot,
        marker_len: None,
        merge: store.merge_options().clone(),
    };
    let mut files = ProjectFiles::new();
    let mut conflicted = Vec::new();
    let mut entries = tree.entries();
    while let Some((path, value)) = entries.next().await {
        let value = value.map_err(HistoryError::store)?;
        let Ok(relative) = RelativePath::new(Path::new(path.as_internal_file_string())) else {
            continue;
        };
        let materialized = materialize_tree_value(store, &path, value, tree.labels())
            .await
            .map_err(HistoryError::store)?;
        match materialized {
            MaterializedTreeValue::File(mut file) => {
                let contents = file.read_all(&path).await.map_err(HistoryError::store)?;
                files.insert(relative, contents);
            }
            MaterializedTreeValue::FileConflict(conflict) => {
                let contents = materialize_merge_result_to_bytes(
                    &conflict.contents,
                    &conflict.labels,
                    &options,
                );
                files.insert(relative.clone(), contents.to_vec());
                conflicted.push(relative);
            }
            MaterializedTreeValue::AccessDenied(error) => return Err(HistoryError::Store(error)),
            MaterializedTreeValue::Absent
            | MaterializedTreeValue::Symlink { .. }
            | MaterializedTreeValue::OtherConflict { .. }
            | MaterializedTreeValue::GitSubmodule(_)
            | MaterializedTreeValue::Tree(_) => {}
        }
    }
    Ok((files, conflicted))
}

/// The project files of a tree whose values clash (paths that are not
/// project files, as in [`files_of`], are left out).
pub(crate) async fn conflicted_paths(tree: &MergedTree) -> Result<Vec<RelativePath>, HistoryError> {
    let mut paths = Vec::new();
    let mut conflicts = tree.conflicts();
    while let Some((path, value)) = conflicts.next().await {
        value.map_err(HistoryError::store)?;
        if let Ok(relative) = RelativePath::new(Path::new(path.as_internal_file_string())) {
            paths.push(relative);
        }
    }
    Ok(paths)
}

/// Three-way merge: the change from `base` to `ours` and the change from
/// `base` to `theirs`, together. Clashes stay recorded in the tree.
pub(crate) async fn merge_trees(
    base: MergedTree,
    ours: MergedTree,
    theirs: MergedTree,
) -> Result<MergedTree, HistoryError> {
    MergedTree::merge(Merge::from_vec(vec![
        (ours, "yours".to_owned()),
        (base, "before".to_owned()),
        (theirs, "theirs".to_owned()),
    ]))
    .await
    .map_err(HistoryError::store)
}

/// Records a revision on `parent` holding `tree`, made by `actor`.
pub(crate) async fn record(
    repo: &mut MutableRepo,
    parent: &CommitId,
    tree: MergedTree,
    description: &Description,
    actor: &Actor,
) -> Result<Commit, HistoryError> {
    let signature = signature(actor);
    repo.new_commit(vec![parent.clone()], tree)
        .set_description(description.to_text())
        .set_author(signature.clone())
        .set_committer(signature)
        .write()
        .await
        .map_err(HistoryError::store)
}

/// Points the bookmark `name` at `id`, or removes it.
pub(crate) fn set_bookmark(repo: &mut MutableRepo, name: &str, id: Option<&CommitId>) {
    let target = id.map_or_else(RefTarget::absent, target);
    repo.set_local_bookmark_target(RefNameBuf::from(name).as_ref(), target);
}

/// The signature recorded for `actor` now.
pub(crate) fn signature(actor: &Actor) -> Signature {
    Signature {
        name: actor.name.clone(),
        email: format!("{}@stave.invalid", actor.id),
        timestamp: Timestamp::now(),
    }
}

/// The hex id of a commit.
pub(crate) fn hex(id: &CommitId) -> String {
    id.hex()
}

/// A commit's time as `YYYY-MM-DD HH:MM`, in the time zone it was made in.
pub(crate) fn format_time(timestamp: &Timestamp) -> String {
    let MillisSinceEpoch(millis) = timestamp.timestamp;
    let local_seconds = millis
        .div_euclid(1000)
        .saturating_add(i64::from(timestamp.tz_offset).saturating_mul(60));
    let days = local_seconds.div_euclid(86_400);
    let seconds_of_day = local_seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}",
        seconds_of_day / 3600,
        (seconds_of_day % 3600) / 60
    )
}

/// The calendar date `days` after 1970-01-01 (Howard Hinnant's algorithm).
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let shifted = days.saturating_add(719_468);
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_read_as_calendar_dates() {
        let timestamp = Timestamp {
            timestamp: MillisSinceEpoch(1_795_000_000_000),
            tz_offset: 60,
        };
        // 1795000000 s is 2026-11-18 11:06:40 UTC; an hour ahead locally.
        assert_eq!(format_time(&timestamp), "2026-11-18 12:06");
        let epoch = Timestamp {
            timestamp: MillisSinceEpoch(0),
            tz_offset: 0,
        };
        assert_eq!(format_time(&epoch), "1970-01-01 00:00");
    }
}
