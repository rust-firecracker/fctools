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
use configuration::{NewVmConfigurationApplier, VmConfiguration};
use http::StatusCode;
use paths::VmStandardPaths;
use tokio::{fs, io::AsyncWriteExt, process::ChildStdin};

pub mod api;
pub mod configuration;
pub mod models;
pub mod paths;

#[derive(Debug)]
pub struct Vm<E: VmmExecutor, S: ShellSpawner> {
    vmm_process: VmmProcess<E, S>,
    is_paused: bool,
    configuration: Option<VmConfiguration>,
    standard_paths: VmStandardPaths,
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
    WriteRebootToStdin(ChildStdin),
    Kill,
}

#[derive(Debug, Clone, PartialEq, PartialOrd, Hash)]
pub struct VmCleanupOptions {
    remove_logs: bool,
    remove_metrics: bool,
    remove_vsock: bool,
}

impl VmCleanupOptions {
    pub fn new() -> Self {
        Self {
            remove_logs: false,
            remove_metrics: false,
            remove_vsock: false,
        }
    }

    pub fn remove_all(mut self) -> Self {
        self.remove_logs = true;
        self.remove_metrics = true;
        self.remove_vsock = true;
        self
    }

    pub fn remove_logs(mut self) -> Self {
        self.remove_logs = true;
        self
    }

    pub fn remove_metrics(mut self) -> Self {
        self.remove_metrics = true;
        self
    }

    pub fn remove_vsock(mut self) -> Self {
        self.remove_vsock = true;
        self
    }
}

impl<E: VmmExecutor, S: ShellSpawner> Vm<E, S> {
    pub async fn prepare(
        executor: E,
        shell_spawner: S,
        installation: FirecrackerInstallation,
        configuration: VmConfiguration,
    ) -> Result<Self, VmError> {
        Self::prepare_arced(executor, Arc::new(shell_spawner), Arc::new(installation), configuration).await
    }

    pub async fn prepare_arced(
        executor: E,
        shell_spawner_arc: Arc<S>,
        installation_arc: Arc<FirecrackerInstallation>,
        mut configuration: VmConfiguration,
    ) -> Result<Self, VmError> {
        if executor.get_socket_path().is_none() {
            return Err(VmError::DisabledApiSocketIsUnsupported);
        }

        // compute outer paths from configuration
        let mut outer_paths = Vec::new();
        match configuration {
            VmConfiguration::New(ref config) => {
                outer_paths.push(config.boot_source.kernel_image_path.clone());

                if let Some(ref path) = config.boot_source.initrd_path {
                    outer_paths.push(path.clone());
                }

                for drive in &config.drives {
                    if let Some(ref path) = drive.path_on_host {
                        outer_paths.push(path.clone());
                    }
                }
            }
            VmConfiguration::FromSnapshot(ref config) => {
                outer_paths.push(config.load_snapshot.snapshot_path.clone());
                outer_paths.push(config.load_snapshot.mem_backend.backend_path.clone());
            }
        }

        // prepare
        let mut vm_process = VmmProcess::new_arced(executor, shell_spawner_arc, installation_arc, outer_paths);
        let mut path_mappings = vm_process.prepare().await.map_err(VmError::ProcessError)?;

        // set inner paths for configuration (to conform to FC expectations) based on returned mappings
        match configuration {
            VmConfiguration::New(ref mut config) => {
                config.boot_source.kernel_image_path = path_mappings
                    .remove(&config.boot_source.kernel_image_path)
                    .ok_or(VmError::MissingPathMapping)?;

                if let Some(ref mut path) = config.boot_source.initrd_path {
                    config.boot_source.initrd_path =
                        Some(path_mappings.remove(path).ok_or(VmError::MissingPathMapping)?);
                }

                for drive in &mut config.drives {
                    if let Some(ref mut path) = drive.path_on_host {
                        *path = path_mappings.remove(path).ok_or(VmError::MissingPathMapping)?;
                    }
                }
            }
            VmConfiguration::FromSnapshot(ref mut config) => {
                config.load_snapshot.snapshot_path = path_mappings
                    .remove(&config.load_snapshot.snapshot_path)
                    .ok_or(VmError::MissingPathMapping)?;
                config.load_snapshot.mem_backend.backend_path = path_mappings
                    .remove(&config.load_snapshot.mem_backend.backend_path)
                    .ok_or(VmError::MissingPathMapping)?;
            }
        };

        // generate accessible paths, ensure paths exist
        let mut accessible_paths = VmStandardPaths {
            drive_sockets: HashMap::new(),
            metrics_path: None,
            log_path: None,
            vsock_multiplexer_path: None,
            vsock_listener_paths: Vec::new(),
        };
        match configuration {
            VmConfiguration::New(ref config) => {
                for drive in &config.drives {
                    if let Some(ref socket) = drive.socket {
                        accessible_paths
                            .drive_sockets
                            .insert(drive.drive_id.clone(), vm_process.inner_to_outer_path(&socket));
                    }
                }

                if let Some(ref logger) = config.logger {
                    if let Some(ref log_path) = logger.log_path {
                        let new_log_path = vm_process.inner_to_outer_path(log_path);
                        prepare_file(&new_log_path, false).await?;
                        accessible_paths.log_path = Some(new_log_path);
                    }
                }

                if let Some(ref metrics) = config.metrics {
                    let new_metrics_path = vm_process.inner_to_outer_path(&metrics.metrics_path);
                    prepare_file(&new_metrics_path, false).await?;
                    accessible_paths.metrics_path = Some(new_metrics_path);
                }

                if let Some(ref vsock) = config.vsock {
                    let new_uds_path = vm_process.inner_to_outer_path(&vsock.uds_path);
                    prepare_file(&new_uds_path, true).await?;
                    accessible_paths.vsock_multiplexer_path = Some(new_uds_path);
                }
            }
            VmConfiguration::FromSnapshot(ref config) => {
                if let Some(ref logger) = config.logger {
                    if let Some(ref log_path) = logger.log_path {
                        let new_log_path = vm_process.inner_to_outer_path(log_path);
                        prepare_file(&new_log_path, false).await?;
                        accessible_paths.log_path = Some(new_log_path);
                    }
                }

                if let Some(ref metrics) = config.metrics {
                    let new_metrics_path = vm_process.inner_to_outer_path(&metrics.metrics_path);
                    prepare_file(&new_metrics_path, false).await?;
                    accessible_paths.metrics_path = Some(new_metrics_path);
                }
            }
        };

        Ok(Self {
            vmm_process: vm_process,
            is_paused: false,
            configuration: Some(configuration),
            standard_paths: accessible_paths,
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
        let mut no_api_calls = false;
        if let VmConfiguration::New(ref config) = configuration {
            if let NewVmConfigurationApplier::ViaJsonConfiguration(inner_path) = config.get_applier() {
                config_override = FirecrackerConfigOverride::Enable(inner_path.clone());
                prepare_file(inner_path, true).await?;
                fs::write(
                    self.vmm_process.inner_to_outer_path(inner_path),
                    serde_json::to_string(config).map_err(VmError::SerdeError)?,
                )
                .await
                .map_err(VmError::IoError)?;
                no_api_calls = true;
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
            Ok(())
        })
        .await
        .map_err(|_| VmError::Timeout)?
        .map_err(VmError::IoError)?;

        match &configuration {
            VmConfiguration::New(config) => {
                if !no_api_calls {
                    api::init_new(self, config).await?;
                }
            }
            VmConfiguration::FromSnapshot(config) => {
                api::init_from_snapshot(self, config).await?;
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
                VmShutdownMethod::PauseThenKill => self.api_pause().await,
                VmShutdownMethod::WriteRebootToStdin(mut stdin) => {
                    match stdin.write_all(b"reboot\n").await.map_err(VmError::IoError) {
                        Ok(()) => stdin.flush().await.map_err(VmError::IoError),
                        Err(err) => Err(err),
                    }
                }
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

    pub async fn cleanup(&mut self, cleanup_options: VmCleanupOptions) -> Result<(), VmError> {
        self.ensure_exited_or_crashed()?;
        self.vmm_process.cleanup().await.map_err(VmError::ProcessError)?;

        if let Some(ref log_path) = self.standard_paths.log_path {
            if cleanup_options.remove_logs {
                tokio::fs::remove_file(log_path).await.map_err(VmError::IoError)?;
            }
        }

        if let Some(ref metrics_path) = self.standard_paths.metrics_path {
            if cleanup_options.remove_metrics {
                tokio::fs::remove_file(metrics_path).await.map_err(VmError::IoError)?;
            }
        }

        if let Some(ref multiplexer_path) = self.standard_paths.vsock_multiplexer_path {
            if cleanup_options.remove_vsock {
                tokio::fs::remove_file(multiplexer_path)
                    .await
                    .map_err(VmError::IoError)?;

                for listener_path in &self.standard_paths.vsock_listener_paths {
                    tokio::fs::remove_file(listener_path).await.map_err(VmError::IoError)?;
                }
            }
        }

        Ok(())
    }

    pub fn take_pipes(&mut self) -> Result<VmmProcessPipes, VmError> {
        self.ensure_paused_or_running()?;
        self.vmm_process.take_pipes().map_err(VmError::ProcessError)
    }

    pub fn standard_paths(&self) -> &VmStandardPaths {
        &self.standard_paths
    }

    pub fn standard_paths_mut(&mut self) -> &mut VmStandardPaths {
        &mut self.standard_paths
    }

    pub fn inner_to_outer_path(&self, inner_path: impl AsRef<Path>) -> PathBuf {
        self.vmm_process.inner_to_outer_path(inner_path)
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
