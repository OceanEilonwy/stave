//! [`History`]: one project's repository, opened from its folder.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use jj_lib::backend::{Backend, BackendInitError, CommitId};
use jj_lib::default_backend_factories::{
    default_backend_factories, default_index_store_initializer, default_op_heads_store_initializer,
    default_op_store_initializer, default_submodule_store_initializer,
    default_workspace_store_initializer,
};
use jj_lib::file_util::relative_path;
use jj_lib::git_backend::GitBackend;
use jj_lib::lock::FileLock;
use jj_lib::object_id::ObjectId as _;
use jj_lib::ref_name::{GitRefNameBuf, RefNameBuf};
use jj_lib::repo::{ReadonlyRepo, Repo as _, RepoLoader};
use jj_lib::settings::UserSettings;
use jj_lib::signing_factory::signer_from_settings;
use jj_lib::transaction::Transaction;

use crate::actor::{Actor, ActorKind, Description};
use crate::branch;
use crate::error::HistoryError;
use crate::folder::{self, HISTORY_DIRECTORY, VersionsStatus};
use crate::paths::ProjectFiles;
use crate::store::{self, ACTION_ATTRIBUTE, ACTOR_ATTRIBUTE, MAIN, WINDOW};
use crate::{RevisionId, Version};

/// One project's versions, opened from its folder.
///
/// Every method reads the repository as it is on disk at that moment, so
/// several handles (in one process or several) can work on one project; the
/// steps that move main take the lock file `.stave/history/main.lock`.
pub struct History {
    pub(crate) folder: PathBuf,
    pub(crate) settings: UserSettings,
    pub(crate) loader: RepoLoader,
    /// The folder's git branch main follows, when the folder is the root of
    /// a git repository whose `HEAD` was on a branch.
    pub(crate) branch: Option<GitRefNameBuf>,
}

impl std::fmt::Debug for History {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("History")
            .field("folder", &self.folder)
            .finish_non_exhaustive()
    }
}

pub(crate) fn block_on<T>(future: impl Future<Output = T>) -> T {
    pollster::block_on(future)
}

impl History {
    /// Whether versions are on for `folder`, off, or can't be turned on
    /// because the folder sits inside a bigger repository.
    ///
    /// # Errors
    ///
    /// [`HistoryError::Io`] when the folder can't be found.
    pub fn status(folder: &Path) -> Result<VersionsStatus, HistoryError> {
        Ok(folder::status(&folder::canonical(folder)?))
    }

    /// Turns versions on for `folder` and opens them; when they are on
    /// already, just opens them.
    ///
    /// A folder that isn't a repository gets `.stave/history/` with the
    /// repository's git data inside it. A folder that is the root of a git
    /// repository keeps its `.git`: the repository uses it as its store and
    /// main is the branch the folder is on. Main starts where the branch
    /// is; every step that moves main moves the branch (and, while git's
    /// `HEAD` is on it, git's index) with it, and commits made to the
    /// branch with git become main's latest when versions are next opened
    /// or main next moves. Nothing else is written to the folder.
    ///
    /// # Errors
    ///
    /// [`HistoryError::Nested`] when the folder sits inside a bigger
    /// repository (nothing is written); [`HistoryError::Io`] or
    /// [`HistoryError::Store`] when the repository can't be made.
    pub fn start(folder: &Path) -> Result<Self, HistoryError> {
        let folder = folder::canonical(folder)?;
        match folder::status(&folder) {
            VersionsStatus::On => return Self::open(&folder),
            VersionsStatus::Nested { repository_root } => {
                return Err(HistoryError::Nested { repository_root });
            }
            VersionsStatus::Off => {}
        }
        let history_directory = folder.join(HISTORY_DIRECTORY);
        std::fs::create_dir_all(&history_directory)
            .map_err(|error| HistoryError::io(&history_directory, error))?;
        // One start at a time (the window and an agent may turn versions on
        // at once); whoever comes second finds them on.
        let _lock =
            FileLock::lock(history_directory.join("start.lock")).map_err(HistoryError::store)?;
        if folder::status(&folder) == VersionsStatus::On {
            return Self::open(&folder);
        }
        // The repository is made beside its place and moved there whole, so
        // versions read as on only once it is complete. It sits as deep as
        // its place, so the paths it keeps relative still hold.
        let making = history_directory.join(format!("repo.making-{}", std::process::id()));
        let _ignored = std::fs::remove_dir_all(&making);
        let made = Self::make(&folder, &history_directory, &making).and_then(|()| {
            let repo_directory = history_directory.join("repo");
            std::fs::rename(&making, &repo_directory)
                .map_err(|error| HistoryError::io(&repo_directory, error))
        });
        if let Err(error) = made {
            // Only what this start made goes; failing to tidy up is not
            // worth hiding the error that caused it.
            let _ignored = std::fs::remove_dir_all(&making);
            return Err(error);
        }
        Self::open(&folder)
    }

    /// Makes the repository in `repo_directory`, for `history_directory`.
    fn make(
        folder: &Path,
        history_directory: &Path,
        repo_directory: &Path,
    ) -> Result<(), HistoryError> {
        std::fs::create_dir_all(repo_directory)
            .map_err(|error| HistoryError::io(repo_directory, error))?;
        // Keep the repository out of any git repository around the files,
        // the folder's own included.
        let ignore = history_directory.join(".gitignore");
        std::fs::write(&ignore, "*\n").map_err(|error| HistoryError::io(&ignore, error))?;

        let git_directory = folder.join(".git");
        let external_git = git_directory.is_dir();
        let settings = store::settings()?;
        let initializer = |settings: &UserSettings,
                           store_path: &Path|
         -> Result<Box<dyn Backend>, BackendInitError> {
            let git_settings = settings
                .git_backend_settings()
                .map_err(BackendInitError::new)?;
            let backend = if external_git {
                let relative = relative_path(store_path, &git_directory);
                GitBackend::init_external(&git_settings, store_path, &relative)?
            } else {
                GitBackend::init_internal(&git_settings, store_path, gix::hash::Kind::Sha1)?
            };
            Ok(Box::new(backend))
        };
        let signer = signer_from_settings(&settings).map_err(HistoryError::store)?;
        let repo = block_on(ReadonlyRepo::init(
            &settings,
            repo_directory,
            &initializer,
            signer,
            default_workspace_store_initializer(),
            default_op_store_initializer(),
            default_op_heads_store_initializer(),
            default_index_store_initializer(),
            default_submodule_store_initializer(),
        ))
        .map_err(HistoryError::store)?;

        if external_git {
            let git = jj_lib::git::get_git_repo(repo.store()).map_err(HistoryError::store)?;
            let branch = branch::record(history_directory, &git)?;
            block_on(async {
                let mut transaction = repo.start_transaction();
                let head = jj_lib::git::import_head_commit(transaction.repo_mut())
                    .await
                    .map_err(HistoryError::store)?;
                if let Some(head) = head {
                    transaction.repo_mut().set_local_bookmark_target(
                        RefNameBuf::from(MAIN).as_ref(),
                        store::target(head.id()),
                    );
                    // The folder holds the files of the commit it is on.
                    store::set_bookmark(transaction.repo_mut(), WINDOW, Some(head.id()));
                    if let Some(branch) = &branch {
                        branch::remember(transaction.repo_mut(), branch, head.id());
                    }
                }
                transaction
                    .set_attribute(ACTION_ATTRIBUTE.to_owned(), "turn on versions".to_owned());
                transaction
                    .commit("turn on versions from the folder's git history")
                    .await
                    .map_err(HistoryError::store)?;
                Ok::<(), HistoryError>(())
            })?;
        }
        Ok(())
    }

    /// Opens the versions of `folder`.
    ///
    /// # Errors
    ///
    /// [`HistoryError::Off`] when versions are not on for the folder;
    /// [`HistoryError::Store`] when the repository can't be read.
    pub fn open(folder: &Path) -> Result<Self, HistoryError> {
        let folder = folder::canonical(folder)?;
        if folder::status(&folder) != VersionsStatus::On {
            return Err(HistoryError::Off);
        }
        let settings = store::settings()?;
        let history_directory = folder.join(HISTORY_DIRECTORY);
        let loader = RepoLoader::init_from_file_system(
            &settings,
            &history_directory.join("repo"),
            &default_backend_factories(),
        )
        .map_err(HistoryError::store)?;
        let history = Self {
            folder,
            settings,
            loader,
            branch: branch::read(&history_directory),
        };
        if history.branch.is_some() {
            // Take in what was committed to the branch with git meanwhile.
            let _lock = history.lock_main()?;
            block_on(history.head_with_git())?;
        }
        Ok(history)
    }

    /// The project folder, absolute.
    pub fn folder(&self) -> &Path {
        &self.folder
    }

    /// Every file in the folder that counts: all but `.stave/history/`,
    /// `.git` and what `.gitignore` excludes. Symbolic links are skipped.
    ///
    /// # Errors
    ///
    /// [`HistoryError::Io`] when a file can't be read; [`HistoryError::Path`]
    /// when a file name is not text.
    pub fn read_folder(&self) -> Result<ProjectFiles, HistoryError> {
        folder::read(&self.folder)
    }

    /// The files of a revision. Files whose changes clash are written out
    /// with conflict markers, every side in full.
    ///
    /// # Errors
    ///
    /// [`HistoryError::UnknownRevision`] when there is no such revision.
    pub fn files_at(&self, revision: &RevisionId) -> Result<ProjectFiles, HistoryError> {
        block_on(async {
            let repo = self.head().await?;
            let id = self.resolve(&repo, revision)?;
            let commit = store::commit(repo.store(), &id).await?;
            let (files, _conflicted) = store::files_of(&commit.tree()).await?;
            Ok(files)
        })
    }

    /// Main's latest revision.
    ///
    /// # Errors
    ///
    /// [`HistoryError::Store`] when the repository can't be read.
    pub fn main(&self) -> Result<RevisionId, HistoryError> {
        block_on(async {
            let repo = self.head().await?;
            let main = store::main_id(&repo)?;
            Ok(RevisionId(store::hex(&main)))
        })
    }

    /// Main's revisions, newest first, at most `limit` of them.
    ///
    /// # Errors
    ///
    /// [`HistoryError::Store`] when the repository can't be read.
    pub fn versions(&self, limit: usize) -> Result<Vec<Version>, HistoryError> {
        block_on(async {
            let repo = self.head().await?;
            let root = repo.store().root_commit_id().clone();
            let mut next = store::main_id(&repo)?;
            let mut versions = Vec::new();
            while versions.len() < limit && next != root {
                let commit = store::commit(repo.store(), &next).await?;
                let description = Description::parse(commit.description());
                let actor = description.actor.unwrap_or_else(|| Actor {
                    kind: ActorKind::User,
                    name: commit.author().name.clone(),
                    id: commit.author().email.clone(),
                });
                versions.push(Version {
                    id: RevisionId(store::hex(commit.id())),
                    actor,
                    message: description.message,
                    time: store::format_time(&commit.committer().timestamp),
                });
                match commit.parent_ids().first() {
                    Some(parent) => next = parent.clone(),
                    None => break,
                }
            }
            Ok(versions)
        })
    }

    /// The repository as it is on disk now, with any commits made to the
    /// folder's git branch since Stave last moved it taken in as main. The
    /// caller holds the main lock.
    pub(crate) async fn head_with_git(&self) -> Result<Arc<ReadonlyRepo>, HistoryError> {
        let repo = self.head().await?;
        match &self.branch {
            Some(branch) => Ok(branch::import(&repo, branch).await?.unwrap_or(repo)),
            None => Ok(repo),
        }
    }

    /// Moves the folder's git branch (if main follows one) to main as it is
    /// in `transaction`, which started from `before`. Call it just before
    /// committing a transaction that moved main.
    pub(crate) fn export_main(
        &self,
        before: &ReadonlyRepo,
        transaction: &mut Transaction,
    ) -> Result<(), HistoryError> {
        match &self.branch {
            Some(branch) => branch::export(before, transaction.repo_mut(), branch),
            None => Ok(()),
        }
    }

    /// Takes the lock that guards moving main, waiting for it. It is let go
    /// when the returned value is dropped.
    pub(crate) fn lock_main(&self) -> Result<FileLock, HistoryError> {
        FileLock::lock(self.folder.join(HISTORY_DIRECTORY).join("main.lock"))
            .map_err(HistoryError::store)
    }

    /// Starts a transaction whose operation records `actor` and `action`.
    pub(crate) fn transaction(
        repo: &Arc<ReadonlyRepo>,
        actor: Option<&Actor>,
        action: &str,
    ) -> Transaction {
        let mut transaction = repo.start_transaction();
        if let Some(actor) = actor {
            transaction.set_attribute(ACTOR_ATTRIBUTE.to_owned(), actor.to_line());
        }
        transaction.set_attribute(ACTION_ATTRIBUTE.to_owned(), action.to_owned());
        transaction
    }

    /// The repository as it is on disk now.
    pub(crate) async fn head(&self) -> Result<Arc<ReadonlyRepo>, HistoryError> {
        self.loader
            .load_at_head(None)
            .await
            .map_err(HistoryError::store)
    }

    /// The commit id for a revision id, checked to exist.
    pub(crate) fn resolve(
        &self,
        repo: &ReadonlyRepo,
        revision: &RevisionId,
    ) -> Result<CommitId, HistoryError> {
        let unknown = || HistoryError::UnknownRevision(revision.clone());
        let id = CommitId::try_from_hex(revision.as_str()).ok_or_else(unknown)?;
        if id.as_bytes().len() != repo.store().commit_id_length() {
            return Err(unknown());
        }
        match repo.store().get_commit(&id) {
            Ok(_) => Ok(id),
            Err(_) => Err(unknown()),
        }
    }
}
