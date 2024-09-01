use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    process::ExitStatus,
    sync::Arc,
    time::Duration,
};

use crate::{
    executor::{arguments::FirecrackerConfigOverride, installation::FirecrackerInstallation, VmmExecutor},
    process::{VmmProcess, VmmProcessError, VmmProcessPipes, VmmProcessState},
    shell_spawner::ShellSpawner,
};
use api::VmApi;
use configuration::{NewVmBootMethod, VmConfiguration, VmConfigurationData};
use http::StatusCode;
use models::VmLoadSnapshot;
use tokio::{fs, task::JoinSet};

pub mod api;
pub mod configuration;
pub mod models;
pub mod snapshot;

#[derive(Debug)]
pub struct Vm<E: VmmExecutor, S: ShellSpawner> {
    vmm_process: VmmProcess<E, S>,
    is_paused: bool,
    original_configuration_data: VmConfigurationData,
    configuration: Option<VmConfiguration>,
    accessible_paths: VmAccessiblePaths,
    executor_traceless: bool,
    snapshot_traces: Vec<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VmState {
    NotStarted,
    Running,
    Paused,
    Exited,
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

#[derive(Debug, thiserror::Error)]
pub enum VmError {
    #[error("The underlying VMM process returned an error: `{0}`")]
    ProcessError(VmmProcessError),
    #[error("Expected the VM to be in the `{expected}` state, but it was actually in the `{actual}` state")]
    ExpectedState { expected: VmState, actual: VmState },
    #[error("Expected the VM to have exited or crashed, but it was actually in the `{actual}` state")]
    ExpectedExitedOrCrashed { actual: VmState },
    #[error("Expected the VM to be either paused or running, but it was actually in the `{actual}` state")]
    ExpectedPausedOrRunning { actual: VmState },
    #[error("Serde serialization or deserialization failed: `{0}`")]
    SerdeError(serde_json::Error),
    #[error("An async I/O operation failed: `{0}`")]
    IoError(tokio::io::Error),
    #[error(
        "The API socket returned an unsuccessful HTTP response with the `{status_code}` status code: `{fault_message}`"
    )]
    ApiRespondedWithFault {
        status_code: StatusCode,
        fault_message: String,
    },
    #[error(
        "The API HTTP request could not be constructed, likely due to an incorrect URI or HTTP configuration: `{0}`"
    )]
    ApiRequestNotConstructed(http::Error),
    #[error("The API HTTP response could not be received from the API socket: `{0}`")]
    ApiResponseCouldNotBeReceived(hyper::Error),
    #[error("Expected the API response to be empty, but it contained the following response body: `{0}`")]
    ApiResponseExpectedEmpty(String),
    #[error("No shutdown methods were specified for the VM shutdown operation")]
    NoShutdownMethodsSpecified,
    #[error("A future timed out according to the given timeout duration")]
    Timeout,
    #[error("Attempted to use a VM configuration with a disabled API socket, which is not supported")]
    DisabledApiSocketIsUnsupported,
    #[error("A path mapping was expected to be constructed by the executor, but was not returned")]
    MissingPathMapping,
}

#[derive(Debug)]
pub enum VmShutdownMethod {
    CtrlAltDel,
    PauseThenKill,
    Kill,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VmAccessiblePaths {
    pub drive_sockets: HashMap<String, PathBuf>,
    pub metrics_path: Option<PathBuf>,
    pub log_path: Option<PathBuf>,
    pub vsock_multiplexer_path: Option<PathBuf>,
    pub vsock_listener_paths: Vec<PathBuf>,
}

impl<E: VmmExecutor, S: ShellSpawner> Vm<E, S> {
    pub async fn prepare(
        executor: E,
        shell_spawner: S,
        installation: FirecrackerInstallation,
        configuration: VmConfiguration,
    ) -> Result<Self, VmError> {
        Self::prepare_arced(
            Arc::new(executor),
            Arc::new(shell_spawner),
            Arc::new(installation),
            configuration,
        )
        .await
    }

    pub async fn prepare_arced(
        executor: Arc<E>,
        shell_spawner_arc: Arc<S>,
        installation_arc: Arc<FirecrackerInstallation>,
        mut configuration: VmConfiguration,
    ) -> Result<Self, VmError> {
        fn get_outer_paths(data: &VmConfigurationData, load_snapshot: Option<&VmLoadSnapshot>) -> Vec<PathBuf> {
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

        if executor.get_socket_path(installation_arc.as_ref()).is_none() {
            return Err(VmError::DisabledApiSocketIsUnsupported);
        }

        // compute outer paths from configuration
        let outer_paths = match &configuration {
            VmConfiguration::New { boot_method: _, data } => get_outer_paths(data, None),
            VmConfiguration::RestoredFromSnapshot { load_snapshot, data } => get_outer_paths(data, Some(load_snapshot)),
        };

        // prepare
        let executor_traceless = executor.traceless();
        let mut vmm_process = VmmProcess::new_arced(executor, shell_spawner_arc, installation_arc, outer_paths);
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
        let mut accessible_paths = VmAccessiblePaths {
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

        if let Some(ref logger) = configuration.data().logger {
            if let Some(ref log_path) = logger.log_path {
                let new_log_path = vmm_process.inner_to_outer_path(log_path);
                prepare_file(&new_log_path, false).await?;
                accessible_paths.log_path = Some(new_log_path);
            }
        }

        if let Some(ref metrics_system) = configuration.data().metrics_system {
            let new_metrics_path = vmm_process.inner_to_outer_path(&metrics_system.metrics_path);
            prepare_file(&new_metrics_path, false).await?;
            accessible_paths.metrics_path = Some(new_metrics_path);
        }

        if let Some(ref vsock) = configuration.data().vsock {
            let new_uds_path = vmm_process.inner_to_outer_path(&vsock.uds_path);
            prepare_file(&new_uds_path, true).await?;
            accessible_paths.vsock_multiplexer_path = Some(new_uds_path);
        }
        if let Some(ref logger) = configuration.data().logger {
            if let Some(ref log_path) = logger.log_path {
                let new_log_path = vmm_process.inner_to_outer_path(log_path);
                prepare_file(&new_log_path, false).await?;
                accessible_paths.log_path = Some(new_log_path);
            }
        }

        if let Some(ref metrics) = configuration.data().metrics_system {
            let new_metrics_path = vmm_process.inner_to_outer_path(&metrics.metrics_path);
            prepare_file(&new_metrics_path, false).await?;
            accessible_paths.metrics_path = Some(new_metrics_path);
        }

        Ok(Self {
            vmm_process,
            is_paused: false,
            original_configuration_data,
            configuration: Some(configuration),
            accessible_paths,
            executor_traceless,
            snapshot_traces: Vec::new(),
        })
    }

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

    pub async fn start(&mut self, socket_wait_timeout: Duration) -> Result<(), VmError> {
        self.ensure_state(VmState::NotStarted)?;
        let configuration = self
            .configuration
            .take()
            .expect("No configuration cannot exist for a VM, unreachable");
        let socket_path = self
            .vmm_process
            .get_socket_path()
            .ok_or(VmError::DisabledApiSocketIsUnsupported)?;

        let mut config_override = FirecrackerConfigOverride::NoOverride;
        if let VmConfiguration::New {
            ref boot_method,
            ref data,
        } = configuration
        {
            if let NewVmBootMethod::ViaJsonConfiguration(inner_path) = boot_method {
                config_override = FirecrackerConfigOverride::Enable(inner_path.clone());
                prepare_file(inner_path, true).await?;
                fs::write(
                    self.vmm_process.inner_to_outer_path(inner_path),
                    serde_json::to_string(data).map_err(VmError::SerdeError)?,
                )
                .await
                .map_err(VmError::IoError)?;
            }
        }

        self.vmm_process
            .invoke(config_override)
            .await
            .map_err(VmError::ProcessError)?;

        tokio::time::timeout(socket_wait_timeout, async move {
            loop {
                if fs::try_exists(&socket_path).await? {
                    break;
                }
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
            Ok(())
        })
        .await
        .map_err(|_| VmError::Timeout)?
        .map_err(VmError::IoError)?;

        match configuration {
            VmConfiguration::New { boot_method, data } => {
                if boot_method == NewVmBootMethod::ViaApiCalls {
                    api::init_new(self, data).await?;
                }
            }
            VmConfiguration::RestoredFromSnapshot { load_snapshot, data } => {
                api::init_restored_from_snapshot(self, data, load_snapshot).await?;
            }
        }

        Ok(())
    }

    pub async fn shutdown(
        &mut self,
        shutdown_methods: Vec<VmShutdownMethod>,
        timeout: Duration,
    ) -> Result<(), VmError> {
        self.ensure_paused_or_running()?;
        let mut last_result = Ok(());

        for shutdown_method in shutdown_methods {
            last_result = match shutdown_method {
                VmShutdownMethod::CtrlAltDel => self
                    .vmm_process
                    .send_ctrl_alt_del()
                    .await
                    .map_err(VmError::ProcessError),
                VmShutdownMethod::PauseThenKill => match self.api_pause().await {
                    Ok(()) => self.vmm_process.send_sigkill().map_err(VmError::ProcessError),
                    Err(err) => Err(err),
                },
                VmShutdownMethod::Kill => self.vmm_process.send_sigkill().map_err(VmError::ProcessError),
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

    pub async fn cleanup(&mut self) -> Result<(), VmError> {
        self.ensure_exited_or_crashed()?;
        self.vmm_process.cleanup().await.map_err(VmError::ProcessError)?;

        if self.executor_traceless {
            return Ok(());
        }

        let mut join_set = JoinSet::new();

        if let Some(log_path) = self.accessible_paths.log_path.clone() {
            join_set.spawn_blocking(move || std::fs::remove_file(log_path));
        }

        if let Some(metrics_path) = self.accessible_paths.metrics_path.clone() {
            join_set.spawn_blocking(move || std::fs::remove_file(metrics_path));
        }

        if let Some(multiplexer_path) = self.accessible_paths.vsock_multiplexer_path.clone() {
            join_set.spawn_blocking(move || std::fs::remove_file(multiplexer_path));

            for listener_path in self.accessible_paths.vsock_listener_paths.clone() {
                join_set.spawn_blocking(move || std::fs::remove_file(listener_path));
            }
        }

        while let Some(snapshot_trace_path) = self.snapshot_traces.pop() {
            join_set.spawn_blocking(move || std::fs::remove_file(snapshot_trace_path));
        }

        while let Some(remove_result) = join_set.join_next().await {
            let _ = remove_result; // ignore "doesn't exist" errors that have no effect
        }

        Ok(())
    }

    pub fn take_pipes(&mut self) -> Result<VmmProcessPipes, VmError> {
        self.ensure_paused_or_running()?;
        self.vmm_process.take_pipes().map_err(VmError::ProcessError)
    }

    pub fn get_accessible_paths(&self) -> &VmAccessiblePaths {
        &self.accessible_paths
    }

    pub fn get_accessible_path_from_inner(&self, inner_path: impl AsRef<Path>) -> PathBuf {
        self.vmm_process.inner_to_outer_path(inner_path)
    }

    pub fn register_vsock_listener_path(&mut self, listener_path: impl Into<PathBuf>) {
        self.accessible_paths.vsock_listener_paths.push(listener_path.into());
    }

    pub(super) fn ensure_state(&mut self, expected_state: VmState) -> Result<(), VmError> {
        let current_state = self.state();
        if current_state != expected_state {
            Err(VmError::ExpectedState {
                expected: expected_state,
                actual: current_state,
            })
        } else {
            Ok(())
        }
    }

    pub(super) fn ensure_paused_or_running(&mut self) -> Result<(), VmError> {
        let current_state = self.state();
        if current_state != VmState::Running && current_state != VmState::Paused {
            Err(VmError::ExpectedPausedOrRunning { actual: current_state })
        } else {
            Ok(())
        }
    }

    fn ensure_exited_or_crashed(&mut self) -> Result<(), VmError> {
        let current_state = self.state();
        if let VmState::Crashed(_) = current_state {
            return Ok(());
        }
        if current_state == VmState::Exited {
            return Ok(());
        }
        Err(VmError::ExpectedExitedOrCrashed { actual: current_state })
    }
}

async fn prepare_file(path: &PathBuf, only_tree: bool) -> Result<(), VmError> {
    if let Some(parent_path) = path.parent() {
        fs::create_dir_all(parent_path).await.map_err(VmError::IoError)?;
    }

    if !only_tree {
        fs::File::create(path).await.map_err(VmError::IoError)?;
    }

    Ok(())
}
