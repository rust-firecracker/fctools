//! Provides a managed and high-level VM abstraction.

use std::{path::PathBuf, process::ExitStatus, sync::Arc, time::Duration};

use crate::{
    process_spawner::ProcessSpawner,
    runtime::{Runtime, RuntimeExecutor, RuntimeFilesystem},
    vmm::{
        executor::{process_handle::ProcessHandlePipes, VmmExecutor},
        installation::VmmInstallation,
        ownership::{upgrade_owner, ChangeOwnerError, VmmOwnershipModel},
        process::{VmmProcess, VmmProcessError, VmmProcessState},
    },
};
use api::VmApiError;
use bytes::Bytes;
use configuration::{InitMethod, VmConfiguration};
use http::Uri;
use http_body_util::Full;
use hyper_client_sockets::unix::{connector::HyperUnixConnector, UnixUriExt};
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
/// A [Vm] is tied to 3 components: [VmmExecutor] E, [ProcessSpawner] S and [Runtime] R, as it wraps a [VmmProcess] tied
/// to these components with opinionated functionality.
#[derive(Debug)]
pub struct Vm<E: VmmExecutor, S: ProcessSpawner, R: Runtime> {
    vmm_process: VmmProcess<E, S, R>,
    process_spawner: Arc<S>,
    ownership_model: VmmOwnershipModel,
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
    ProcessError(VmmProcessError),
    ChangeOwnerError(ChangeOwnerError),
    FilesystemError(std::io::Error),
    TaskJoinFailed,
    MkfifoError(std::io::Error),
    StateCheckError(VmStateCheckError),
    ApiError(VmApiError),
    ConfigurationSerdeError(serde_json::Error),
    SocketWaitTimeout,
    DisabledApiSocketIsUnsupported,
    MissingPathMapping,
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
            VmError::TaskJoinFailed => write!(f, "Joining on an async task via the runtime failed"),
            VmError::MkfifoError(err) => write!(f, "Making a FIFO named pipe failed: {err}"),
            VmError::StateCheckError(err) => write!(f, "A state check of the VM failed: {err}"),
            VmError::ApiError(err) => write!(f, "A request issued to the API server internally failed: {err}"),
            VmError::ConfigurationSerdeError(err) => {
                write!(f, "Serialization of the transient JSON configuration failed: {err}")
            }
            VmError::SocketWaitTimeout => write!(f, "The wait for the API socket to become available timed out"),
            VmError::DisabledApiSocketIsUnsupported => write!(
                f,
                "Attempted to use a VM configuration with a disabled API socket, which is not supported"
            ),
            VmError::MissingPathMapping => write!(
                f,
                "A path mapping was expected to be constructed by the executor, but was not returned"
            ),
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
    /// Prepare the full environment of a [Vm] without booting it. Analogously to the [VmmProcess], the passed-in resources
    /// can be either owned or [Arc]-ed; in the former case they will be put into an [Arc].
    pub async fn prepare(
        executor: E,
        ownership_model: VmmOwnershipModel,
        process_spawner: impl Into<Arc<S>>,
        installation: impl Into<Arc<VmmInstallation>>,
        mut configuration: VmConfiguration,
    ) -> Result<Self, VmError> {
        let process_spawner = process_spawner.into();
        let installation = installation.into();

        if executor.get_socket_path(installation.as_ref()).is_none() {
            return Err(VmError::DisabledApiSocketIsUnsupported);
        }

        let mut vmm_process = VmmProcess::new(executor, ownership_model, process_spawner.clone(), installation);
        vmm_process
            .prepare(configuration.data_mut().resource_references())
            .await
            .map_err(VmError::ProcessError)?;

        Ok(Self {
            vmm_process,
            process_spawner,
            ownership_model,
            is_paused: false,
            configuration,
        })
    }

    /// Retrieve the [VmState] of the [Vm], based on internal tracking and that being done by the [VmmProcess].
    pub fn state(&mut self) -> VmState {
        match self.vmm_process.state() {
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
            let config_effective_path = self.vmm_process.local_to_effective_path(config_local_path.clone());
            config_path = Some(config_local_path.clone());
            upgrade_owner::<R>(
                &config_effective_path,
                self.ownership_model,
                self.process_spawner.as_ref(),
            )
            .await
            .map_err(VmError::ChangeOwnerError)?;

            R::Filesystem::write_file(
                &config_effective_path,
                serde_json::to_string(data).map_err(VmError::ConfigurationSerdeError)?,
            )
            .await
            .map_err(VmError::FilesystemError)?;
        }

        self.vmm_process
            .invoke(config_path)
            .await
            .map_err(VmError::ProcessError)?;

        let client = hyper_util::client::legacy::Builder::new(R::get_hyper_executor()).build::<_, Full<Bytes>>(
            HyperUnixConnector {
                backend: R::get_hyper_client_sockets_backend(),
            },
        );

        R::Executor::timeout(socket_wait_timeout, async move {
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
    pub async fn shutdown(
        &mut self,
        actions: impl IntoIterator<Item = VmShutdownAction>,
    ) -> Result<VmShutdownOutcome, VmShutdownError> {
        shutdown::apply(self, actions).await
    }

    /// Clean up the full environment of this [Vm] after it being [VmState::Exited] or [VmState::Crashed].
    pub async fn cleanup(&mut self) -> Result<(), VmError> {
        self.ensure_exited_or_crashed().map_err(VmError::StateCheckError)?;
        self.vmm_process
            .cleanup(self.configuration.data_mut().resource_references())
            .await
            .map_err(VmError::ProcessError)
    }

    /// Take out the [ProcessHandlePipes] of the underlying process handle if possible.
    pub fn take_pipes(&mut self) -> Result<ProcessHandlePipes<R::Process>, VmError> {
        self.ensure_paused_or_running().map_err(VmError::StateCheckError)?;
        self.vmm_process.take_pipes().map_err(VmError::ProcessError)
    }

    /// Get a shared reference to the [Vm]'s [VmConfiguration].
    pub fn configuration(&self) -> &VmConfiguration {
        &self.configuration
    }

    /// Translates the given local resource path to an effective resource path.
    pub fn local_to_effective_path(&self, local_path: impl Into<PathBuf>) -> PathBuf {
        self.vmm_process.local_to_effective_path(local_path)
    }

    pub(super) fn ensure_state(&mut self, expected_state: VmState) -> Result<(), VmStateCheckError> {
        let current_state = self.state();
        if current_state != expected_state {
            Err(VmStateCheckError::Other {
                expected: expected_state,
                actual: current_state,
            })
        } else {
            Ok(())
        }
    }

    pub(super) fn ensure_paused_or_running(&mut self) -> Result<(), VmStateCheckError> {
        let current_state = self.state();
        if current_state != VmState::Running && current_state != VmState::Paused {
            Err(VmStateCheckError::PausedOrRunning { actual: current_state })
        } else {
            Ok(())
        }
    }

    fn ensure_exited_or_crashed(&mut self) -> Result<(), VmStateCheckError> {
        let current_state = self.state();
        if let VmState::Crashed(_) = current_state {
            return Ok(());
        }
        if current_state == VmState::Exited {
            return Ok(());
        }
        Err(VmStateCheckError::ExitedOrCrashed { actual: current_state })
    }
}
