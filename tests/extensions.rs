use std::{os::unix::fs::FileTypeExt, time::Duration};

use bytes::Bytes;
use fctools::{
    extension::{
        grpc_vsock::VsockGrpcExt, http_vsock::VsockHttpExt, metrics::spawn_metrics_task,
        snapshot_editor::SnapshotEditorExt,
    },
    runtime::tokio::TokioRuntime,
    vm::{
        api::VmApi,
        models::{CreateSnapshot, MetricsSystem, SnapshotType},
    },
    vmm::process::HyperResponseExt,
};
use futures_util::StreamExt;
use grpc_codegen::{guest_agent_service_client::GuestAgentServiceClient, Ping, Pong};
use http_body_util::Full;
use serde::{Deserialize, Serialize};
use test_framework::{
    get_real_firecracker_installation, get_tmp_fifo_path, get_tmp_path, shutdown_test_vm, TestOptions, TestVm,
    VmBuilder,
};
use tokio::fs::metadata;

mod grpc_codegen;
mod test_framework;

#[test]
fn snapshot_editor_can_rebase_memory() {
    VmBuilder::new().run(|mut vm| async move {
        vm.api_pause().await.unwrap();
        let base_snapshot = vm
            .api_create_snapshot(CreateSnapshot::new(get_tmp_path(), get_tmp_path()))
            .await
            .unwrap();
        vm.api_resume().await.unwrap();
        vm.api_pause().await.unwrap();
        let diff_snapshot = vm
            .api_create_snapshot(CreateSnapshot::new(get_tmp_path(), get_tmp_path()).snapshot_type(SnapshotType::Diff))
            .await
            .unwrap();
        vm.api_resume().await.unwrap();

        get_real_firecracker_installation()
            .snapshot_editor::<TokioRuntime>()
            .rebase_memory(base_snapshot.mem_file_path, diff_snapshot.mem_file_path)
            .await
            .unwrap();
        shutdown_test_vm(&mut vm).await;
    })
}

#[test]
fn snapshot_editor_can_get_snapshot_version() {
    VmBuilder::new().run(|mut vm| async move {
        vm.api_pause().await.unwrap();
        let snapshot = vm
            .api_create_snapshot(CreateSnapshot::new(get_tmp_path(), get_tmp_path()))
            .await
            .unwrap();
        vm.api_resume().await.unwrap();

        let version = get_real_firecracker_installation()
            .snapshot_editor::<TokioRuntime>()
            .get_snapshot_version(snapshot.snapshot_path)
            .await
            .unwrap();
        assert_eq!(version.trim(), TestOptions::get().await.toolchain.snapshot_version);
        shutdown_test_vm(&mut vm).await;
    });
}

#[test]
fn snapshot_editor_can_get_snapshot_vcpu_states() {
    VmBuilder::new().run(|mut vm| async move {
        vm.api_pause().await.unwrap();
        let snapshot = vm
            .api_create_snapshot(CreateSnapshot::new(get_tmp_path(), get_tmp_path()))
            .await
            .unwrap();
        vm.api_resume().await.unwrap();

        let data = get_real_firecracker_installation()
            .snapshot_editor::<TokioRuntime>()
            .get_snapshot_vcpu_states(snapshot.snapshot_path)
            .await
            .unwrap();
        let first_line = data.lines().next().unwrap();
        assert!(first_line.contains("vcpu 0:"));

        shutdown_test_vm(&mut vm).await;
    });
}

#[test]
fn snapshot_editor_can_get_snapshot_vm_state() {
    VmBuilder::new().run(|mut vm| async move {
        vm.api_pause().await.unwrap();
        let snapshot = vm
            .api_create_snapshot(CreateSnapshot::new(get_tmp_path(), get_tmp_path()))
            .await
            .unwrap();
        vm.api_resume().await.unwrap();

        let data = get_real_firecracker_installation()
            .snapshot_editor::<TokioRuntime>()
            .get_snapshot_vm_state(snapshot.snapshot_path)
            .await
            .unwrap();
        assert!(data.contains("kvm"));
        shutdown_test_vm(&mut vm).await;
    });
}

#[test]
fn metrics_task_can_receive_data_from_plaintext() {
    VmBuilder::new()
        .metrics_system(MetricsSystem::new(get_tmp_path()))
        .run(|vm| test_metrics_recv(false, vm));
}

#[test]
fn metrics_task_can_receive_data_from_fifo() {
    VmBuilder::new()
        .metrics_system(MetricsSystem::new(get_tmp_fifo_path()))
        .run(|vm| test_metrics_recv(true, vm));
}

async fn test_metrics_recv(is_fifo: bool, mut vm: TestVm) {
    let metrics_path = vm.get_accessible_paths().metrics_path.clone().unwrap();
    let file_type = metadata(&metrics_path).await.unwrap().file_type();

    if is_fifo {
        assert!(file_type.is_fifo());
    } else {
        assert!(!file_type.is_fifo());
        assert!(file_type.is_file());
    }

    let mut metrics_task =
        spawn_metrics_task::<TokioRuntime>(vm.get_accessible_paths().metrics_path.clone().unwrap(), 100);
    let metrics = metrics_task.receiver.next().await.unwrap();
    assert!(metrics.put_api_requests.actions_count > 0);
    shutdown_test_vm(&mut vm).await;
}

#[test]
fn metrics_task_can_be_cancelled_via_join_handle() {
    VmBuilder::new()
        .metrics_system(MetricsSystem::new(get_tmp_path()))
        .run(|mut vm| async move {
            let mut metrics_task =
                spawn_metrics_task::<TokioRuntime>(vm.get_accessible_paths().metrics_path.clone().unwrap(), 100);
            drop(metrics_task.task);
            tokio::time::timeout(Duration::from_secs(1), async {
                while metrics_task.receiver.next().await.is_some() {}
            })
            .await
            .unwrap();
            shutdown_test_vm(&mut vm).await;
        });
}

#[derive(Serialize)]
struct PingRequest {
    a: u32,
    b: u32,
}

#[derive(Deserialize, Debug, PartialEq, Eq)]
struct PingResponse {
    c: u32,
}

const VSOCK_HTTP_GUEST_PORT: u32 = 8000;
const VSOCK_GRPC_GUEST_PORT: u32 = 9000;

#[test]
fn vsock_can_make_plain_http_connection() {
    VmBuilder::new().vsock_device().run(|mut vm| async move {
        let mut connection = vm.vsock_connect_over_http(VSOCK_HTTP_GUEST_PORT).await.unwrap();
        let response = connection.send_request(make_vsock_req()).await.unwrap();
        assert_vsock_resp(response).await;
        shutdown_test_vm(&mut vm).await;
    });
}

#[test]
fn vsock_can_make_pooled_http_connection() {
    VmBuilder::new().vsock_device().run(|mut vm| async move {
        let connection_pool = vm.vsock_create_http_connection_pool(VSOCK_HTTP_GUEST_PORT).unwrap();
        let response = connection_pool.send_request("/ping", make_vsock_req()).await.unwrap();
        assert_vsock_resp(response).await;
        shutdown_test_vm(&mut vm).await;
    });
}

#[test]
fn vsock_can_perform_unary_grpc_request() {
    VmBuilder::new().vsock_device().run(|mut vm| async move {
        let channel = vm
            .vsock_connect_over_grpc(VSOCK_GRPC_GUEST_PORT, |endpoint| endpoint)
            .await
            .unwrap();
        let mut client = GuestAgentServiceClient::new(channel);
        let response = client.unary(Ping { number: 5 }).await.unwrap();
        assert_eq!(response.into_inner(), Pong { number: 25 });
        shutdown_test_vm(&mut vm).await;
    });
}

#[test]
fn vsock_can_perform_client_streaming_grpc_request() {
    VmBuilder::new().vsock_device().run(|mut vm| async move {
        let channel = vm.vsock_connect_over_grpc(VSOCK_GRPC_GUEST_PORT, |e| e).await.unwrap();
        let mut client = GuestAgentServiceClient::new(channel);
        let stream = futures_util::stream::repeat(Ping { number: 2 }).take(4);
        let response = client.client_streaming(stream).await.unwrap();
        assert_eq!(response.into_inner(), Pong { number: 16 });
        shutdown_test_vm(&mut vm).await;
    });
}

#[test]
fn vsock_can_perform_server_streaming_grpc_request() {
    VmBuilder::new().vsock_device().run(|mut vm| async move {
        let mut client =
            GuestAgentServiceClient::new(vm.vsock_connect_over_grpc(VSOCK_GRPC_GUEST_PORT, |e| e).await.unwrap());
        let mut streaming = client.server_streaming(Ping { number: 5 }).await.unwrap().into_inner();
        let mut count = 0;

        while let Ok(Some(message)) = streaming.message().await {
            assert_eq!(message, Pong { number: 5 });
            count += 1;
        }

        assert_eq!(count, 5);
        shutdown_test_vm(&mut vm).await;
    });
}

#[test]
fn vsock_can_perform_duplex_streaming_grpc_request() {
    VmBuilder::new().vsock_device().run(|mut vm| async move {
        let mut client =
            GuestAgentServiceClient::new(vm.vsock_connect_over_grpc(VSOCK_GRPC_GUEST_PORT, |e| e).await.unwrap());
        let request_stream =
            futures_util::stream::iter(vec![Ping { number: 1 }, Ping { number: 2 }, Ping { number: 3 }]);
        let mut response_stream = client.duplex_streaming(request_stream).await.unwrap().into_inner();
        let mut responses = Vec::new();

        while let Ok(Some(response)) = response_stream.message().await {
            responses.push(response);
        }

        assert_eq!(
            responses,
            vec![Pong { number: 1 }, Pong { number: 2 }, Pong { number: 3 }]
        );
        shutdown_test_vm(&mut vm).await;
    });
}

fn make_vsock_req() -> http::Request<Full<Bytes>> {
    let request_json = serde_json::to_string(&PingRequest { a: 4, b: 5 }).unwrap();
    http::Request::builder()
        .uri("/ping")
        .method("POST")
        .header("Content-Type", "application/json")
        .body(Full::new(Bytes::from(request_json)))
        .unwrap()
}

async fn assert_vsock_resp(mut response: http::Response<hyper::body::Incoming>) {
    let response_json = response.recv_to_string().await.unwrap();
    assert_eq!(
        serde_json::from_str::<PingResponse>(&response_json).unwrap(),
        PingResponse { c: 20 }
    );
}
