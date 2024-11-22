//! Provides a managed and high-level VM abstraction.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    process::ExitStatus,
    sync::Arc,
    time::Duration,
};

use crate::{
    process_spawner::ProcessSpawner,
    runtime::{Runtime, RuntimeExecutor, RuntimeFilesystem, RuntimeJoinSet},
    vmm::{
        executor::{process_handle::ProcessHandlePipes, VmmExecutor},
        installation::VmmInstallation,
        ownership::{downgrade_owner, downgrade_owner_recursively, upgrade_owner, ChangeOwnerError, VmmOwnershipModel},
        process::{VmmProcess, VmmProcessError, VmmProcessState},
    },
};
use api::VmApiError;
use bytes::Bytes;
use configuration::{InitMethod, VmConfiguration, VmConfigurationData};
use http::Uri;
use http_body_util::Full;
use hyper_client_sockets::unix::{connector::HyperUnixConnector, UnixUriExt};
use models::LoadSnapshot;
use nix::sys::stat::Mode;
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
    executor: Arc<E>,
    ownership_model: VmmOwnershipModel,
    is_paused: bool,
    original_configuration_data: VmConfigurationData,
    configuration: Option<VmConfiguration>,
    accessible_paths: AccessiblePaths,
    snapshot_traces: Vec<PathBuf>,
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

/// A set of common absolute (outer) paths associated with a [Vm].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessiblePaths {
    /// A hash map of drive IDs to paths of their sockets, if those were configured.
    pub drive_sockets: HashMap<String, PathBuf>,
    /// A path to the metrics file, if a metrics device was configured.
    pub metrics_path: Option<PathBuf>,
    /// A path to the log file, if a logger was configured.
    pub log_path: Option<PathBuf>,
    /// A path to the Unix socket managed by Firecracker that multiplexes connections over the vsock device to provided
    /// guest ports, if a vsock device was configured.
    pub vsock_multiplexer_path: Option<PathBuf>,
    /// A list of vsock device listener paths that receive guest-initiated connections, if a vsock device was configured.
    pub vsock_listener_paths: Vec<PathBuf>,
}

impl<E: VmmExecutor, S: ProcessSpawner, R: Runtime> Vm<E, S, R> {
    /// Prepare the full environment of a [Vm] without booting it. Analogously to the [VmmProcess], the passed-in resources
    /// can be either owned or [Arc]-ed; in the former case they will be put into an [Arc].
    pub async fn prepare(
        executor: impl Into<Arc<E>>,
        ownership_model: VmmOwnershipModel,
        process_spawner: impl Into<Arc<S>>,
        installation: impl Into<Arc<VmmInstallation>>,
        mut configuration: VmConfiguration,
    ) -> Result<Self, VmError> {
        fn get_outer_paths(data: &VmConfigurationData, load_snapshot: Option<&LoadSnapshot>) -> Vec<PathBuf> {
            let mut outer_paths = Vec::new();

            outer_paths.push(data.boot_source.kernel_image_path.clone());

            if let Some(ref path) = data.boot_source.initrd_path {
                outer_paths.push(path.clone());
            }

            for drive in &data.drives {
                if let Some(ref path) = drive.path_on_host {
                    outer_paths.push(path.clone());
                }
            }

            if let Some(load_snapshot) = load_snapshot {
                outer_paths.push(load_snapshot.snapshot_path.clone());
                outer_paths.push(load_snapshot.mem_backend.backend_path.clone());
            }

            outer_paths
        }

        let executor = executor.into();
        let process_spawner = process_spawner.into();
        let installation = installation.into();

        if executor.get_socket_path(installation.as_ref()).is_none() {
            return Err(VmError::DisabledApiSocketIsUnsupported);
        }

        // compute outer paths from configuration
        let outer_paths = match &configuration {
            VmConfiguration::New { init_method: _, data } => get_outer_paths(data, None),
            VmConfiguration::RestoredFromSnapshot { load_snapshot, data } => get_outer_paths(data, Some(load_snapshot)),
        };

        // prepare
        let mut vmm_process = VmmProcess::new(
            executor.clone(),
            ownership_model,
            process_spawner.clone(),
            installation,
            outer_paths,
        );
        let mut path_mappings = vmm_process.prepare().await.map_err(VmError::ProcessError)?;

        // transform data according to returned mappings
        let original_configuration_data = configuration.data().clone();

        configuration.data_mut().boot_source.kernel_image_path = path_mappings
            .remove(&configuration.data().boot_source.kernel_image_path)
            .ok_or(VmError::MissingPathMapping)?;

        if let Some(ref mut path) = configuration.data_mut().boot_source.initrd_path {
            configuration.data_mut().boot_source.initrd_path =
                Some(path_mappings.remove(path).ok_or(VmError::MissingPathMapping)?);
        }

        for drive in &mut configuration.data_mut().drives {
            if let Some(ref mut path) = drive.path_on_host {
                *path = path_mappings.remove(path).ok_or(VmError::MissingPathMapping)?;
            }
        }

        if let VmConfiguration::RestoredFromSnapshot {
            ref mut load_snapshot,
            data: _,
        } = configuration
        {
            load_snapshot.snapshot_path = path_mappings
                .remove(&load_snapshot.snapshot_path)
                .ok_or(VmError::MissingPathMapping)?;
            load_snapshot.mem_backend.backend_path = path_mappings
                .remove(&load_snapshot.mem_backend.backend_path)
                .ok_or(VmError::MissingPathMapping)?;
        }

        // generate accessible paths, ensure paths exist
        let mut accessible_paths = AccessiblePaths {
            drive_sockets: HashMap::new(),
            metrics_path: None,
            log_path: None,
            vsock_multiplexer_path: None,
            vsock_listener_paths: Vec::new(),
        };

        for drive in &configuration.data().drives {
            if let Some(ref socket) = drive.socket {
                accessible_paths
                    .drive_sockets
                    .insert(drive.drive_id.clone(), vmm_process.inner_to_outer_path(socket));
            }
        }

        let mut join_set: RuntimeJoinSet<_, R::Executor> = RuntimeJoinSet::new();

        if let Some(ref logger) = configuration.data().logger_system {
            if let Some(ref log_path) = logger.log_path {
                let new_log_path = vmm_process.inner_to_outer_path(log_path);
                join_set.spawn(prepare_file::<R>(new_log_path.clone(), ownership_model, false));
                accessible_paths.log_path = Some(new_log_path);
            }
        }

        if let Some(ref metrics_system) = configuration.data().metrics_system {
            let new_metrics_path = vmm_process.inner_to_outer_path(&metrics_system.metrics_path);
            join_set.spawn(prepare_file::<R>(new_metrics_path.clone(), ownership_model, false));
            accessible_paths.metrics_path = Some(new_metrics_path);
        }

        if let Some(ref vsock) = configuration.data().vsock_device {
            let new_uds_path = vmm_process.inner_to_outer_path(&vsock.uds_path);
            join_set.spawn(prepare_file::<R>(new_uds_path.clone(), ownership_model, true));
            accessible_paths.vsock_multiplexer_path = Some(new_uds_path);
        }

        join_set.wait().await.unwrap_or(Err(VmError::TaskJoinFailed))?;

        Ok(Self {
            vmm_process,
            process_spawner,
            executor,
            ownership_model,
            is_paused: false,
            original_configuration_data,
            configuration: Some(configuration),
            accessible_paths,
            snapshot_traces: Vec::new(),
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
        let configuration = self
            .configuration
            .take()
            .expect("No configuration cannot exist for a VM, unreachable");
        let socket_path = self
            .vmm_process
            .get_socket_path()
            .ok_or(VmError::DisabledApiSocketIsUnsupported)?;

        let mut config_path = None;
        if let VmConfiguration::New {
            init_method: InitMethod::ViaJsonConfiguration(ref inner_path),
            ref data,
        } = configuration
        {
            let outer_path = self.vmm_process.inner_to_outer_path(inner_path);
            config_path = Some(inner_path.clone());
            prepare_file::<R>(outer_path.clone(), self.ownership_model, true).await?;
            R::Filesystem::write_file(
                &outer_path,
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

        match configuration {
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
        async fn remove_file<RI: Runtime>(
            process_spawner: Arc<impl ProcessSpawner>,
            path: PathBuf,
            ownership_model: VmmOwnershipModel,
        ) -> Result<(), VmError> {
            upgrade_owner::<RI>(&path, ownership_model, process_spawner.as_ref())
                .await
                .map_err(VmError::ChangeOwnerError)?;

            RI::Filesystem::remove_file(&path)
                .await
                .map_err(VmError::FilesystemError)
        }

        self.ensure_exited_or_crashed().map_err(VmError::StateCheckError)?;
        self.vmm_process.cleanup().await.map_err(VmError::ProcessError)?;

        if self.executor.is_traceless() {
            return Ok(());
        }

        let mut join_set: RuntimeJoinSet<_, R::Executor> = RuntimeJoinSet::new();

        if let Some(log_path) = self.accessible_paths.log_path.clone() {
            join_set.spawn(remove_file::<R>(
                self.process_spawner.clone(),
                log_path,
                self.ownership_model,
            ));
        }

        if let Some(metrics_path) = self.accessible_paths.metrics_path.clone() {
            join_set.spawn(remove_file::<R>(
                self.process_spawner.clone(),
                metrics_path,
                self.ownership_model,
            ));
        }

        if let Some(multiplexer_path) = self.accessible_paths.vsock_multiplexer_path.clone() {
            join_set.spawn(remove_file::<R>(
                self.process_spawner.clone(),
                multiplexer_path,
                self.ownership_model,
            ));

            for listener_path in self.accessible_paths.vsock_listener_paths.clone() {
                join_set.spawn(remove_file::<R>(
                    self.process_spawner.clone(),
                    listener_path,
                    self.ownership_model,
                ));
            }
        }

        while let Some(snapshot_trace_path) = self.snapshot_traces.pop() {
            join_set.spawn(remove_file::<R>(
                self.process_spawner.clone(),
                snapshot_trace_path,
                self.ownership_model,
            ));
        }

        join_set.wait().await.unwrap_or(Err(VmError::TaskJoinFailed))
    }

    /// Take out the [ProcessHandlePipes] of the underlying process handle if possible.
    pub fn take_pipes(&mut self) -> Result<ProcessHandlePipes<R::Process>, VmError> {
        self.ensure_paused_or_running().map_err(VmError::StateCheckError)?;
        self.vmm_process.take_pipes().map_err(VmError::ProcessError)
    }

    /// Get a set of standard [AccessiblePaths] for this [Vm].
    pub fn get_accessible_paths(&self) -> &AccessiblePaths {
        &self.accessible_paths
    }

    /// Translate an arbitrary inner path of this [Vm] into an owned accessible/outer path.
    pub fn get_accessible_path_from_inner(&self, inner_path: impl AsRef<Path>) -> PathBuf {
        self.vmm_process.inner_to_outer_path(inner_path)
    }

    /// Register a user-defined vsock listener path into the [AccessiblePaths], so that it can be tracked.
    pub fn register_vsock_listener_path(&mut self, listener_path: impl Into<PathBuf>) {
        self.accessible_paths.vsock_listener_paths.push(listener_path.into());
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

async fn prepare_file<R: Runtime>(
    path: PathBuf,
    ownership_model: VmmOwnershipModel,
    only_tree: bool,
) -> Result<(), VmError> {
    if let Some(parent_path) = path.parent() {
        R::Filesystem::create_dir_all(parent_path)
            .await
            .map_err(VmError::FilesystemError)?;
    }

    if !only_tree {
        if path.extension().and_then(|ext| ext.to_str()) == Some("fifo") {
            if nix::unistd::mkfifo(&path, Mode::S_IROTH | Mode::S_IWOTH | Mode::S_IRUSR | Mode::S_IWUSR).is_err() {
                return Err(VmError::MkfifoError(std::io::Error::last_os_error()));
            }
        } else {
            R::Filesystem::create_file(&path)
                .await
                .map_err(VmError::FilesystemError)?;
        }

        downgrade_owner_recursively::<R>(&path, ownership_model)
            .await
            .map_err(VmError::ChangeOwnerError)?;
    } else if let Some(parent_path) = path.parent() {
        downgrade_owner(parent_path, ownership_model).map_err(VmError::ChangeOwnerError)?;
    }

    Ok(())
}
