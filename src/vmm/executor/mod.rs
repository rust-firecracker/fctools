use std::{
    collections::HashMap,
    future::Future,
    path::{Path, PathBuf},
    sync::Arc,
};

#[cfg(feature = "jailed-vmm-executor")]
use jailed::JailRenamerError;
#[cfg(feature = "unrestricted-vmm-executor")]
use tokio::task::JoinSet;
use tokio::{process::Child, task::JoinError};

use crate::{
    fs_backend::{FsBackend, FsBackendError},
    process_spawner::ProcessSpawner,
};

use super::{
    installation::VmmInstallation,
    ownership::{downgrade_owner, upgrade_owner, ChangeOwnerError, VmmOwnershipModel},
};

#[cfg(feature = "either-vmm-executor")]
#[cfg_attr(docsrs, doc(cfg(feature = "either-vmm-executor")))]
pub mod either;
#[cfg(feature = "jailed-vmm-executor")]
#[cfg_attr(docsrs, doc(cfg(feature = "jailed-vmm-executor")))]
pub mod jailed;
#[cfg(feature = "unrestricted-vmm-executor")]
#[cfg_attr(docsrs, doc(cfg(feature = "unrestricted-vmm-executor")))]
pub mod unrestricted;

pub mod process_handle;

/// An error emitted by a [VmmExecutor].
#[derive(Debug, thiserror::Error)]
pub enum VmmExecutorError {
    #[error("A non-FS I/O error occurred: `{0}")]
    IoError(std::io::Error),
    #[error("An I/O error emitted by an FS backend occurred: `{0}`")]
    FsBackendError(FsBackendError),
    #[error("An ownership change as part of an ownership upgrade or downgrade failed: `{0}")]
    ChangeOwnerError(ChangeOwnerError),
    #[error("Joining on a spawned async task failed: `{0}`")]
    TaskJoinFailed(JoinError),
    #[error("Spawning an auxiliary or primary process via the process spawner failed: `{0}`")]
    ProcessSpawnFailed(std::io::Error),
    #[error("A passed-in resource at the path `{0}` was expected but doesn't exist or isn't accessible")]
    ExpectedResourceMissing(PathBuf),
    #[error("A directory that is supposed to have a parent in the filesystem has none")]
    ExpectedDirectoryParentMissing,
    #[cfg(feature = "jailed-vmm-executor")]
    #[cfg_attr(docsrs, doc(cfg(feature = "jailed-vmm-executor")))]
    #[error("Invoking the jail renamer to produce an inner path failed: `{0}`")]
    JailRenamerFailed(JailRenamerError),
    #[error("Another error occurred: `{0}`")]
    Other(Box<dyn std::error::Error + Send>),
}

/// A [VmmExecutor] manages the environment of a VMM, correctly invoking its process, while
/// setting up and subsequently cleaning its environment. This allows modularity between different modes of VMM execution.
pub trait VmmExecutor: Send + Sync {
    /// Get the host location of the VMM socket, if one exists.
    fn get_socket_path(&self, installation: &VmmInstallation) -> Option<PathBuf>;

    /// Resolves an inner path into an outer path.
    fn inner_to_outer_path(&self, installation: &VmmInstallation, inner_path: &Path) -> PathBuf;

    // Returns a boolean determining whether this executor leaves any traces on the host filesystem after cleanup.
    fn is_traceless(&self) -> bool;

    /// Prepare all transient resources for the VMM invocation.
    fn prepare(
        &self,
        installation: &VmmInstallation,
        process_spawner: Arc<impl ProcessSpawner>,
        fs_backend: Arc<impl FsBackend>,
        outer_paths: Vec<PathBuf>,
        ownership_model: VmmOwnershipModel,
    ) -> impl Future<Output = Result<HashMap<PathBuf, PathBuf>, VmmExecutorError>> + Send;

    /// Invoke the VMM on the given [VmmInstallation] and return the spawned async [Child] process.
    fn invoke(
        &self,
        installation: &VmmInstallation,
        process_spawner: Arc<impl ProcessSpawner>,
        config_path: Option<PathBuf>,
        ownership_model: VmmOwnershipModel,
    ) -> impl Future<Output = Result<Child, VmmExecutorError>> + Send;

    /// Clean up all transient resources of the VMM invocation.
    fn cleanup(
        &self,
        installation: &VmmInstallation,
        process_spawner: Arc<impl ProcessSpawner>,
        fs_backend: Arc<impl FsBackend>,
        ownership_model: VmmOwnershipModel,
    ) -> impl Future<Output = Result<(), VmmExecutorError>> + Send;
}

async fn create_file_with_tree(
    fs_backend: Arc<impl FsBackend>,
    process_spawner: Arc<impl ProcessSpawner>,
    ownership_model: VmmOwnershipModel,
    path: PathBuf,
) -> Result<(), VmmExecutorError> {
    if let Some(parent_path) = path.parent() {
        fs_backend
            .create_dir_all(parent_path)
            .await
            .map_err(VmmExecutorError::FsBackendError)?;
    }

    fs_backend
        .create_file(&path)
        .await
        .map_err(VmmExecutorError::FsBackendError)?;

    downgrade_owner(&path, ownership_model, process_spawner.as_ref(), fs_backend.as_ref())
        .await
        .map_err(VmmExecutorError::ChangeOwnerError)?;

    Ok(())
}

#[cfg(feature = "unrestricted-vmm-executor")]
async fn join_on_set(mut join_set: JoinSet<Result<(), VmmExecutorError>>) -> Result<(), VmmExecutorError> {
    while let Some(result) = join_set.join_next().await {
        match result {
            Ok(result) => {
                if let Err(err) = result {
                    join_set.abort_all();
                    return Err(err);
                }
            }
            Err(err) => {
                join_set.abort_all();
                return Err(VmmExecutorError::TaskJoinFailed(err));
            }
        }
    }

    Ok(())
}
