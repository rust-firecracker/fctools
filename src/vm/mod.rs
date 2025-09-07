//! Provides managed and high-level VM APIs that are most suitable for production applications
//! that only need to concern themselves with the high-level details of a Firecracker VM.
//! These abstractions is built on the `vmm-core`, `vmm-executor` and `vmm-process` features.

use std::{path::PathBuf, process::ExitStatus, time::Duration};

use crate::{
    process_spawner::ProcessSpawner,
    runtime::{Runtime, util::RuntimeHyperExecutor},
    vmm::{
        executor::{VmmExecutor, process_handle::ProcessHandlePipes},
        installation::VmmInstallation,
        ownership::{ChangeOwnerError, upgrade_owner},
        process::{VmmProcess, VmmProcessError, VmmProcessState},
        resource::system::{ResourceSystem, ResourceSystemError},
    },
};
use api::VmApiError;
use bytes::Bytes;
use configuration::{InitMethod, VmConfiguration};
use http::Uri;
use http_body_util::Full;
use hyper_client_sockets::{connector::UnixConnector, uri::UnixUri};
use shutdown::{VmShutdownAction, VmShutdownError, VmShutdownOutcome};

pub mod api;
pub mod configuration;
pub mod models;
pub mod shutdown;
pub mod snapshot;

/// A [Vm] is an abstraction over a [VmmProcess], and automates away tasks not handled by a VMM process in an opinionated
/// fashion, such as: moving resources in and out, transforming resource paths from inner to outer and vice versa,
/// removing VM traces, creating snapshots, binding to the exact endpoints of the API server and fallback-based shutdown.
///
/// A [Vm] is generic over 3 components: [VmmExecutor] E, [ProcessSpawner] S and [Runtime] R, as it wraps a [VmmProcess] tied
/// to these components with opinionated functionality.
#[derive(Debug)]
pub struct Vm<E: VmmExecutor, S: ProcessSpawner, R: Runtime> {
    pub(crate) vmm_process: VmmProcess<E, S, R>,
    is_paused: bool,
    configuration: VmConfiguration,
}

/// The high-level state of a [Vm]. Unlike the state of a [VmmProcess], this state tracks the virtual machine and its operating state,
/// not that of the VMM itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VmState {
    /// The [Vm] has been prepared but not started yet.
    NotStarted,
    /// The [Vm] was booted and is running.
    Running,
    /// The [Vm] was booted, but was paused per API request.
    Paused,
    /// The [Vm] (and [VmmProcess]) exited gracefully, typically with a 0 exit status.
    Exited,
    /// The [Vm] (and [VmmProcess]) exited with the provided abnormal exit status.
    Crashed(ExitStatus),
}

impl std::fmt::Display for VmState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VmState::NotStarted => write!(f, "Not started"),
            VmState::Running => write!(f, "Running"),
            VmState::Paused => write!(f, "Paused"),
            VmState::Exited => write!(f, "Exited"),
            VmState::Crashed(exit_status) => write!(f, "Crashed with exit status: {exit_status}"),
        }
    }
}

/// All errors that can be produced by a [Vm].
#[derive(Debug)]
pub enum VmError {
    /// A [VmmProcessError] occurred.
    ProcessError(VmmProcessError),
    /// A [ChangeOwnerError] occurred.
    ChangeOwnerError(ChangeOwnerError),
    /// An I/O error occurred while performing operations on the filesystem.
    FilesystemError(std::io::Error),
    /// The [VmState] of the [Vm] was incorrect due to a reason specified within a [VmStateCheckError].
    StateCheckError(VmStateCheckError),
    /// A [VmApiError] error occurred while making Management API calls automatically as part of VM startup.
    ApiError(VmApiError),
    /// An error occurred while serializing the VM's configuration to JSON via [serde_json] as part of VM startup.
    SerdeError(serde_json::Error),
    /// A future waiting for the Management API Unix socket to become available timed out in accordance with the
    /// provided timeout [Duration].
    SocketWaitTimeout,
    /// Using a [VmConfiguration] with a disabled Management API Unix socket was attempted, which is not supported
    /// by the VM layer.
    DisabledApiSocketIsUnsupported,
    /// A [ResourceSystemError] occurred.
    ResourceSystemError(ResourceSystemError),
}

impl std::error::Error for VmError {}

impl std::fmt::Display for VmError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VmError::ProcessError(err) => write!(f, "The underlying VMM process returned an error: {err}"),
            VmError::ChangeOwnerError(err) => {
                write!(f, "An ownership change failed: {err}")
            }
            VmError::FilesystemError(err) => {
                write!(f, "A filesystem operation backed by the runtime failed: {err}")
            }
            VmError::StateCheckError(err) => write!(f, "A state check of the VM failed: {err}"),
            VmError::ApiError(err) => write!(f, "A request issued to the API server internally failed: {err}"),
            VmError::SerdeError(err) => {
                write!(f, "Serialization of the transient JSON configuration failed: {err}")
            }
            VmError::SocketWaitTimeout => write!(f, "The wait for the API socket to become available timed out"),
            VmError::DisabledApiSocketIsUnsupported => write!(
                f,
                "Attempted to use a VM configuration with a disabled API socket, which is not supported"
            ),
            VmError::ResourceSystemError(err) => write!(f, "A resource system error occurred: {err}"),
        }
    }
}

#[derive(Debug)]
pub enum VmStateCheckError {
    ExitedOrCrashed { actual: VmState },
    PausedOrRunning { actual: VmState },
    Other { expected: VmState, actual: VmState },
}

impl std::error::Error for VmStateCheckError {}

impl std::fmt::Display for VmStateCheckError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VmStateCheckError::ExitedOrCrashed { actual } => write!(
                f,
                "Expected the VM to have exited or crashed, but the actual state was {actual}"
            ),
            VmStateCheckError::PausedOrRunning { actual } => write!(
                f,
                "Expected the VM to be paused or running, but the actual state was {actual}"
            ),
            VmStateCheckError::Other { expected, actual } => write!(
                f,
                "Expected the VM to be in {expected} state, but it was in the {actual} state"
            ),
        }
    }
}

impl<E: VmmExecutor, S: ProcessSpawner, R: Runtime> Vm<E, S, R> {
    /// Prepare the full environment of a [Vm] without booting it. This requires a [VmConfiguration], in which all resources
    /// are created within the given [ResourceSystem], a [VmmExecutor] and a [VmmInstallation].
    pub async fn prepare(
        executor: E,
        resource_system: ResourceSystem<S, R>,
        installation: VmmInstallation,
        configuration: VmConfiguration,
    ) -> Result<Self, VmError> {
        if executor.get_socket_path(&installation).is_none() {
            return Err(VmError::DisabledApiSocketIsUnsupported);
        }

        let mut vmm_process = VmmProcess::new(executor, resource_system, installation);

        vmm_process.prepare().await.map_err(VmError::ProcessError)?;

        Ok(Self {
            vmm_process,
            is_paused: false,
            configuration,
        })
    }

    /// Retrieve the [VmState] of the [Vm], based on internal tracking and that being done by the [VmmProcess].
    pub fn get_state(&mut self) -> VmState {
        match self.vmm_process.get_state() {
            VmmProcessState::Started => match self.is_paused {
                true => VmState::Paused,
                false => VmState::Running,
            },
            VmmProcessState::Exited => VmState::Exited,
            VmmProcessState::Crashed(exit_status) => VmState::Crashed(exit_status),
            _ => VmState::NotStarted,
        }
    }

    /// Start/boot the [Vm] and perform all necessary initialization steps according to the [VmConfiguration].
    pub async fn start(&mut self, socket_wait_timeout: Duration) -> Result<(), VmError> {
        self.ensure_state(VmState::NotStarted)
            .map_err(VmError::StateCheckError)?;
        let socket_path = self
            .vmm_process
            .get_socket_path()
            .ok_or(VmError::DisabledApiSocketIsUnsupported)?;

        let mut config_path = None;
        if let VmConfiguration::New {
            init_method: InitMethod::ViaJsonConfiguration(ref config_local_path),
            ref data,
        } = self.configuration
        {
            let config_effective_path = self.vmm_process.resolve_effective_path(config_local_path.clone());
            config_path = Some(config_local_path.clone());
            upgrade_owner(
                &config_effective_path,
                self.vmm_process.resource_system.ownership_model,
                &self.vmm_process.resource_system.process_spawner,
                &self.vmm_process.resource_system.runtime,
            )
            .await
            .map_err(VmError::ChangeOwnerError)?;

            self.vmm_process
                .resource_system
                .runtime
                .fs_write(
                    &config_effective_path,
                    serde_json::to_string(data).map_err(VmError::SerdeError)?,
                )
                .await
                .map_err(VmError::FilesystemError)?;
        }

        self.vmm_process
            .invoke(config_path)
            .await
            .map_err(VmError::ProcessError)?;

        let client = hyper_util::client::legacy::Builder::new(RuntimeHyperExecutor(
            self.vmm_process.resource_system.runtime.clone(),
        ))
        .build::<_, Full<Bytes>>(UnixConnector::<R::SocketBackend>::new());

        self.vmm_process
            .resource_system
            .runtime
            .timeout(socket_wait_timeout, async move {
                loop {
                    if client
                        .get(Uri::unix(&socket_path, "/").expect("/ route was invalid for the socket path"))
                        .await
                        .is_ok()
                    {
                        break;
                    }
                }
            })
            .await
            .map_err(|_| VmError::SocketWaitTimeout)?;

        match self.configuration.clone() {
            VmConfiguration::New { init_method, data } => {
                if init_method == InitMethod::ViaApiCalls {
                    api::init_new(self, data).await.map_err(VmError::ApiError)?;
                }
            }
            VmConfiguration::RestoredFromSnapshot { load_snapshot, data } => {
                api::init_restored_from_snapshot(self, data, load_snapshot)
                    .await
                    .map_err(VmError::ApiError)?;
            }
        }

        Ok(())
    }

    /// Shut down the [Vm] by applying the given sequence of [VmShutdownAction]s until one works or all fail. If even one action works,
    /// a [VmShutdownOutcome] is returned with further information about the shutdown result, otherwise, the [VmShutdownError] caused
    /// by the last [VmShutdownAction] in the sequence is returned.
    pub async fn shutdown<I: IntoIterator<Item = VmShutdownAction>>(
        &mut self,
        actions: I,
    ) -> Result<VmShutdownOutcome, VmShutdownError> {
        shutdown::apply(self, actions.into_iter()).await
    }

    /// Clean up the full environment of this [Vm] after it being [VmState::Exited] or [VmState::Crashed].
    pub async fn cleanup(&mut self) -> Result<(), VmError> {
        self.ensure_exited_or_crashed().map_err(VmError::StateCheckError)?;
        self.vmm_process.cleanup().await.map_err(VmError::ProcessError)
    }

    /// Take out the [ProcessHandlePipes] of the underlying process handle if possible.
    pub fn take_pipes(&mut self) -> Result<ProcessHandlePipes<R::Child>, VmError> {
        self.ensure_paused_or_running().map_err(VmError::StateCheckError)?;
        self.vmm_process.take_pipes().map_err(VmError::ProcessError)
    }

    /// Get a shared reference to the [Vm]'s [VmConfiguration].
    pub fn get_configuration(&self) -> &VmConfiguration {
        &self.configuration
    }

    /// Transforms a given local resource path into an effective resource path using the underlying [VmmProcess].
    /// This should be used with care and only in cases when the facilities of the [ResourceSystem] prove to be insufficient.
    pub fn resolve_effective_path<P: Into<PathBuf>>(&self, local_path: P) -> PathBuf {
        self.vmm_process.resolve_effective_path(local_path)
    }

    /// Get a shared reference to the [ResourceSystem] used by this [Vm].
    pub fn get_resource_system(&self) -> &ResourceSystem<S, R> {
        self.vmm_process.get_resource_system()
    }

    /// Get a mutable reference to the [ResourceSystem] used by this [Vm].
    pub fn get_resource_system_mut(&mut self) -> &mut ResourceSystem<S, R> {
        self.vmm_process.get_resource_system_mut()
    }

    #[inline]
    fn ensure_state(&mut self, expected_state: VmState) -> Result<(), VmStateCheckError> {
        let current_state = self.get_state();

        if current_state != expected_state {
            Err(VmStateCheckError::Other {
                expected: expected_state,
                actual: current_state,
            })
        } else {
            Ok(())
        }
    }

    #[inline]
    fn ensure_paused_or_running(&mut self) -> Result<(), VmStateCheckError> {
        let current_state = self.get_state();

        if current_state != VmState::Running && current_state != VmState::Paused {
            Err(VmStateCheckError::PausedOrRunning { actual: current_state })
        } else {
            Ok(())
        }
    }

    #[inline]
    fn ensure_exited_or_crashed(&mut self) -> Result<(), VmStateCheckError> {
        let current_state = self.get_state();

        if let VmState::Crashed(_) = current_state {
            return Ok(());
        }

        if current_state == VmState::Exited {
            return Ok(());
        }

        Err(VmStateCheckError::ExitedOrCrashed { actual: current_state })
    }
}
