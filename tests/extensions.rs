use std::{os::unix::fs::FileTypeExt, time::Duration};

use bytes::Bytes;
use codegen::{GuestAgentServiceClient, Ping, Pong};
use fctools::{
    extension::{
        grpc_vsock::VmVsockGrpc, http_vsock::VmVsockHttp, metrics::spawn_metrics_task,
        snapshot_editor::SnapshotEditorExt,
    },
    runtime::{tokio::TokioRuntime, RuntimeTask},
    vm::{api::VmApi, models::SnapshotType},
    vmm::{process::HyperResponseExt, resource::CreatedResourceType},
};
use futures_util::StreamExt;
use http_body_util::Full;
use serde::{Deserialize, Serialize};
use test_framework::{
    get_create_snapshot, get_real_firecracker_installation, shutdown_test_vm, TestOptions, TestVm, VmBuilder,
};
use tokio::fs::metadata;

mod codegen {
    #[derive(Clone, Copy, PartialEq, prost::Message)]
    pub struct Ping {
        #[prost(uint32, tag = "1")]
        pub number: u32,
    }
    #[derive(Clone, Copy, PartialEq, prost::Message)]
    pub struct Pong {
        #[prost(uint32, tag = "1")]
        pub number: u32,
    }

    use tonic::codegen::*;

    #[derive(Debug, Clone)]
    pub struct GuestAgentServiceClient<T> {
        inner: tonic::client::Grpc<T>,
    }

    impl<T> GuestAgentServiceClient<T>
    where
        T: tonic::client::GrpcService<tonic::body::Body>,
        T::Error: Into<StdError>,
        T::ResponseBody: Body<Data = Bytes> + std::marker::Send + 'static,
        <T::ResponseBody as Body>::Error: Into<StdError> + std::marker::Send,
    {
        pub fn new(inner: T) -> Self {
            let inner = tonic::client::Grpc::new(inner);
            Self { inner }
        }

        pub async fn unary(
            &mut self,
            request: impl tonic::IntoRequest<super::Ping>,
        ) -> std::result::Result<tonic::Response<super::Pong>, tonic::Status> {
            self.inner
                .ready()
                .await
                .map_err(|e| tonic::Status::unknown(format!("Service was not ready: {}", e.into())))?;
            let codec = tonic::codec::ProstCodec::default();
            let path = http::uri::PathAndQuery::from_static("/guest_agent.GuestAgentService/Unary");
            let mut req = request.into_request();
            req.extensions_mut()
                .insert(GrpcMethod::new("guest_agent.GuestAgentService", "Unary"));
            self.inner.unary(req, path, codec).await
        }

        pub async fn client_streaming(
            &mut self,
            request: impl tonic::IntoStreamingRequest<Message = super::Ping>,
        ) -> std::result::Result<tonic::Response<super::Pong>, tonic::Status> {
            self.inner
                .ready()
                .await
                .map_err(|e| tonic::Status::unknown(format!("Service was not ready: {}", e.into())))?;
            let codec = tonic::codec::ProstCodec::default();
            let path = http::uri::PathAndQuery::from_static("/guest_agent.GuestAgentService/ClientStreaming");
            let mut req = request.into_streaming_request();
            req.extensions_mut()
                .insert(GrpcMethod::new("guest_agent.GuestAgentService", "ClientStreaming"));
            self.inner.client_streaming(req, path, codec).await
        }

        pub async fn server_streaming(
            &mut self,
            request: impl tonic::IntoRequest<super::Ping>,
        ) -> std::result::Result<tonic::Response<tonic::codec::Streaming<super::Pong>>, tonic::Status> {
            self.inner
                .ready()
                .await
                .map_err(|e| tonic::Status::unknown(format!("Service was not ready: {}", e.into())))?;
            let codec = tonic::codec::ProstCodec::default();
            let path = http::uri::PathAndQuery::from_static("/guest_agent.GuestAgentService/ServerStreaming");
            let mut req = request.into_request();
            req.extensions_mut()
                .insert(GrpcMethod::new("guest_agent.GuestAgentService", "ServerStreaming"));
            self.inner.server_streaming(req, path, codec).await
        }

        pub async fn duplex_streaming(
            &mut self,
            request: impl tonic::IntoStreamingRequest<Message = super::Ping>,
        ) -> std::result::Result<tonic::Response<tonic::codec::Streaming<super::Pong>>, tonic::Status> {
            self.inner
                .ready()
                .await
                .map_err(|e| tonic::Status::unknown(format!("Service was not ready: {}", e.into())))?;
            let codec = tonic::codec::ProstCodec::default();
            let path = http::uri::PathAndQuery::from_static("/guest_agent.GuestAgentService/DuplexStreaming");
            let mut req = request.into_streaming_request();
            req.extensions_mut()
                .insert(GrpcMethod::new("guest_agent.GuestAgentService", "DuplexStreaming"));
            self.inner.streaming(req, path, codec).await
        }
    }
}

mod test_framework;

#[test]
fn snapshot_editor_can_rebase_memory() {
    VmBuilder::new().run(|mut vm| async move {
        vm.pause().await.unwrap();
        let create_snapshot = get_create_snapshot(vm.get_resource_system_mut());
        let base_snapshot = vm.create_snapshot(create_snapshot).await.unwrap();
        vm.resume().await.unwrap();
        vm.pause().await.unwrap();

        let mut diff_create_snapshot = get_create_snapshot(vm.get_resource_system_mut());
        diff_create_snapshot.snapshot_type = Some(SnapshotType::Diff);
        let diff_snapshot = vm.create_snapshot(diff_create_snapshot).await.unwrap();

        vm.resume().await.unwrap();

        get_real_firecracker_installation()
            .snapshot_editor(TokioRuntime)
            .rebase_memory(base_snapshot.mem_file_path, diff_snapshot.mem_file_path)
            .await
            .unwrap();

        shutdown_test_vm(&mut vm).await;
    })
}

#[test]
fn snapshot_editor_can_get_snapshot_version() {
    VmBuilder::new().run(|mut vm| async move {
        vm.pause().await.unwrap();
        let create_snapshot = get_create_snapshot(vm.get_resource_system_mut());
        let snapshot = vm.create_snapshot(create_snapshot).await.unwrap();
        vm.resume().await.unwrap();

        let version = get_real_firecracker_installation()
            .snapshot_editor(TokioRuntime)
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
        vm.pause().await.unwrap();
        let create_snapshot = get_create_snapshot(vm.get_resource_system_mut());
        let snapshot = vm.create_snapshot(create_snapshot).await.unwrap();
        vm.resume().await.unwrap();

        let data = get_real_firecracker_installation()
            .snapshot_editor(TokioRuntime)
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
        vm.pause().await.unwrap();
        let create_snapshot = get_create_snapshot(vm.get_resource_system_mut());
        let snapshot = vm.create_snapshot(create_snapshot).await.unwrap();
        vm.resume().await.unwrap();

        let data = get_real_firecracker_installation()
            .snapshot_editor(TokioRuntime)
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
        .metrics_system(CreatedResourceType::File)
        .run(|vm| test_metrics_recv(false, vm));
}

#[test]
fn metrics_task_can_receive_data_from_fifo() {
    VmBuilder::new()
        .metrics_system(CreatedResourceType::Fifo)
        .run(|vm| test_metrics_recv(true, vm));
}

async fn test_metrics_recv(is_fifo: bool, mut vm: TestVm) {
    let metrics_path = vm
        .get_configuration()
        .get_data()
        .metrics_system
        .as_ref()
        .unwrap()
        .metrics
        .get_effective_path()
        .unwrap()
        .to_owned();

    let file_type = metadata(&metrics_path).await.unwrap().file_type();

    if is_fifo {
        assert!(file_type.is_fifo());
    } else {
        assert!(!file_type.is_fifo());
        assert!(file_type.is_file());
    }

    let mut metrics_task = spawn_metrics_task(metrics_path, 100, TokioRuntime);
    let metrics = metrics_task.receiver.next().await.unwrap();
    assert!(metrics.put_api_requests.actions_count > 0);
    shutdown_test_vm(&mut vm).await;
}

#[test]
fn metrics_task_can_be_cancelled_via_join_handle() {
    VmBuilder::new()
        .metrics_system(CreatedResourceType::Fifo)
        .run(|mut vm| async move {
            let mut metrics_task = spawn_metrics_task(
                vm.get_configuration()
                    .get_data()
                    .metrics_system
                    .as_ref()
                    .unwrap()
                    .metrics
                    .get_effective_path()
                    .unwrap()
                    .to_owned(),
                100,
                TokioRuntime,
            );
            metrics_task.task.cancel().await;
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
fn vsock_can_use_http_client_backed_by_connection() {
    VmBuilder::new().vsock_device().run(|mut vm| async move {
        let client = vm.connect_to_http_over_vsock(VSOCK_HTTP_GUEST_PORT).await.unwrap();
        let response = client.send_request(make_vsock_req()).await.unwrap();
        assert_vsock_resp(response).await;
        shutdown_test_vm(&mut vm).await;
    });
}

#[test]
fn vsock_can_use_http_client_backed_by_connection_pool() {
    VmBuilder::new().vsock_device().run(|mut vm| async move {
        let client = vm.connect_to_http_over_vsock_via_pool(VSOCK_HTTP_GUEST_PORT).unwrap();
        let response = client.send_request(make_vsock_req()).await.unwrap();
        assert_vsock_resp(response).await;
        shutdown_test_vm(&mut vm).await;
    });
}

#[test]
fn vsock_can_perform_unary_grpc_request() {
    VmBuilder::new().vsock_device().run(|mut vm| async move {
        let channel = vm
            .connect_to_grpc_over_vsock(VSOCK_GRPC_GUEST_PORT, |endpoint| endpoint)
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
        let channel = vm
            .connect_to_grpc_over_vsock(VSOCK_GRPC_GUEST_PORT, |e| e)
            .await
            .unwrap();
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
        let mut client = GuestAgentServiceClient::new(
            vm.connect_to_grpc_over_vsock(VSOCK_GRPC_GUEST_PORT, |e| e)
                .await
                .unwrap(),
        );
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
        let mut client = GuestAgentServiceClient::new(
            vm.connect_to_grpc_over_vsock(VSOCK_GRPC_GUEST_PORT, |e| e)
                .await
                .unwrap(),
        );
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

#[test]
fn vsock_can_connect_to_grpc_lazily() {
    VmBuilder::new().vsock_device().run(|mut vm| async move {
        let channel = vm
            .connect_lazily_to_grpc_over_vsock(VSOCK_GRPC_GUEST_PORT, |e| e)
            .unwrap();
        let mut client = GuestAgentServiceClient::new(channel);
        let response = client.unary(Ping { number: 5 }).await.unwrap();
        assert_eq!(response.into_inner(), Pong { number: 25 });
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
    let response_json = response.read_body_to_string().await.unwrap();

    assert_eq!(
        serde_json::from_str::<PingResponse>(&response_json).unwrap(),
        PingResponse { c: 20 }
    );
}
