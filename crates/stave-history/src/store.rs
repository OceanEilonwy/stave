//! The jj side: settings, trees built from project files and files read back
//! out of trees, commits and bookmarks. Nothing here is public; no jj type
//! leaves the crate.

use std::path::Path;
use std::sync::Arc;

use futures::StreamExt as _;
use jj_lib::backend::{CommitId, CopyId, MillisSinceEpoch, Signature, Timestamp, TreeValue};
use jj_lib::commit::Commit;
use jj_lib::config::StackedConfig;
use jj_lib::conflicts::{
    ConflictMarkerStyle, ConflictMaterializeOptions, MaterializedTreeValue,
    materialize_merge_result_to_bytes, materialize_tree_value,
};
use jj_lib::merge::Merge;
use jj_lib::merged_tree::MergedTree;
use jj_lib::merged_tree_builder::MergedTreeBuilder;
use jj_lib::object_id::ObjectId as _;
use jj_lib::op_store::RefTarget;
use jj_lib::ref_name::RefNameBuf;
use jj_lib::repo::{ReadonlyRepo, Repo as _};
use jj_lib::repo_path::RepoPathBuf;
use jj_lib::settings::UserSettings;
use jj_lib::store::Store;

use crate::actor::Actor;
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

/// The target for a bookmark pointing at `id`.
pub(crate) fn target(id: &CommitId) -> RefTarget {
    RefTarget::normal(id.clone())
}

/// Main's commit: the bookmark, or the root commit before the first save.
pub(crate) fn main_id(repo: &ReadonlyRepo) -> CommitId {
    bookmark(repo, MAIN).unwrap_or_else(|| repo.store().root_commit_id().clone())
}

/// The window's base: its bookmark, or main when it has none yet.
pub(crate) fn window_id(repo: &ReadonlyRepo) -> CommitId {
    bookmark(repo, WINDOW).unwrap_or_else(|| main_id(repo))
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

/// Writes `files` as a tree.
pub(crate) async fn tree_from_files(
    store: &Arc<Store>,
    files: &ProjectFiles,
) -> Result<MergedTree, HistoryError> {
    let mut builder = MergedTreeBuilder::new(store.empty_merged_tree());
    for (path, contents) in files.iter() {
        let path = repo_path(path)?;
        let mut reader = contents;
        let id = store
            .write_file(&path, &mut reader)
            .await
            .map_err(HistoryError::store)?;
        builder.set_or_remove(
            path,
            Some(TreeValue::File {
                id,
                executable: false,
                copy_id: CopyId::placeholder(),
            }),
        );
    }
    builder.write_tree().await.map_err(HistoryError::store)
}

/// A tree's files, with clashing files written out with snapshot-style
/// conflict markers (every side in full), and the list of those files.
/// Symbolic links and submodules are not project files and are left out.
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
        let relative = RelativePath::new(Path::new(path.as_internal_file_string()))?;
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

/// The files of a tree whose values clash.
pub(crate) async fn conflicted_paths(tree: &MergedTree) -> Result<Vec<RelativePath>, HistoryError> {
    let mut paths = Vec::new();
    let mut conflicts = tree.conflicts();
    while let Some((path, value)) = conflicts.next().await {
        value.map_err(HistoryError::store)?;
        paths.push(RelativePath::new(Path::new(
            path.as_internal_file_string(),
        ))?);
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
