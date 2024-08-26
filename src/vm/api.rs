use async_trait::async_trait;
use bytes::Bytes;
use http::{Request, Response};
use http_body_util::Full;
use hyper::body::Incoming;
use serde::{de::DeserializeOwned, Serialize};

use crate::{executor::VmmExecutor, process::HyperResponseExt, shell_spawner::ShellSpawner};

use super::{
    configuration::{FromSnapshotVmConfiguration, NewVmConfiguration},
    models::{
        VmAction, VmActionType, VmApiError, VmBalloon, VmBalloonStatistics, VmCreateSnapshot, VmEffectiveConfiguration,
        VmFirecrackerVersion, VmInfo, VmMachineConfiguration, VmUpdateBalloon, VmUpdateBalloonStatistics,
        VmUpdateDrive, VmUpdateNetworkInterface, VmUpdateState, VmUpdatedState,
    },
    paths::VmSnapshotPaths,
    Vm, VmError, VmState,
};

#[async_trait]
pub trait VmApi {
    async fn api_custom_request(
        &mut self,
        route: impl AsRef<str> + Send,
        request: Request<Full<Bytes>>,
        new_is_paused: Option<bool>,
    ) -> Result<Response<Incoming>, VmError>;

    async fn api_get_info(&mut self) -> Result<VmInfo, VmError>;

    async fn api_flush_metrics(&mut self) -> Result<(), VmError>;

    async fn api_get_balloon(&mut self) -> Result<VmBalloon, VmError>;

    async fn api_update_balloon(&mut self, update_balloon: VmUpdateBalloon) -> Result<(), VmError>;

    async fn api_get_balloon_statistics(&mut self) -> Result<VmBalloonStatistics, VmError>;

    async fn api_update_balloon_statistics(
        &mut self,
        update_balloon_statistics: VmUpdateBalloonStatistics,
    ) -> Result<(), VmError>;

    async fn api_update_drive(&mut self, update_drive: VmUpdateDrive) -> Result<(), VmError>;

    async fn api_update_network_interface(
        &mut self,
        update_network_interface: VmUpdateNetworkInterface,
    ) -> Result<(), VmError>;

    async fn api_get_machine_configuration(&mut self) -> Result<VmMachineConfiguration, VmError>;

    async fn api_create_snapshot(&mut self, create_snapshot: VmCreateSnapshot) -> Result<VmSnapshotPaths, VmError>;

    async fn api_get_firecracker_version(&mut self) -> Result<String, VmError>;

    async fn api_get_effective_configuration(&mut self) -> Result<VmEffectiveConfiguration, VmError>;

    async fn api_pause(&mut self) -> Result<(), VmError>;

    async fn api_resume(&mut self) -> Result<(), VmError>;

    async fn api_create_mmds(&mut self, value: &serde_json::Value) -> Result<(), VmError>;

    async fn api_update_mmds(&mut self, value: &serde_json::Value) -> Result<(), VmError>;

    async fn api_get_mmds(&mut self) -> Result<serde_json::Value, VmError>;
}

#[async_trait]
impl<E: VmmExecutor, S: ShellSpawner> VmApi for Vm<E, S> {
    async fn api_custom_request(
        &mut self,
        route: impl AsRef<str> + Send,
        request: Request<Full<Bytes>>,
        new_is_paused: Option<bool>,
    ) -> Result<Response<Incoming>, VmError> {
        self.ensure_paused_or_running()?;
        let response = self
            .vmm_process
            .send_api_request(route, request)
            .await
            .map_err(VmError::ProcessError)?;
        if let Some(new_is_paused) = new_is_paused {
            self.is_paused = new_is_paused;
        }
        Ok(response)
    }

    async fn api_get_info(&mut self) -> Result<VmInfo, VmError> {
        self.ensure_paused_or_running()?;
        send_api_request_with_response(self, "/", "GET", None::<i32>).await
    }

    async fn api_flush_metrics(&mut self) -> Result<(), VmError> {
        self.ensure_paused_or_running()?;
        send_api_request(
            self,
            "/actions",
            "PUT",
            Some(VmAction {
                action_type: VmActionType::FlushMetrics,
            }),
        )
        .await
    }

    async fn api_get_balloon(&mut self) -> Result<VmBalloon, VmError> {
        self.ensure_paused_or_running()?;
        send_api_request_with_response(self, "/balloon", "GET", None::<i32>).await
    }

    async fn api_update_balloon(&mut self, update_balloon: VmUpdateBalloon) -> Result<(), VmError> {
        self.ensure_paused_or_running()?;
        send_api_request(self, "/balloon", "PATCH", Some(update_balloon)).await
    }

    async fn api_get_balloon_statistics(&mut self) -> Result<VmBalloonStatistics, VmError> {
        self.ensure_state(VmState::Running)?;
        send_api_request_with_response(self, "/balloon/statistics", "GET", None::<i32>).await
    }

    async fn api_update_balloon_statistics(
        &mut self,
        update_balloon_statistics: VmUpdateBalloonStatistics,
    ) -> Result<(), VmError> {
        self.ensure_paused_or_running()?;
        send_api_request(self, "/balloon/statistics", "PATCH", Some(update_balloon_statistics)).await
    }

    async fn api_update_drive(&mut self, update_drive: VmUpdateDrive) -> Result<(), VmError> {
        self.ensure_paused_or_running()?;
        send_api_request(
            self,
            format!("/drives/{}", update_drive.drive_id).as_str(),
            "PATCH",
            Some(update_drive),
        )
        .await
    }

    async fn api_update_network_interface(
        &mut self,
        update_network_interface: VmUpdateNetworkInterface,
    ) -> Result<(), VmError> {
        self.ensure_paused_or_running()?;
        send_api_request(
            self,
            format!("/network-interfaces/{}", update_network_interface.iface_id).as_str(),
            "PATCH",
            Some(update_network_interface),
        )
        .await
    }

    async fn api_get_machine_configuration(&mut self) -> Result<VmMachineConfiguration, VmError> {
        self.ensure_paused_or_running()?;
        send_api_request_with_response(self, "/machine-config", "GET", None::<i32>).await
    }

    async fn api_create_snapshot(&mut self, create_snapshot: VmCreateSnapshot) -> Result<VmSnapshotPaths, VmError> {
        self.ensure_state(VmState::Paused)?;
        send_api_request(self, "/snapshot/create", "PUT", Some(&create_snapshot)).await?;
        Ok(VmSnapshotPaths {
            snapshot_path: self.vmm_process.inner_to_outer_path(create_snapshot.snapshot_path),
            mem_file_path: self.vmm_process.inner_to_outer_path(create_snapshot.mem_file_path),
        })
    }

    async fn api_get_firecracker_version(&mut self) -> Result<String, VmError> {
        self.ensure_paused_or_running()?;
        Ok(
            send_api_request_with_response::<VmFirecrackerVersion, _, _>(self, "/version", "GET", None::<i32>)
                .await?
                .firecracker_version,
        )
    }

    async fn api_get_effective_configuration(&mut self) -> Result<VmEffectiveConfiguration, VmError> {
        self.ensure_paused_or_running()?;
        send_api_request_with_response(self, "/vm/config", "GET", None::<i32>).await
    }

    async fn api_pause(&mut self) -> Result<(), VmError> {
        self.ensure_state(VmState::Running)?;
        send_api_request(
            self,
            "/vm",
            "PATCH",
            Some(VmUpdateState {
                state: VmUpdatedState::Paused,
            }),
        )
        .await?;
        self.is_paused = true;
        Ok(())
    }

    async fn api_resume(&mut self) -> Result<(), VmError> {
        self.ensure_state(VmState::Paused)?;
        send_api_request(
            self,
            "/vm",
            "PATCH",
            Some(VmUpdateState {
                state: VmUpdatedState::Resumed,
            }),
        )
        .await?;
        self.is_paused = false;
        Ok(())
    }

    async fn api_create_mmds(&mut self, value: &serde_json::Value) -> Result<(), VmError> {
        self.ensure_paused_or_running()?;
        send_api_request(self, "/mmds", "PUT", Some(value)).await
    }

    async fn api_update_mmds(&mut self, value: &serde_json::Value) -> Result<(), VmError> {
        self.ensure_paused_or_running()?;
        send_api_request(self, "/mmds", "PATCH", Some(value)).await
    }

    async fn api_get_mmds(&mut self) -> Result<serde_json::Value, VmError> {
        self.ensure_paused_or_running()?;
        send_api_request_with_response(self, "/mmds", "GET", None::<i32>).await
    }
}

pub(super) async fn init_new<E: VmmExecutor, S: ShellSpawner>(
    vm: &mut Vm<E, S>,
    configuration: &NewVmConfiguration,
) -> Result<(), VmError> {
    send_api_request(vm, "/boot-source", "PUT", Some(&configuration.boot_source)).await?;

    for drive in &configuration.drives {
        send_api_request(vm, format!("/drives/{}", drive.drive_id).as_str(), "PUT", Some(drive)).await?;
    }

    send_api_request(vm, "/machine-config", "PUT", Some(&configuration.machine_configuration)).await?;

    if let Some(ref cpu_template) = configuration.cpu_template {
        send_api_request(vm, "/cpu-config", "PUT", Some(cpu_template)).await?;
    }

    for network_interface in &configuration.network_interfaces {
        send_api_request(
            vm,
            format!("/network-interfaces/{}", network_interface.iface_id).as_str(),
            "PUT",
            Some(network_interface),
        )
        .await?;
    }

    if let Some(ref balloon) = configuration.balloon {
        send_api_request(vm, "/balloon", "PUT", Some(balloon)).await?;
    }

    if let Some(ref vsock) = configuration.vsock {
        send_api_request(vm, "/vsock", "PUT", Some(vsock)).await?;
    }

    if let Some(ref logger) = configuration.logger {
        send_api_request(vm, "/logger", "PUT", Some(logger)).await?;
    }

    if let Some(ref metrics) = configuration.metrics_system {
        send_api_request(vm, "/metrics", "PUT", Some(metrics)).await?;
    }

    if let Some(ref mmds_configuration) = configuration.mmds_configuration {
        send_api_request(vm, "/mmds/config", "PUT", Some(mmds_configuration)).await?;
    }

    if let Some(ref entropy) = configuration.entropy {
        send_api_request(vm, "/entropy", "PUT", Some(entropy)).await?;
    }

    send_api_request(
        vm,
        "/actions",
        "PUT",
        Some(VmAction {
            action_type: VmActionType::InstanceStart,
        }),
    )
    .await
}

pub(super) async fn init_from_snapshot<E: VmmExecutor, S: ShellSpawner>(
    vm: &mut Vm<E, S>,
    configuration: &FromSnapshotVmConfiguration,
) -> Result<(), VmError> {
    if let Some(ref logger) = configuration.logger {
        send_api_request(vm, "/logger", "PUT", Some(logger)).await?;
    }

    if let Some(ref metrics) = configuration.metrics {
        send_api_request(vm, "/metrics", "PUT", Some(metrics)).await?;
    }

    send_api_request(vm, "/snapshot/load", "PUT", Some(&configuration.load_snapshot)).await
}

async fn send_api_request<E: VmmExecutor, S: ShellSpawner>(
    vm: &mut Vm<E, S>,
    route: &str,
    method: &str,
    request_body: Option<impl Serialize>,
) -> Result<(), VmError> {
    let response_json: String = send_api_request_internal(vm, route, method, request_body).await?;
    if response_json.trim().is_empty() {
        Ok(())
    } else {
        Err(VmError::ApiResponseExpectedEmpty(response_json))
    }
}

async fn send_api_request_with_response<Resp: DeserializeOwned, E: VmmExecutor, S: ShellSpawner>(
    vm: &mut Vm<E, S>,
    route: &str,
    method: &str,
    request_body: Option<impl Serialize>,
) -> Result<Resp, VmError> {
    let response_json = send_api_request_internal(vm, route, method, request_body).await?;
    serde_json::from_str(&response_json).map_err(VmError::SerdeError)
}

async fn send_api_request_internal<E: VmmExecutor, S: ShellSpawner>(
    vm: &mut Vm<E, S>,
    route: &str,
    method: &str,
    request_body: Option<impl Serialize>,
) -> Result<String, VmError> {
    let request_builder = Request::builder().method(method);
    let request = match request_body {
        Some(body) => {
            let request_json = serde_json::to_string(&body).map_err(VmError::SerdeError)?;
            println!("To \"{route}\" with: {request_json}");
            request_builder
                .header("Content-Type", "application/json")
                .body(Full::new(Bytes::from(request_json)))
        }
        None => request_builder.body(Full::new(Bytes::new())),
    }
    .map_err(VmError::ApiRequestNotConstructed)?;
    let mut response = vm
        .vmm_process
        .send_api_request(route, request)
        .await
        .map_err(VmError::ProcessError)?;
    let response_json = response
        .recv_to_string()
        .await
        .map_err(VmError::ApiResponseCouldNotBeReceived)?;

    if !response.status().is_success() {
        let api_error: VmApiError = serde_json::from_str(&response_json).map_err(VmError::SerdeError)?;
        return Err(VmError::ApiRespondedWithFault {
            status_code: response.status(),
            fault_message: api_error.fault_message,
        });
    }

    Ok(response_json)
}
