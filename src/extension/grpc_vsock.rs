use std::{future::Future, marker::PhantomData, path::PathBuf, pin::Pin, sync::Arc, task::Poll};

use http::Uri;
use tonic::transport::{Channel, Endpoint};

use crate::{
    process_spawner::ProcessSpawner,
    runtime::{util::RuntimeHyperExecutor, Runtime},
    vm::Vm,
    vmm::executor::VmmExecutor,
};

/// An error emitted by the gRPC-over-vsock extension.
#[derive(Debug)]
pub enum VmVsockGrpcError {
    /// The virtio-vsock device is not configured for the VM.
    VsockNotConfigured,
    /// The provided guest port was rejected as a gRPC address by [tonic].
    ProvidedAddressRejected(tonic::transport::Error),
    /// An I/O error occurred while establishing a gRPC connection through [tonic].
    ConnectionError(tonic::transport::Error),
    /// The vsock Unix socket resource was uninitialized.
    VsockResourceUninitialized,
}

impl std::error::Error for VmVsockGrpcError {}

impl std::fmt::Display for VmVsockGrpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VmVsockGrpcError::VsockNotConfigured => write!(f, "A vsock device was not configured for this VM"),
            VmVsockGrpcError::ProvidedAddressRejected(err) => {
                write!(f, "The provided address was rejected as an Endpoint: {err}")
            }
            VmVsockGrpcError::ConnectionError(err) => write!(f, "The gRPC connection failed: {err}"),
            VmVsockGrpcError::VsockResourceUninitialized => write!(f, "The vsock resource was uninitialized"),
        }
    }
}

/// An extension that allows connecting to guest applications that expose a gRPC server being tunneled over
/// the Firecracker vsock device. The established tonic [Channel]-s can be used with codegen or any other type
/// of tonic client. Only unencrypted connections are supported, as, due to the extensive security already
/// provided by Firecracker's VMM when performing vsock connections, TLS encryption is largely redundant.
pub trait VmVsockGrpc {
    /// Connect to a guest port over gRPC eagerly, i.e. by establishing the connection right away.
    /// configure_endpoint can be used as a function to customize Endpoint options via its builder.
    fn connect_to_grpc_over_vsock<C: FnOnce(Endpoint) -> Endpoint>(
        &self,
        guest_port: u32,
        configure_endpoint: C,
    ) -> impl Future<Output = Result<Channel, VmVsockGrpcError>> + Send;

    /// Connect to a guest port over gRPC lazily, i.e. not actually establishing the connection until
    /// first usage of the Channel.
    /// configure_endpoint can be used as a function to customize Endpoint options via its builder.
    fn connect_lazily_to_grpc_over_vsock<C: FnOnce(Endpoint) -> Endpoint>(
        &self,
        guest_port: u32,
        configure_endpoint: C,
    ) -> Result<Channel, VmVsockGrpcError>;
}

impl<E: VmmExecutor, S: ProcessSpawner, R: Runtime> VmVsockGrpc for Vm<E, S, R> {
    fn connect_to_grpc_over_vsock<C: FnOnce(Endpoint) -> Endpoint>(
        &self,
        guest_port: u32,
        configure_endpoint: C,
    ) -> impl Future<Output = Result<Channel, VmVsockGrpcError>> + Send {
        let result = create_endpoint_and_service(self, guest_port, configure_endpoint);
        async move {
            let (endpoint, service) = result?;
            endpoint
                .connect_with_connector(service)
                .await
                .map_err(VmVsockGrpcError::ConnectionError)
        }
    }

    fn connect_lazily_to_grpc_over_vsock<C: FnOnce(Endpoint) -> Endpoint>(
        &self,
        guest_port: u32,
        configure_endpoint: C,
    ) -> Result<Channel, VmVsockGrpcError> {
        let (endpoint, service) = create_endpoint_and_service(self, guest_port, configure_endpoint)?;
        Ok(endpoint.connect_with_connector_lazy(service))
    }
}

#[inline]
fn create_endpoint_and_service<E: VmmExecutor, S: ProcessSpawner, R: Runtime, C: FnOnce(Endpoint) -> Endpoint>(
    vm: &Vm<E, S, R>,
    guest_port: u32,
    configure_endpoint: C,
) -> Result<(Endpoint, FirecrackerTowerService<R::SocketBackend>), VmVsockGrpcError> {
    let uds_path = vm
        .get_configuration()
        .get_data()
        .vsock_device
        .as_ref()
        .ok_or(VmVsockGrpcError::VsockNotConfigured)?
        .uds
        .get_effective_path()
        .ok_or(VmVsockGrpcError::VsockResourceUninitialized)?;

    let endpoint = configure_endpoint(
        Endpoint::try_from(format!("http://[::1]:{guest_port}"))
            .map_err(VmVsockGrpcError::ProvidedAddressRejected)?
            .executor(RuntimeHyperExecutor(vm.vmm_process.resource_system.runtime.clone())),
    );

    let service = FirecrackerTowerService {
        guest_port,
        uds_path: Arc::new(uds_path),
        marker: PhantomData,
    };

    Ok((endpoint, service))
}

struct FirecrackerTowerService<B: hyper_client_sockets::Backend> {
    guest_port: u32,
    uds_path: Arc<PathBuf>,
    marker: PhantomData<B>,
}

impl<B: hyper_client_sockets::Backend> tower_service::Service<Uri> for FirecrackerTowerService<B> {
    type Response = B::FirecrackerIo;

    type Error = std::io::Error;

    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send + 'static>>;

    fn poll_ready(&mut self, _cx: &mut std::task::Context<'_>) -> std::task::Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, _req: Uri) -> Self::Future {
        let uds_path = self.uds_path.clone();
        let guest_port = self.guest_port;

        Box::pin(async move {
            let stream = B::connect_to_firecracker_socket(uds_path.as_ref(), guest_port).await?;
            Ok(stream)
        })
    }
}
