use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    process::ExitStatus,
    sync::Arc,
    time::Duration,
};

use crate::{
    executor::{
        arguments::FirecrackerConfigOverride, installation::FirecrackerInstallation, VmmExecutor,
    },
    shell_spawner::ShellSpawner,
    vmm_process::{
        HyperResponseExt, VmmProcess, VmmProcessError, VmmProcessPipes, VmmProcessState,
    },
};
use bytes::Bytes;
use configuration::{NewVmConfigurationApplier, VmConfiguration};
use http::StatusCode;
use http_body_util::Full;
use hyper::Request;
use models::{
    VmAction, VmActionType, VmApiError, VmBalloonStatistics, VmCreateSnapshot,
    VmFetchedConfiguration, VmFirecrackerVersion, VmInfo, VmSnapshotPaths, VmStandardPaths,
    VmStateForUpdate, VmUpdateBalloon, VmUpdateBalloonStatistics, VmUpdateDrive,
    VmUpdateNetworkInterface, VmUpdateState,
};
use serde::{de::DeserializeOwned, Serialize};
use tokio::{fs, io::AsyncWriteExt, process::ChildStdin};

pub mod configuration;
pub mod models;

#[derive(Debug)]
pub struct Vm<E: VmmExecutor, S: ShellSpawner> {
    vmm_process: VmmProcess<E, S>,
    is_paused: bool,
    configuration: Option<VmConfiguration>,
    accessible_paths: VmStandardPaths,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VmState {
    NotStarted,
    Running,
    Paused,
    Exited,
    Crashed(ExitStatus),
}

#[derive(Debug)]
pub enum VmError {
    ProcessError(VmmProcessError),
    ExpectedState {
        expected: Vec<VmState>,
        actual: VmState,
    },
    ExpectedExitedOrCrashed {
        actual: VmState,
    },
    SerdeError(serde_json::Error),
    IoError(tokio::io::Error),
    ApiRespondedWithFault {
        status_code: StatusCode,
        fault_message: String,
    },
    ApiRequestNotConstructed(http::Error),
    ApiResponseCouldNotBeReceived(hyper::Error),
    ApiResponseExpectedEmpty,
    NoShutdownMethodsSpecified,
    ShutdownApplicationTimedOut,
    ShutdownWaitTimedOut,
    SocketWaitTimedOut,
    DisabledApiSocketIsUnsupported,
    MissingPathMapping,
}

#[derive(Debug)]
pub enum VmShutdownMethod {
    CtrlAltDel,
    PauseThenKill,
    WriteRebootToStdin(ChildStdin),
    Kill,
}

impl<E: VmmExecutor, S: ShellSpawner> Vm<E, S> {
    pub async fn prepare(
        executor: E,
        shell_spawner: S,
        installation: FirecrackerInstallation,
        configuration: VmConfiguration,
    ) -> Result<Self, VmError> {
        Self::prepare_arced(
            executor,
            Arc::new(shell_spawner),
            Arc::new(installation),
            configuration,
        )
        .await
    }

    pub async fn prepare_arced(
        executor: E,
        shell_spawner_arc: Arc<S>,
        installation_arc: Arc<FirecrackerInstallation>,
        mut configuration: VmConfiguration,
    ) -> Result<Self, VmError> {
        if executor.get_outer_socket_path().is_none() {
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
        let mut vm_process =
            VmmProcess::new_arced(executor, shell_spawner_arc, installation_arc, outer_paths);
        let mut path_mappings = vm_process.prepare().await.map_err(VmError::ProcessError)?;

        // set inner paths for configuration (to conform to FC expectations) based on returned mappings
        match configuration {
            VmConfiguration::New(ref mut config) => {
                config.boot_source.kernel_image_path = path_mappings
                    .remove(&config.boot_source.kernel_image_path)
                    .ok_or(VmError::MissingPathMapping)?;

                if let Some(ref mut path) = config.boot_source.initrd_path {
                    config.boot_source.initrd_path = Some(
                        path_mappings
                            .remove(path)
                            .ok_or(VmError::MissingPathMapping)?,
                    );
                }

                for drive in &mut config.drives {
                    if let Some(ref mut path) = drive.path_on_host {
                        *path = path_mappings
                            .remove(path)
                            .ok_or(VmError::MissingPathMapping)?;
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
            vsock_uds_path: None,
        };
        match configuration {
            VmConfiguration::New(ref config) => {
                for drive in &config.drives {
                    if let Some(ref socket) = drive.socket {
                        accessible_paths.drive_sockets.insert(
                            drive.drive_id.clone(),
                            vm_process.inner_to_outer_path(&socket),
                        );
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
                    accessible_paths.vsock_uds_path = Some(new_uds_path);
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
            accessible_paths,
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
            if let NewVmConfigurationApplier::ViaJsonConfiguration(inner_path) =
                config.get_applier()
            {
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
        .map_err(|_| VmError::SocketWaitTimedOut)?
        .map_err(VmError::IoError)?;

        match &configuration {
            VmConfiguration::New(config) => {
                if no_api_calls {
                    return Ok(());
                }

                self.send_req("/boot-source", "PUT", Some(&config.boot_source))
                    .await?;

                for drive in &config.drives {
                    self.send_req(
                        format!("/drives/{}", drive.drive_id).as_str(),
                        "PUT",
                        Some(drive),
                    )
                    .await?;
                }

                self.send_req(
                    "/machine-config",
                    "PUT",
                    Some(&config.machine_configuration),
                )
                .await?;

                for network_interface in &config.network_interfaces {
                    self.send_req(
                        format!("/network-interfaces/{}", network_interface.iface_id).as_str(),
                        "PUT",
                        Some(network_interface),
                    )
                    .await?;
                }

                if let Some(balloon) = &config.balloon {
                    self.send_req("/balloon", "PUT", Some(balloon)).await?;
                }

                if let Some(vsock) = &config.vsock {
                    self.send_req("/vsock", "PUT", Some(vsock)).await?;
                }

                if let Some(logger) = &config.logger {
                    self.send_req("/logger", "PUT", Some(logger)).await?;
                }

                if let Some(metrics) = &config.metrics {
                    self.send_req("/metrics", "PUT", Some(metrics)).await?;
                }

                if let Some(mmds_configuration) = &config.mmds_configuration {
                    self.send_req("/mmds/config", "PUT", Some(mmds_configuration))
                        .await?;
                }

                if let Some(entropy) = &config.entropy {
                    self.send_req("/entropy", "PUT", Some(entropy)).await?;
                }

                self.send_req(
                    "/actions",
                    "PUT",
                    Some(VmAction {
                        action_type: VmActionType::InstanceStart,
                    }),
                )
                .await?;
            }
            VmConfiguration::FromSnapshot(config) => {
                if let Some(logger) = &config.logger {
                    self.send_req("/logger", "PUT", Some(logger)).await?;
                }
                if let Some(metrics) = &config.metrics {
                    self.send_req("/metrics", "PUT", Some(metrics)).await?;
                }
                self.send_req("/snapshot/load", "PUT", Some(&config.load_snapshot))
                    .await?;
            }
        }

        Ok(())
    }

    pub async fn shutdown(
        &mut self,
        shutdown_methods: Vec<VmShutdownMethod>,
        timeout: Duration,
    ) -> Result<(), VmError> {
        self.ensure_state(VmState::Running)?;
        let mut last_result = Ok(());

        for shutdown_method in shutdown_methods {
            last_result = match shutdown_method {
                VmShutdownMethod::CtrlAltDel => self
                    .vmm_process
                    .send_ctrl_alt_del()
                    .await
                    .map_err(VmError::ProcessError),
                VmShutdownMethod::PauseThenKill => {
                    self.send_req(
                        "/vm",
                        "PATCH",
                        Some(VmUpdateState {
                            state: VmStateForUpdate::Paused,
                        }),
                    )
                    .await
                }
                VmShutdownMethod::WriteRebootToStdin(mut stdin) => {
                    stdin
                        .write_all(b"reboot\n")
                        .await
                        .map_err(VmError::IoError)?;
                    stdin.flush().await.map_err(VmError::IoError)
                }
                VmShutdownMethod::Kill => self
                    .vmm_process
                    .send_sigkill()
                    .map_err(VmError::ProcessError),
            };

            if last_result.is_ok() {
                last_result = tokio::time::timeout(timeout, self.vmm_process.wait_for_exit())
                    .await
                    .map_err(|_| VmError::ShutdownWaitTimedOut)
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
        self.vmm_process
            .cleanup()
            .await
            .map_err(VmError::ProcessError)
    }

    pub fn take_pipes(&mut self) -> Result<VmmProcessPipes, VmError> {
        self.ensure_states(vec![VmState::Paused, VmState::Running])?;
        self.vmm_process.take_pipes().map_err(VmError::ProcessError)
    }

    pub fn get_standard_paths(&self) -> &VmStandardPaths {
        &self.accessible_paths
    }

    pub fn get_path(&self, inner_path: impl AsRef<Path>) -> PathBuf {
        self.vmm_process.inner_to_outer_path(inner_path)
    }

    pub async fn get_info(&mut self) -> Result<VmInfo, VmError> {
        self.ensure_paused_or_running()?;
        self.send_req_with_resp("/", "GET", None::<i32>).await
    }

    pub async fn flush_metrics(&mut self) -> Result<(), VmError> {
        self.ensure_paused_or_running()?;
        self.send_req(
            "/actions",
            "PUT",
            Some(VmAction {
                action_type: VmActionType::FlushMetrics,
            }),
        )
        .await
    }

    pub async fn update_balloon(&mut self, update_balloon: VmUpdateBalloon) -> Result<(), VmError> {
        self.ensure_paused_or_running()?;
        self.send_req("/balloon", "PATCH", Some(update_balloon))
            .await
    }

    pub async fn get_balloon_statistics(&mut self) -> Result<VmBalloonStatistics, VmError> {
        self.ensure_state(VmState::Running)?;
        self.send_req_with_resp("/balloon/statistics", "GET", None::<i32>)
            .await
    }

    pub async fn update_balloon_statistics(
        &mut self,
        update_balloon_statistics: VmUpdateBalloonStatistics,
    ) -> Result<(), VmError> {
        self.ensure_paused_or_running()?;
        self.send_req(
            "/balloon/statistics",
            "PATCH",
            Some(update_balloon_statistics),
        )
        .await
    }

    pub async fn update_drive(&mut self, update_drive: VmUpdateDrive) -> Result<(), VmError> {
        self.ensure_paused_or_running()?;
        self.send_req(
            format!("/drives/{}", update_drive.drive_id).as_str(),
            "PATCH",
            Some(update_drive),
        )
        .await
    }

    pub async fn update_network_interface(
        &mut self,
        update_network_interface: VmUpdateNetworkInterface,
    ) -> Result<(), VmError> {
        self.ensure_paused_or_running()?;
        self.send_req(
            format!("/network-interfaces/{}", update_network_interface.iface_id).as_str(),
            "PATCH",
            Some(update_network_interface),
        )
        .await
    }

    pub async fn create_snapshot(
        &mut self,
        create_snapshot: VmCreateSnapshot,
    ) -> Result<VmSnapshotPaths, VmError> {
        self.ensure_state(VmState::Paused)?;
        self.send_req("/snapshot/create", "PUT", Some(&create_snapshot))
            .await?;
        Ok(VmSnapshotPaths::new(
            self.vmm_process
                .inner_to_outer_path(create_snapshot.snapshot_path),
            self.vmm_process
                .inner_to_outer_path(create_snapshot.mem_file_path),
        ))
    }

    pub async fn get_firecracker_version(&mut self) -> Result<VmFirecrackerVersion, VmError> {
        self.ensure_paused_or_running()?;
        self.send_req_with_resp("/version", "GET", None::<i32>)
            .await
    }

    pub async fn fetch_configuration_from_vm_process(
        &mut self,
    ) -> Result<VmFetchedConfiguration, VmError> {
        self.ensure_paused_or_running()?;
        self.send_req_with_resp("/vm/config", "GET", None::<i32>)
            .await
    }

    pub async fn pause(&mut self) -> Result<(), VmError> {
        self.ensure_state(VmState::Running)?;
        self.send_req(
            "/vm",
            "PATCH",
            Some(VmUpdateState {
                state: VmStateForUpdate::Paused,
            }),
        )
        .await
    }

    pub async fn resume(&mut self) -> Result<(), VmError> {
        self.ensure_state(VmState::Paused)?;
        self.send_req(
            "/vm",
            "PATCH",
            Some(VmUpdateState {
                state: VmStateForUpdate::Resumed,
            }),
        )
        .await
    }

    fn ensure_state(&mut self, expected_state: VmState) -> Result<(), VmError> {
        self.ensure_states(vec![expected_state])
    }

    fn ensure_paused_or_running(&mut self) -> Result<(), VmError> {
        self.ensure_states(vec![VmState::Running, VmState::Paused])
    }

    fn ensure_states(&mut self, expected_states: Vec<VmState>) -> Result<(), VmError> {
        let state = self.state();
        if !expected_states.contains(&state) {
            return Err(VmError::ExpectedState {
                expected: expected_states,
                actual: state,
            });
        }
        Ok(())
    }

    fn ensure_exited_or_crashed(&mut self) -> Result<(), VmError> {
        let state = self.state();
        if let VmState::Crashed(_) = state {
            return Ok(());
        }
        if state == VmState::Exited {
            return Ok(());
        }
        Err(VmError::ExpectedExitedOrCrashed { actual: state })
    }

    async fn send_req(
        &mut self,
        route: &str,
        method: &str,
        request_body: Option<impl Serialize>,
    ) -> Result<(), VmError> {
        let response_json: String = self.send_req_core(route, method, request_body).await?;
        if response_json.trim().is_empty() {
            Ok(())
        } else {
            Err(VmError::ApiResponseExpectedEmpty)
        }
    }

    async fn send_req_with_resp<Resp: DeserializeOwned>(
        &mut self,
        route: &str,
        method: &str,
        request_body: Option<impl Serialize>,
    ) -> Result<Resp, VmError> {
        let response_json = self.send_req_core(route, method, request_body).await?;
        serde_json::from_str(&response_json).map_err(VmError::SerdeError)
    }

    async fn send_req_core(
        &mut self,
        route: &str,
        method: &str,
        request_body: Option<impl Serialize>,
    ) -> Result<String, VmError> {
        let request_builder = Request::builder().method(method);
        let request = match request_body {
            Some(body) => {
                let request_json = serde_json::to_string(&body).map_err(VmError::SerdeError)?;
                request_builder
                    .header("Content-Type", "application/json")
                    .body(Full::new(Bytes::from(request_json)))
            }
            None => request_builder.body(Full::new(Bytes::new())),
        }
        .map_err(VmError::ApiRequestNotConstructed)?;
        let mut response = self
            .vmm_process
            .send_api_request(route, request)
            .await
            .map_err(VmError::ProcessError)?;
        let response_json = response
            .recv_to_string()
            .await
            .map_err(VmError::ApiResponseCouldNotBeReceived)?;

        if !response.status().is_success() {
            let api_error: VmApiError =
                serde_json::from_str(&response_json).map_err(VmError::SerdeError)?;
            return Err(VmError::ApiRespondedWithFault {
                status_code: response.status(),
                fault_message: api_error.fault_message,
            });
        }

        Ok(response_json)
    }
}

async fn prepare_file(path: &PathBuf, only_tree: bool) -> Result<(), VmError> {
    if let Some(parent_path) = path.parent() {
        fs::create_dir_all(parent_path)
            .await
            .map_err(VmError::IoError)?;
    }

    if !only_tree {
        fs::File::create(path).await.map_err(VmError::IoError)?;
    }

    Ok(())
}
