//! The folder's own git branch, when the folder is the root of a git
//! repository: main is that branch, so the person's git tools keep working.
//!
//! Main stays a jj bookmark (`main`) inside the repository, and the branch
//! the folder was on when versions were turned on (recorded in
//! `.stave/history/branch`) follows it: every step that moves main moves the
//! branch too, and a commit made to the branch with git is taken in as main
//! before the next such step. The repository's view keeps the branch's
//! position as Stave last left it, so a move made with git shows as a
//! difference from it.

use std::path::Path;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use jj_lib::backend::CommitId;
use jj_lib::object_id::ObjectId as _;
use jj_lib::op_store::RefTarget;
use jj_lib::ref_name::GitRefNameBuf;
use jj_lib::repo::{MutableRepo, ReadonlyRepo, Repo as _};

use crate::actor::{Actor, ActorKind};
use crate::error::HistoryError;
use crate::history::History;
use crate::store::{self, MAIN, MOVED_MAIN_ATTRIBUTE};

/// The file in `.stave/history` naming the branch main follows.
const BRANCH_FILE: &str = "branch";

/// The branch main follows (`refs/heads/main`), for a folder whose history
/// is in `history_directory`, if it follows one.
pub(crate) fn read(history_directory: &Path) -> Option<GitRefNameBuf> {
    let text = std::fs::read_to_string(history_directory.join(BRANCH_FILE)).ok()?;
    let name = text.trim();
    name.starts_with("refs/heads/")
        .then(|| GitRefNameBuf::from(name))
}

/// Records the branch git's `HEAD` points at as the one main follows, and
/// returns it. A detached `HEAD` names no branch, so main follows none.
pub(crate) fn record(
    history_directory: &Path,
    git: &gix::Repository,
) -> Result<Option<GitRefNameBuf>, HistoryError> {
    let Some(head) = git.head_name().map_err(HistoryError::store)? else {
        return Ok(None);
    };
    let name = head.as_bstr().to_string();
    if !name.starts_with("refs/heads/") {
        return Ok(None);
    }
    let file = history_directory.join(BRANCH_FILE);
    std::fs::write(&file, format!("{name}\n")).map_err(|error| HistoryError::io(&file, error))?;
    Ok(Some(GitRefNameBuf::from(name)))
}

/// Where `branch` is in git now, if it exists.
fn position(
    git: &gix::Repository,
    branch: &GitRefNameBuf,
) -> Result<Option<CommitId>, HistoryError> {
    let Some(reference) = git
        .try_find_reference(branch.as_str())
        .map_err(HistoryError::store)?
    else {
        return Ok(None);
    };
    let id = reference
        .into_fully_peeled_id()
        .map_err(HistoryError::store)?;
    Ok(Some(CommitId::from_bytes(id.as_bytes())))
}

/// Takes in a commit made to `branch` with git since Stave last moved it:
/// main moves to it. Returns the repository after, or `None` when the
/// branch is where Stave left it (or was deleted, which leaves main alone).
/// The caller holds the main lock.
pub(crate) async fn import(
    repo: &Arc<ReadonlyRepo>,
    branch: &GitRefNameBuf,
) -> Result<Option<Arc<ReadonlyRepo>>, HistoryError> {
    let git = jj_lib::git::get_git_repo(repo.store()).map_err(HistoryError::store)?;
    let Some(now) = position(&git, branch)? else {
        return Ok(None);
    };
    if repo.view().get_git_ref(branch.as_ref()).as_normal() == Some(&now) {
        return Ok(None);
    }
    let has_commit = repo
        .index()
        .has_id(&now)
        .await
        .map_err(HistoryError::store)?;
    if !has_commit {
        jj_lib::git::get_git_backend(repo.store())
            .map_err(HistoryError::store)?
            .import_head_commits([&now])
            .map_err(HistoryError::store)?;
    }
    let commit = store::commit(repo.store(), &now).await?;
    // The person committed with git: their change, as undo sees it.
    let committer = Actor {
        kind: ActorKind::User,
        name: commit.author().name.clone(),
        id: "git".to_owned(),
    };
    let mut transaction = History::transaction(repo, Some(&committer), "git commit");
    transaction.set_attribute(MOVED_MAIN_ATTRIBUTE.to_owned(), "true".to_owned());
    transaction
        .repo_mut()
        .add_head(&commit)
        .await
        .map_err(HistoryError::store)?;
    store::set_bookmark(transaction.repo_mut(), MAIN, Some(&now));
    transaction
        .repo_mut()
        .set_git_ref_target(branch.as_ref(), RefTarget::normal(now));
    let repo = transaction
        .commit(format!("take in git commits on {}", branch.as_str()))
        .await
        .map_err(HistoryError::store)?;
    Ok(Some(repo))
}

/// Remembers `id` as where `branch` is, without moving it: for turning
/// versions on, when main starts where the branch is.
pub(crate) fn remember(repo: &mut MutableRepo, branch: &GitRefNameBuf, id: &CommitId) {
    repo.set_git_ref_target(branch.as_ref(), RefTarget::normal(id.clone()));
}

/// Moves `branch` to main as it is in `repo`, a transaction about to be
/// committed; `before` is the repository the transaction started from.
/// When git's `HEAD` is on the branch, git's index is set to main's files
/// too, so `git status` shows the folder against the new main. While
/// main holds clashes the branch is left where it is.
///
/// # Errors
///
/// [`HistoryError::BranchMoved`] when the branch moved with git since
/// `before` (the transaction should then not be committed).
pub(crate) fn export(
    before: &ReadonlyRepo,
    repo: &mut MutableRepo,
    branch: &GitRefNameBuf,
) -> Result<(), HistoryError> {
    let Some(main) = store::bookmark_in(repo, MAIN) else {
        // Main is back before the first revision; git has no such commit.
        return Ok(());
    };
    // A revision with clashes has no tree git's tools could show: the
    // branch stays where it is (and is remembered there) until main is
    // clean again, and the next clean move of main moves it.
    let main_commit = repo
        .store()
        .get_commit(&main)
        .map_err(HistoryError::store)?;
    if main_commit.tree_ids().as_resolved().is_none() {
        return Ok(());
    }
    let known = before
        .view()
        .get_git_ref(branch.as_ref())
        .as_normal()
        .cloned();
    if known.as_ref() != Some(&main) {
        let git = jj_lib::git::get_git_repo(repo.store()).map_err(HistoryError::store)?;
        let moved = || HistoryError::BranchMoved {
            branch: branch.as_str().trim_start_matches("refs/heads/").to_owned(),
        };
        let expected = match &known {
            Some(id) => gix::refs::transaction::PreviousValue::MustExistAndMatch(
                gix::refs::Target::Object(object_id(id)?),
            ),
            None => gix::refs::transaction::PreviousValue::MustNotExist,
        };
        let name: gix::refs::FullName = branch.as_str().try_into().map_err(HistoryError::store)?;
        let edit = gix::refs::transaction::RefEdit {
            change: gix::refs::transaction::Change::Update {
                log: gix::refs::transaction::LogChange {
                    message: "stave: move main".into(),
                    ..Default::default()
                },
                expected,
                new: gix::refs::Target::Object(object_id(&main)?),
            },
            name,
            deref: false,
        };
        let seconds = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| since.as_secs());
        let time = format!("{seconds} +0000");
        let committer = gix::actor::SignatureRef {
            name: "Stave".into(),
            email: "stave@stave.invalid".into(),
            time: &time,
        };
        git.edit_references_as([edit], Some(committer))
            .map_err(|_error| moved())?;
        reset_index(&git, branch, repo, &main)?;
    }
    remember(repo, branch, &main);
    Ok(())
}

/// Sets git's index to the files of `main` when `HEAD` is on `branch`.
fn reset_index(
    git: &gix::Repository,
    branch: &GitRefNameBuf,
    repo: &MutableRepo,
    main: &CommitId,
) -> Result<(), HistoryError> {
    let on_branch = git
        .head_name()
        .map_err(HistoryError::store)?
        .is_some_and(|head| head.as_bstr() == branch.as_str().as_bytes());
    if !on_branch {
        return Ok(());
    }
    let commit = repo.store().get_commit(main).map_err(HistoryError::store)?;
    // A revision with clashes has no single tree git could use.
    let Some(tree) = commit.tree_ids().as_resolved() else {
        return Ok(());
    };
    let mut index = if tree == repo.store().empty_tree_id() {
        gix::index::File::from_state(gix::index::State::new(git.object_hash()), git.index_path())
    } else {
        git.index_from_tree(&object_id_of(tree.as_bytes())?)
            .map_err(HistoryError::store)?
    };
    index
        .write(gix::index::write::Options::default())
        .map_err(HistoryError::store)
}

/// A commit id as git's object id.
fn object_id(id: &CommitId) -> Result<gix::ObjectId, HistoryError> {
    object_id_of(id.as_bytes())
}

fn object_id_of(bytes: &[u8]) -> Result<gix::ObjectId, HistoryError> {
    gix::hash::oid::try_from_bytes(bytes)
        .map(ToOwned::to_owned)
        .map_err(HistoryError::store)
}
