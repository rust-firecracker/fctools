use std::{future::Future, marker::PhantomData, path::PathBuf, pin::Pin, sync::Arc, task::Poll};

use http::Uri;
use tonic::transport::{Channel, Endpoint};

use crate::{process_spawner::ProcessSpawner, runtime::Runtime, vm::Vm, vmm::executor::VmmExecutor};

/// An error emitted by the gRPC-over-vsock extension.
#[derive(Debug)]
pub enum VsockGrpcError {
    VsockNotConfigured,
    ProvidedAddressRejected(tonic::transport::Error),
    ConnectionFailed(tonic::transport::Error),
    VsockResourceUninitialized,
}

impl std::error::Error for VsockGrpcError {}

impl std::fmt::Display for VsockGrpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VsockGrpcError::VsockNotConfigured => write!(f, "A vsock device was not configured for this VM"),
            VsockGrpcError::ProvidedAddressRejected(err) => {
                write!(f, "The provided address was rejected as an Endpoint: {err}")
            }
            VsockGrpcError::ConnectionFailed(err) => write!(f, "The gRPC connection failed: {err}"),
            VsockGrpcError::VsockResourceUninitialized => write!(f, "The vsock resource was uninitialized"),
        }
    }
}

/// An extension that allows connecting to guest applications that expose a gRPC server being tunneled over
/// the Firecracker vsock device. The established tonic [Channel]-s can be used with codegen or any other type
/// of tonic client.
pub trait VsockGrpcExt {
    /// Connect to a guest port over gRPC eagerly, i.e. by establishing the connection right away.
    /// configure_endpoint can be used as a function to customize Endpoint options via its builder.
    fn vsock_connect_over_grpc(
        &self,
        guest_port: u32,
        configure_endpoint: impl FnOnce(Endpoint) -> Endpoint,
    ) -> impl Future<Output = Result<Channel, VsockGrpcError>> + Send;

    /// Connect to a guest port over gRPC lazily, i.e. not actually establishing the connection until
    /// first usage of the Channel.
    /// configure_endpoint can be used as a function to customize Endpoint options via its builder.
    fn vsock_lazily_connect_over_grpc(
        &self,
        guest_port: u32,
        configure_endpoint: impl FnOnce(Endpoint) -> Endpoint,
    ) -> Result<Channel, VsockGrpcError>;
}

impl<E: VmmExecutor, S: ProcessSpawner, R: Runtime> VsockGrpcExt for Vm<E, S, R> {
    fn vsock_connect_over_grpc(
        &self,
        guest_port: u32,
        configure_endpoint: impl FnOnce(Endpoint) -> Endpoint,
    ) -> impl Future<Output = Result<Channel, VsockGrpcError>> + Send {
        let result = create_endpoint_and_service(self, guest_port, configure_endpoint);
        async move {
            let (endpoint, service) = result?;
            endpoint
                .connect_with_connector(service)
                .await
                .map_err(VsockGrpcError::ConnectionFailed)
        }
    }

    fn vsock_lazily_connect_over_grpc(
        &self,
        guest_port: u32,
        configure_endpoint: impl FnOnce(Endpoint) -> Endpoint,
    ) -> Result<Channel, VsockGrpcError> {
        let (endpoint, service) = create_endpoint_and_service(self, guest_port, configure_endpoint)?;
        Ok(endpoint.connect_with_connector_lazy(service))
    }
}

#[inline]
fn create_endpoint_and_service<E: VmmExecutor, S: ProcessSpawner, R: Runtime>(
    vm: &Vm<E, S, R>,
    guest_port: u32,
    configure_endpoint: impl FnOnce(Endpoint) -> Endpoint,
) -> Result<(Endpoint, FirecrackerTowerService<R::SocketBackend>), VsockGrpcError> {
    let uds_path = vm
        .configuration()
        .data()
        .vsock_device
        .as_ref()
        .ok_or(VsockGrpcError::VsockNotConfigured)?
        .uds
        .get_effective_path()
        .ok_or(VsockGrpcError::VsockResourceUninitialized)?;

    let mut endpoint =
        Endpoint::try_from(format!("http://[::1]:{guest_port}")).map_err(VsockGrpcError::ProvidedAddressRejected)?;
    endpoint = configure_endpoint(endpoint);

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
