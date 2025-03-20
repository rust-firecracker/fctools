use std::{future::Future, num::ParseIntError, path::PathBuf, process::ExitStatus, sync::Arc};

#[cfg(feature = "jailed-vmm-executor")]
use jailed::JailRenamerError;
use process_handle::ProcessHandle;

use crate::{process_spawner::ProcessSpawner, runtime::Runtime};

use super::{
    installation::VmmInstallation,
    ownership::{ChangeOwnerError, VmmOwnershipModel},
    resource::{system::ResourceSystemError, Resource},
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
    PidfdAllocationError(std::io::Error),
    ProcessWaitError(std::io::Error),
    FilesystemError(std::io::Error),
    ChangeOwnerError(ChangeOwnerError),
    ResourceSystemError(ResourceSystemError),
    ExpectedDirectoryParentMissing(PathBuf),
    TaskJoinFailed,
    ProcessSpawnFailed(std::io::Error),
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
            VmmExecutorError::ResourceSystemError(err) => {
                write!(f, "An error occurred within the resource system: {err}")
            }
            VmmExecutorError::ExpectedDirectoryParentMissing(path) => {
                write!(f, "A parent of a directory is missing: {}", path.display())
            }
            VmmExecutorError::TaskJoinFailed => write!(f, "Joining on an async task via the runtime failed"),
            VmmExecutorError::ProcessSpawnFailed(err) => write!(f, "Spawning a process failed: {err}"),
            #[cfg(feature = "jailed-vmm-executor")]
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

    /// Transform a given local resource path to an effective resource path.
    fn local_to_effective_path(&self, installation: &VmmInstallation, local_path: PathBuf) -> PathBuf;

    /// Prepare all transient resources for the VMM invocation.
    fn prepare<S: ProcessSpawner, R: Runtime>(
        &self,
        context: VmmExecutorContext<S, R>,
    ) -> impl Future<Output = Result<(), VmmExecutorError>> + Send;

    /// Invoke the VMM on the given [VmmInstallation] and return the [ProcessHandle] that performs a connection to
    /// the created process, regardless of it possibly being not a child and rather having been unshare()-d into
    /// a separate PID namespace.
    fn invoke<S: ProcessSpawner, R: Runtime>(
        &self,
        context: VmmExecutorContext<S, R>,
        config_path: Option<PathBuf>,
    ) -> impl Future<Output = Result<ProcessHandle<R>, VmmExecutorError>> + Send;

    /// Clean up all transient resources of the VMM invocation.
    fn cleanup<S: ProcessSpawner, R: Runtime>(
        &self,
        context: VmmExecutorContext<S, R>,
    ) -> impl Future<Output = Result<(), VmmExecutorError>> + Send;
}

/// A [VmmExecutorContext] encapsulates the data that a [VmmExecutor] prepare/invoke/cleanup invocation needs in
/// order to function. Creating a [VmmExecutorContext] is mainly simple, except for needing to pass in an exclusive
/// mutable reference to a [VmmResourceManager] tied to the 'r lifetime.
pub struct VmmExecutorContext<S: ProcessSpawner, R: Runtime> {
    pub installation: Arc<VmmInstallation>,
    pub process_spawner: S,
    pub runtime: R,
    pub ownership_model: VmmOwnershipModel,
    pub resources: Vec<Resource>,
}

#[cfg(any(feature = "unrestricted-vmm-executor", feature = "jailed-vmm-executor"))]
#[inline]
fn expand_context<S: ProcessSpawner, R: Runtime>(
    arguments: &super::arguments::VmmArguments,
    context: &mut VmmExecutorContext<S, R>,
) {
    if let Some(logs) = arguments.logs.clone() {
        context.resources.push(logs);
    }

    if let Some(metrics) = arguments.metrics.clone() {
        context.resources.push(metrics);
    }
}
