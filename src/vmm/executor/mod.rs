use std::{future::Future, path::PathBuf, process::ExitStatus};

#[cfg(feature = "jailed-vmm-executor")]
use jailed::VirtualPathResolverError;
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

/// An error that can be emitted by a [VmmExecutor] implementation.
#[derive(Debug)]
pub enum VmmExecutorError {
    /// An I/O error occurred while allocating a Linux pidfd for a process.
    PidfdAllocationError(std::io::Error),
    /// An I/O error occurred while spawning a process via a [ProcessSpawner].
    ProcessSpawnFailed(std::io::Error),
    /// An I/O error occurred while waiting for the exit of a child process spawned by a [ProcessSpawner].
    ProcessWaitError(std::io::Error),
    /// A process exited with an unsuccessful non-zero [ExitStatus].
    ProcessExitedWithNonZeroStatus(ExitStatus),
    /// An I/O error occurred while interacting with the filesystem.
    FilesystemError(std::io::Error),
    /// A [ChangeOwnerError] occurred while performing necessary ownership changes.
    ChangeOwnerError(ChangeOwnerError),
    /// A [ResourceSystemError] occurred while scheduling [Resource] initialization and/or disposal.
    ResourceSystemError(ResourceSystemError),
    /// The given owned [PathBuf] was expected to have a directory parent, yet it was located at the root
    /// of the filesystem.
    ExpectedDirectoryParentMissing(PathBuf),
    /// A [VirtualPathResolverError] occurred.
    #[cfg(feature = "jailed-vmm-executor")]
    #[cfg_attr(docsrs, doc(cfg(feature = "jailed-vmm-executor")))]
    VirtualPathResolverError(VirtualPathResolverError),
    /// Another type of error occurred within the [VmmExecutor] implementation's code. This error variant is
    /// reserved for custom [VmmExecutor] implementations and isn't used by the built-in ones.
    Other(Box<dyn std::error::Error + Send + Sync>),
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
            VmmExecutorError::ProcessSpawnFailed(err) => write!(f, "Spawning a process failed: {err}"),
            #[cfg(feature = "jailed-vmm-executor")]
            VmmExecutorError::VirtualPathResolverError(err) => {
                write!(f, "Invoking the virtual path resolver failed: {err}")
            }
            VmmExecutorError::ProcessExitedWithNonZeroStatus(exit_status) => {
                write!(f, "A watched process exited with a non-zero exit status: {exit_status}")
            }
            VmmExecutorError::Other(err) => write!(f, "Another error occurred: {err}"),
        }
    }
}

/// A [VmmExecutor] manages the environment of a VMM, correctly invoking its process, as well as
/// setting up and subsequently cleaning its environment. This allows modularity between different modes of VMM execution.
pub trait VmmExecutor: Send + Sync {
    /// Get the host location of the VMM socket, if one exists.
    fn get_socket_path(&self, installation: &VmmInstallation) -> Option<PathBuf>;

    /// Resolve an effective path of a resource from its virtual path.
    fn resolve_effective_path(&self, installation: &VmmInstallation, local_path: PathBuf) -> PathBuf;

    /// Prepare all transient resources for the VMM invocation. It is assumed that an implementation of this function
    /// appropriately schedules the initialization of all [Resource]s inside the given [VmmExecutorContext] to effective
    /// and virtual paths according to the executor's discretion. It will therefore be necessary to manually synchronize
    /// the resource system that the [Resource]s come from in order to ensure the scheduled operations complete.
    /// This synchronization is automatically performed by VMM processes and VMs.
    fn prepare<S: ProcessSpawner, R: Runtime>(
        &self,
        context: VmmExecutorContext<'_, S, R>,
    ) -> impl Future<Output = Result<(), VmmExecutorError>> + Send;

    /// Invoke the VMM on the given [VmmInstallation] and return the [ProcessHandle] that performs a connection to
    /// the created process, regardless of it possibly not being a child and rather having been unshare()-d into
    /// a separate PID namespace.
    fn invoke<S: ProcessSpawner, R: Runtime>(
        &self,
        context: VmmExecutorContext<'_, S, R>,
        config_path: Option<PathBuf>,
    ) -> impl Future<Output = Result<ProcessHandle<R>, VmmExecutorError>> + Send;

    /// Clean up all transient resources of the VMM invocation. It is assumed that an implementation of this function
    /// appropriately schedules the disposal of all [Resource]s inside the given [VmmExecutorContext], except for cases
    /// when disposal is redundant. It will therefore be necessary to manually synchronize the resource system that
    /// the [Resource]s come from in order to ensure the scheduled operations complete. This manual synchronization is
    /// automatically performed by VMM processes and VMs.
    fn cleanup<S: ProcessSpawner, R: Runtime>(
        &self,
        context: VmmExecutorContext<'_, S, R>,
    ) -> impl Future<Output = Result<(), VmmExecutorError>> + Send;
}

/// A [VmmExecutorContext] encapsulates the data that a [VmmExecutor] prepare/invoke/cleanup invocation needs in
/// order to function. It is tied to a lifetime representing that of all [Resource]s being operated with and to
/// a generic [ProcessSpawner] as well as [Runtime].
#[derive(Debug, Clone)]
pub struct VmmExecutorContext<'r, S: ProcessSpawner, R: Runtime> {
    /// The [VmmInstallation] the invocation is tied to.
    pub installation: VmmInstallation,
    /// A [ProcessSpawner] that the [VmmExecutorContext] is generic over.
    pub process_spawner: S,
    /// A [Runtime] that the [VmmExecutorContext] is generic over.
    pub runtime: R,
    /// A [VmmOwnershipModel] to use for ownership operations within the executor.
    pub ownership_model: VmmOwnershipModel,
    /// A shared slice of all [Resource]s to consider for initialization and disposal.
    pub resources: &'r [Resource],
}
