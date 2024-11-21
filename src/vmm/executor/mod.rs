use std::{
    collections::HashMap,
    future::Future,
    num::ParseIntError,
    path::{Path, PathBuf},
    process::ExitStatus,
    sync::Arc,
};

#[cfg(feature = "jailed-vmm-executor")]
use jailed::renamer::JailRenamerError;
use process_handle::ProcessHandle;

use crate::{
    process_spawner::ProcessSpawner,
    runtime::{Runtime, RuntimeFilesystem},
};

use super::{
    installation::VmmInstallation,
    ownership::{downgrade_owner, upgrade_owner, ChangeOwnerError, VmmOwnershipModel},
    with_c_path_ptr,
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
#[derive(Debug)]
pub enum VmmExecutorError {
    MkfifoError(std::io::Error),
    PidfdAllocationError(std::io::Error),
    ProcessWaitError(std::io::Error),
    FilesystemError(std::io::Error),
    ChangeOwnerError(ChangeOwnerError),
    TaskJoinFailed,
    ProcessSpawnFailed(std::io::Error),
    ExpectedResourceMissing(PathBuf),
    ExpectedDirectoryParentMissing,
    #[cfg(feature = "jailed-vmm-executor")]
    #[cfg_attr(docsrs, doc(cfg(feature = "jailed-vmm-executor")))]
    JailRenamerFailed(JailRenamerError),
    ProcessExitedWithIncorrectStatus(ExitStatus),
    ParseIntError(ParseIntError),
    Other(Box<dyn std::error::Error + Send>),
}

impl std::error::Error for VmmExecutorError {}

impl std::fmt::Display for VmmExecutorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VmmExecutorError::MkfifoError(err) => write!(f, "Making a FIFO pipe failed: {err}"),
            VmmExecutorError::PidfdAllocationError(err) => {
                write!(f, "Allocating a pidfd for a process handle failed: {err}")
            }
            VmmExecutorError::ProcessWaitError(err) => write!(f, "Waiting on a child process failed: {err}"),
            VmmExecutorError::FilesystemError(err) => {
                write!(f, "A filesystem operation backed by the runtime failed: {err}")
            }
            VmmExecutorError::ChangeOwnerError(err) => {
                write!(f, "An ownership change failed: {err}")
            }
            VmmExecutorError::TaskJoinFailed => write!(f, "Joining on an async task via the runtime failed"),
            VmmExecutorError::ProcessSpawnFailed(err) => write!(f, "Spawning a process failed: {err}"),
            VmmExecutorError::ExpectedResourceMissing(path) => {
                write!(
                    f,
                    "A resource at the path {} was expected but isn't available",
                    path.display()
                )
            }
            VmmExecutorError::ExpectedDirectoryParentMissing => {
                write!(f, "An expected directory in the filesystem doesn't have a parent")
            }
            VmmExecutorError::JailRenamerFailed(err) => {
                write!(f, "Invoking the jail renamer to produce an inner path failed: {err}")
            }
            VmmExecutorError::ProcessExitedWithIncorrectStatus(exit_status) => {
                write!(f, "A watched process exited with a non-zero exit status: {exit_status}")
            }
            VmmExecutorError::ParseIntError(err) => write!(f, "Parsing an integer from a string failed: {err}"),
            VmmExecutorError::Other(err) => write!(f, "Another error occurred: {err}"),
        }
    }
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
    fn prepare<R: Runtime>(
        &self,
        installation: &VmmInstallation,
        process_spawner: Arc<impl ProcessSpawner>,
        outer_paths: Vec<PathBuf>,
        ownership_model: VmmOwnershipModel,
    ) -> impl Future<Output = Result<HashMap<PathBuf, PathBuf>, VmmExecutorError>> + Send;

    /// Invoke the VMM on the given [VmmInstallation] and return the [ProcessHandle] that performs a connection to
    /// the created process, regardless of it possibly being not a child and rather having been unshare()-d into
    /// a separate PID namespace.
    fn invoke<R: Runtime>(
        &self,
        installation: &VmmInstallation,
        process_spawner: Arc<impl ProcessSpawner>,
        config_path: Option<PathBuf>,
        ownership_model: VmmOwnershipModel,
    ) -> impl Future<Output = Result<ProcessHandle<R::Process>, VmmExecutorError>> + Send;

    /// Clean up all transient resources of the VMM invocation.
    fn cleanup<R: Runtime>(
        &self,
        installation: &VmmInstallation,
        process_spawner: Arc<impl ProcessSpawner>,
        ownership_model: VmmOwnershipModel,
    ) -> impl Future<Output = Result<(), VmmExecutorError>> + Send;
}

async fn create_file_or_fifo<R: Runtime>(
    ownership_model: VmmOwnershipModel,
    path: PathBuf,
) -> Result<(), VmmExecutorError> {
    if let Some(parent_path) = path.parent() {
        R::Filesystem::create_dir_all(parent_path)
            .await
            .map_err(VmmExecutorError::FilesystemError)?;
    }

    if path.extension().and_then(|ext| ext.to_str()) == Some("fifo") {
        let ret = with_c_path_ptr(&path, |ptr| unsafe { libc::mkfifo(ptr, 0) });
        if ret != 0 {
            return Err(VmmExecutorError::MkfifoError(std::io::Error::last_os_error()));
        }
    } else {
        R::Filesystem::create_file(&path)
            .await
            .map_err(VmmExecutorError::FilesystemError)?;
    }

    downgrade_owner(&path, ownership_model).map_err(VmmExecutorError::ChangeOwnerError)?;

    Ok(())
}
