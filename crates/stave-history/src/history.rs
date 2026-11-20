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
use jj_lib::object_id::ObjectId as _;
use jj_lib::ref_name::RefNameBuf;
use jj_lib::repo::{ReadonlyRepo, Repo as _, RepoLoader};
use jj_lib::settings::UserSettings;
use jj_lib::signing_factory::signer_from_settings;

use crate::actor::{Actor, ActorKind, Description};
use crate::error::HistoryError;
use crate::folder::{self, HISTORY_DIRECTORY, VersionsStatus};
use crate::paths::ProjectFiles;
use crate::store::{self, ACTION_ATTRIBUTE, MAIN};
use crate::{RevisionId, Version};

/// One project's versions, opened from its folder.
///
/// Every method reads the repository as it is on disk at that moment, so
/// several handles (in one process or several) can work on one project; the
/// steps that move main take the lock file `.stave/history/main.lock`.
pub struct History {
    folder: PathBuf,
    settings: UserSettings,
    loader: RepoLoader,
}

impl std::fmt::Debug for History {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("History")
            .field("folder", &self.folder)
            .finish_non_exhaustive()
    }
}

fn block_on<T>(future: impl Future<Output = T>) -> T {
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
    /// main starts at the commit the folder is on. Nothing else is written
    /// to the folder.
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
        let made_stave_directory = !folder.join(".stave").exists();
        let result = Self::make(&folder, &history_directory);
        if result.is_err() {
            // Leave the folder as it was. Failing to tidy up is not worth
            // hiding the error that caused it.
            let tidy = if made_stave_directory {
                folder.join(".stave")
            } else {
                history_directory
            };
            let _ignored = std::fs::remove_dir_all(tidy);
        }
        result
    }

    fn make(folder: &Path, history_directory: &Path) -> Result<Self, HistoryError> {
        let repo_directory = history_directory.join("repo");
        std::fs::create_dir_all(&repo_directory)
            .map_err(|error| HistoryError::io(&repo_directory, error))?;
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
            &repo_directory,
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
        Self::open(folder)
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
        let loader = RepoLoader::init_from_file_system(
            &settings,
            &folder.join(HISTORY_DIRECTORY).join("repo"),
            &default_backend_factories(),
        )
        .map_err(HistoryError::store)?;
        Ok(Self {
            folder,
            settings,
            loader,
        })
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
            Ok(RevisionId(store::hex(&store::main_id(&repo))))
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
            let mut next = store::main_id(&repo);
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

    /// The repository as it is on disk now.
    async fn head(&self) -> Result<Arc<ReadonlyRepo>, HistoryError> {
        self.loader
            .load_at_head(None)
            .await
            .map_err(HistoryError::store)
    }

    /// The commit id for a revision id, checked to exist.
    fn resolve(
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
