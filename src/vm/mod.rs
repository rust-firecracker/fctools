//! Provides a managed and high-level VM abstraction.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    process::ExitStatus,
    sync::Arc,
    time::Duration,
};

use crate::{
    fs_backend::{FsBackend, FsBackendError},
    process_spawner::ProcessSpawner,
    vmm::{
        executor::{process_handle::ProcessHandlePipes, VmmExecutor},
        installation::VmmInstallation,
        ownership::{downgrade_owner, upgrade_owner, ChangeOwnerError, VmmOwnershipModel},
        process::{VmmProcess, VmmProcessError, VmmProcessState},
    },
};
use api::{VmApi, VmApiError};
use configuration::{InitMethod, VmConfiguration, VmConfigurationData};
use models::LoadSnapshot;
use nix::sys::stat::Mode;
use tokio::task::{JoinError, JoinSet};

pub mod api;
pub mod configuration;
pub mod models;
pub mod snapshot;

/// A [Vm] is an abstraction over a [VmmProcess], and automates away tasks not handled by a VMM process in an opinionated
/// fashion, such as: moving resources in and out, transforming resource paths from inner to outer and vice versa,
/// removing VM traces, creating snapshots, binding to the exact endpoints of the API server and fallback-based shutdown.
///
/// A [Vm] is tied to 3 components: [VmmExecutor] E, [ProcessSpawner] S and [FsBackend] F, as it wraps a [VmmProcess] tied
/// to these components with opinionated functionality.
#[derive(Debug)]
pub struct Vm<E: VmmExecutor, S: ProcessSpawner, F: FsBackend> {
    vmm_process: VmmProcess<E, S, F>,
    fs_backend: Arc<F>,
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
#[derive(Debug, thiserror::Error)]
pub enum VmError {
    #[error("The underlying VMM process returned an error: {0}")]
    ProcessError(VmmProcessError),
    #[error("An ownership change requested on the VM level failed: {0}")]
    ChangeOwnerError(ChangeOwnerError),
    #[error("A filesystem backend operation failed: {0}")]
    FsBackendError(FsBackendError),
    #[error("Joining on an async task failed: {0}")]
    TaskJoinFailed(JoinError),
    #[error("Making a FIFO named pipe failed: {0}")]
    MkfifoError(std::io::Error),
    #[error("A state check of the VM failed: {0}")]
    StateCheckError(VmStateCheckError),
    #[error("A request issued to the API server internally failed: {0}")]
    ApiError(VmApiError),
    #[error("Serialization of the transient JSON configuration failed: {0}")]
    ConfigurationSerdeError(serde_json::Error),
    #[error("No shutdown methods were specified for a VM shutdown operation")]
    NoShutdownMethodsSpecified,
    #[error("A future timed out according to the given timeout duration")]
    Timeout,
    #[error("Attempted to use a VM configuration with a disabled API socket, which is not supported")]
    DisabledApiSocketIsUnsupported,
    #[error("A path mapping was expected to be constructed by the executor, but was not returned")]
    MissingPathMapping,
}

#[derive(Debug, thiserror::Error)]
pub enum VmStateCheckError {
    #[error("Expected the VM to have exited or crashed, but the actual state was {actual}")]
    ExitedOrCrashed { actual: VmState },
    #[error("Expected the VM to be paused or running, but the actual state was {actual}")]
    PausedOrRunning { actual: VmState },
    #[error("Expected the VM to be in the {expected} state, but the actual state was {actual}")]
    Other { expected: VmState, actual: VmState },
}

/// A shutdown method that can be applied to attempt to terminate both the guest operating system and the VMM.
#[derive(Debug)]
pub enum ShutdownMethod {
    /// Send a Ctrl+Alt+Del, which is only available on x86_64.
    CtrlAltDel,
    /// Pause the VM per API request, wait for a graceful pause but then send a SIGKILL to the VMM process.
    /// This is generally used to try to emulate the graceful shutdown behavior of Ctrl+Alt+Del on non-x86_64 systems.
    PauseThenKill,
    /// Send a SIGKILL to the VMM process.
    Kill,
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

impl<E: VmmExecutor, S: ProcessSpawner, F: FsBackend> Vm<E, S, F> {
    /// Prepare the full environment of a [Vm] without booting it. Analogously to the [VmmProcess], the passed-in resources
    /// can be either owned or [Arc]-ed; in the former case they will be put into an [Arc].
    pub async fn prepare(
        executor: impl Into<Arc<E>>,
        ownership_model: VmmOwnershipModel,
        process_spawner: impl Into<Arc<S>>,
        fs_backend: impl Into<Arc<F>>,
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
        let fs_backend = fs_backend.into();
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
            fs_backend.clone(),
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
                    .insert(drive.drive_id.clone(), vmm_process.inner_to_outer_path(&socket));
            }
        }

        let mut prepare_join_set = JoinSet::new();

        if let Some(ref logger) = configuration.data().logger_system {
            if let Some(ref log_path) = logger.log_path {
                let new_log_path = vmm_process.inner_to_outer_path(log_path);
                prepare_join_set.spawn(prepare_file(
                    fs_backend.clone(),
                    process_spawner.clone(),
                    new_log_path.clone(),
                    ownership_model,
                    false,
                ));
                accessible_paths.log_path = Some(new_log_path);
            }
        }

        if let Some(ref metrics_system) = configuration.data().metrics_system {
            let new_metrics_path = vmm_process.inner_to_outer_path(&metrics_system.metrics_path);
            prepare_join_set.spawn(prepare_file(
                fs_backend.clone(),
                process_spawner.clone(),
                new_metrics_path.clone(),
                ownership_model,
                false,
            ));
            accessible_paths.metrics_path = Some(new_metrics_path);
        }

        if let Some(ref vsock) = configuration.data().vsock_device {
            let new_uds_path = vmm_process.inner_to_outer_path(&vsock.uds_path);
            prepare_join_set.spawn(prepare_file(
                fs_backend.clone(),
                process_spawner.clone(),
                new_uds_path.clone(),
                ownership_model,
                true,
            ));
            accessible_paths.vsock_multiplexer_path = Some(new_uds_path);
        }

        while let Some(result) = prepare_join_set.join_next().await {
            match result {
                Ok(result) => result?,
                Err(err) => {
                    return Err(VmError::TaskJoinFailed(err));
                }
            }
        }

        Ok(Self {
            vmm_process,
            fs_backend,
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
            ref init_method,
            ref data,
        } = configuration
        {
            if let InitMethod::ViaJsonConfiguration(inner_path) = init_method {
                let outer_path = self.vmm_process.inner_to_outer_path(inner_path);
                config_path = Some(inner_path.clone());
                prepare_file(
                    self.fs_backend.clone(),
                    self.process_spawner.clone(),
                    outer_path.clone(),
                    self.ownership_model,
                    true,
                )
                .await?;
                self.fs_backend
                    .write_file(
                        &outer_path,
                        serde_json::to_string(data).map_err(VmError::ConfigurationSerdeError)?,
                    )
                    .await
                    .map_err(VmError::FsBackendError)?;
            }
        }

        self.vmm_process
            .invoke(config_path)
            .await
            .map_err(VmError::ProcessError)?;

        let fs_backend = self.fs_backend.clone();
        tokio::time::timeout(socket_wait_timeout, async move {
            // wait until socket exists
            loop {
                if let Ok(true) = fs_backend.check_exists(&socket_path).await {
                    break;
                }
            }

            Ok(())
        })
        .await
        .map_err(|_| VmError::Timeout)?
        .map_err(VmError::FsBackendError)?;

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

    /// Shut down the [Vm] by sequentially applying the given [Vec<ShutdownMethod>] in order with the given timeout [Duration],
    /// until one of them works without erroring or all of them have errored.
    pub async fn shutdown(&mut self, shutdown_methods: Vec<ShutdownMethod>, timeout: Duration) -> Result<(), VmError> {
        self.ensure_paused_or_running().map_err(VmError::StateCheckError)?;
        let mut last_result = Ok(());

        for shutdown_method in shutdown_methods {
            last_result = match shutdown_method {
                ShutdownMethod::CtrlAltDel => self
                    .vmm_process
                    .send_ctrl_alt_del()
                    .await
                    .map_err(VmError::ProcessError),
                ShutdownMethod::PauseThenKill => match self.api_pause().await {
                    Ok(()) => self.vmm_process.send_sigkill().map_err(VmError::ProcessError),
                    Err(err) => Err(VmError::ApiError(err)),
                },
                ShutdownMethod::Kill => self.vmm_process.send_sigkill().map_err(VmError::ProcessError),
            };

            if last_result.is_ok() {
                last_result = tokio::time::timeout(timeout, self.vmm_process.wait_for_exit())
                    .await
                    .map_err(|_| VmError::Timeout)
                    .map(|_| ());
            }

            if last_result.is_ok() {
                break;
            }
        }

        if last_result.is_err() {
            return last_result;
        }

        Ok(())
    }

    /// Clean up the full environment of this [Vm] after it being [VmState::Exited] or [VmState::Crashed].
    pub async fn cleanup(&mut self) -> Result<(), VmError> {
        async fn remove_file(
            process_spawner: Arc<impl ProcessSpawner>,
            fs_backend: Arc<impl FsBackend>,
            path: PathBuf,
            ownership_model: VmmOwnershipModel,
        ) -> Result<(), VmError> {
            upgrade_owner(&path, ownership_model, process_spawner.as_ref(), fs_backend.as_ref())
                .await
                .map_err(VmError::ChangeOwnerError)?;

            fs_backend.remove_file(&path).await.map_err(VmError::FsBackendError)
        }

        self.ensure_exited_or_crashed().map_err(VmError::StateCheckError)?;
        self.vmm_process.cleanup().await.map_err(VmError::ProcessError)?;

        if self.executor.is_traceless() {
            return Ok(());
        }

        let mut join_set = JoinSet::new();

        if let Some(log_path) = self.accessible_paths.log_path.clone() {
            join_set.spawn(remove_file(
                self.process_spawner.clone(),
                self.fs_backend.clone(),
                log_path,
                self.ownership_model,
            ));
        }

        if let Some(metrics_path) = self.accessible_paths.metrics_path.clone() {
            join_set.spawn(remove_file(
                self.process_spawner.clone(),
                self.fs_backend.clone(),
                metrics_path,
                self.ownership_model,
            ));
        }

        if let Some(multiplexer_path) = self.accessible_paths.vsock_multiplexer_path.clone() {
            join_set.spawn(remove_file(
                self.process_spawner.clone(),
                self.fs_backend.clone(),
                multiplexer_path,
                self.ownership_model,
            ));

            for listener_path in self.accessible_paths.vsock_listener_paths.clone() {
                join_set.spawn(remove_file(
                    self.process_spawner.clone(),
                    self.fs_backend.clone(),
                    listener_path,
                    self.ownership_model,
                ));
            }
        }

        while let Some(snapshot_trace_path) = self.snapshot_traces.pop() {
            join_set.spawn(remove_file(
                self.process_spawner.clone(),
                self.fs_backend.clone(),
                snapshot_trace_path,
                self.ownership_model,
            ));
        }

        while let Some(remove_result) = join_set.join_next().await {
            let _ = remove_result; // ignore "doesn't exist" errors that have no effect
        }

        Ok(())
    }

    /// Take out the [ProcessHandlePipes] of the underlying process handle if possible.
    pub fn take_pipes(&mut self) -> Result<ProcessHandlePipes, VmError> {
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

async fn prepare_file(
    fs_backend: Arc<impl FsBackend>,
    process_spawner: Arc<impl ProcessSpawner>,
    path: PathBuf,
    ownership_model: VmmOwnershipModel,
    only_tree: bool,
) -> Result<(), VmError> {
    if let Some(parent_path) = path.parent() {
        fs_backend
            .create_dir_all(parent_path)
            .await
            .map_err(VmError::FsBackendError)?;
    }

    if !only_tree {
        if path.extension().map(|ext| ext.to_str()).flatten() == Some("fifo") {
            if nix::unistd::mkfifo(&path, Mode::S_IROTH | Mode::S_IWOTH | Mode::S_IRUSR | Mode::S_IWUSR).is_err() {
                return Err(VmError::MkfifoError(std::io::Error::last_os_error()));
            }
        } else {
            fs_backend.create_file(&path).await.map_err(VmError::FsBackendError)?;
        }

        downgrade_owner(&path, ownership_model, process_spawner.as_ref(), fs_backend.as_ref())
            .await
            .map_err(VmError::ChangeOwnerError)?;
    }

    Ok(())
}
