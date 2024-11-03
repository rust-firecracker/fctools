use std::{
    collections::HashMap,
    future::Future,
    num::ParseIntError,
    path::{Path, PathBuf},
    process::ExitStatus,
    sync::Arc,
};

#[cfg(feature = "jailed-vmm-executor")]
use jailed::JailRenamerError;
use nix::sys::stat::Mode;
use process_handle::ProcessHandle;
use tokio::task::JoinError;
#[cfg(feature = "unrestricted-vmm-executor")]
use tokio::task::JoinSet;

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
    #[error("Making a FIFO pipe failed: {0}")]
    MkfifoError(std::io::Error),
    #[error("Allocating a pidfd for a non-child process failed: {0}")]
    PidfdAllocationError(std::io::Error),
    #[error("Waiting on the exit of a child process failed: {0}")]
    ProcessWaitError(std::io::Error),
    #[error("An I/O error emitted by an FS backend occurred: {0}")]
    FsBackendError(FsBackendError),
    #[error("An ownership change as part of an ownership upgrade or downgrade failed: {0}")]
    ChangeOwnerError(ChangeOwnerError),
    #[error("Joining on a spawned async task failed: {0}")]
    TaskJoinFailed(JoinError),
    #[error("Spawning an auxiliary or primary process via the process spawner failed: {0}")]
    ProcessSpawnFailed(std::io::Error),
    #[error("A passed-in resource at the path {0} was expected but doesn't exist or isn't accessible")]
    ExpectedResourceMissing(PathBuf),
    #[error("A directory that is supposed to have a parent in the filesystem has none")]
    ExpectedDirectoryParentMissing,
    #[cfg(feature = "jailed-vmm-executor")]
    #[cfg_attr(docsrs, doc(cfg(feature = "jailed-vmm-executor")))]
    #[error("Invoking the jail renamer to produce an inner path failed: {0}")]
    JailRenamerFailed(JailRenamerError),
    #[error("The watched process exited with an unsuccessful exit status: {0}")]
    ProcessExitedWithIncorrectStatus(ExitStatus),
    #[error("Parsing an integer from a string failed: {0}")]
    ParseIntError(ParseIntError),
    #[error("Another error occurred: {0}")]
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

    /// Invoke the VMM on the given [VmmInstallation] and return the [ProcessHandle] that performs a connection to
    /// the created process, regardless of it possibly being not a child and rather having been unshare()-d into
    /// a separate PID namespace.
    fn invoke(
        &self,
        installation: &VmmInstallation,
        process_spawner: Arc<impl ProcessSpawner>,
        fs_backend: Arc<impl FsBackend>,
        config_path: Option<PathBuf>,
        ownership_model: VmmOwnershipModel,
    ) -> impl Future<Output = Result<ProcessHandle, VmmExecutorError>> + Send;

    /// Clean up all transient resources of the VMM invocation.
    fn cleanup(
        &self,
        installation: &VmmInstallation,
        process_spawner: Arc<impl ProcessSpawner>,
        fs_backend: Arc<impl FsBackend>,
        ownership_model: VmmOwnershipModel,
    ) -> impl Future<Output = Result<(), VmmExecutorError>> + Send;
}

async fn create_file_or_fifo(
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

    if path.extension().map(|ext| ext.to_str()).flatten() == Some("fifo") {
        if nix::unistd::mkfifo(&path, Mode::S_IROTH | Mode::S_IWOTH | Mode::S_IRUSR | Mode::S_IWUSR).is_err() {
            return Err(VmmExecutorError::MkfifoError(std::io::Error::last_os_error()));
        }
    } else {
        fs_backend
            .create_file(&path)
            .await
            .map_err(VmmExecutorError::FsBackendError)?;
    }

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
