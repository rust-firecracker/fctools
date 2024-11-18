use std::{future::Future, path::PathBuf, pin::Pin};

use bytes::Bytes;
use http::{Request, Response, Uri};
use http_body_util::Full;
use hyper::{body::Incoming, client::conn::http1::SendRequest};
use hyper_client_sockets::{FirecrackerUriExt, HyperFirecrackerConnector, HyperFirecrackerStream};

use crate::{
    process_spawner::ProcessSpawner,
    runtime::{Runtime, RuntimeExecutor},
    vm::Vm,
    vmm::executor::VmmExecutor,
};

/// An error that can be emitted by the HTTP-over-vsock extension.
#[derive(Debug, thiserror::Error)]
pub enum VsockHttpError {
    #[error("A vsock device was not configured for this VM")]
    VsockNotConfigured,
    #[error("Could not connect to the vsock socket: {0}")]
    CannotConnect(std::io::Error),
    #[error("Could not perform an HTTP handshake with the vsock socket: {0}")]
    CannotHandshake(hyper::Error),
}

/// An error that can be emitted by the [VsockHttpPool] abstraction.
#[derive(Debug, thiserror::Error)]
pub enum VsockHttpPoolError {
    #[error("A vsock URI cannot be constructed: {0}")]
    UriCannotBeConstructed(Box<dyn std::error::Error>),
    #[error("An error occurred in the hyper-util connection pool: {0}")]
    ConnectionPoolError(hyper_util::client::legacy::Error),
}

/// A managed HTTP connection pool using vsock. Currently the underlying implementation is backed by hyper-util,
/// but this may change in the future without any changes to the exposed API.
pub struct VsockHttpPool {
    client: hyper_util::client::legacy::Client<HyperFirecrackerConnector, Full<Bytes>>,
    socket_path: PathBuf,
    guest_port: u32,
}

impl VsockHttpPool {
    /// Send a HTTP request via this pool. Since this is a pool and not a single connection, only shared access to
    /// the pool is needed.
    pub async fn send_request(
        &self,
        uri: impl AsRef<str> + Send,
        mut request: Request<Full<Bytes>>,
    ) -> Result<Response<Incoming>, VsockHttpPoolError> {
        let actual_uri = Uri::firecracker(&self.socket_path, self.guest_port, uri)
            .map_err(VsockHttpPoolError::UriCannotBeConstructed)?;
        *request.uri_mut() = actual_uri;
        self.client
            .request(request)
            .await
            .map_err(VsockHttpPoolError::ConnectionPoolError)
    }
}

/// An extension that allows connecting to guest applications that expose a plain-HTTP (REST or any other) server being tunneled over
/// the Firecracker vsock device.
pub trait VsockHttpExt {
    /// Eagerly make a single HTTP connection to the given guest port, without support for connection pooling.
    fn vsock_connect_over_http(
        &self,
        guest_port: u32,
    ) -> impl Future<Output = Result<SendRequest<Full<Bytes>>, VsockHttpError>> + Send;

    /// Make a lazy HTTP connection pool (backed by hyper-util) that pools multiple connections internally to the given guest port.
    fn vsock_create_http_connection_pool<X>(
        &self,
        guest_port: u32,
        hyper_executor: X,
    ) -> Result<VsockHttpPool, VsockHttpError>
    where
        X: hyper::rt::Executor<Pin<Box<dyn Future<Output = ()> + Send>>> + Send + Sync + Clone + 'static;
}

impl<E: VmmExecutor, S: ProcessSpawner, R: Runtime> VsockHttpExt for Vm<E, S, R> {
    async fn vsock_connect_over_http(&self, guest_port: u32) -> Result<SendRequest<Full<Bytes>>, VsockHttpError> {
        let uds_path = self
            .get_accessible_paths()
            .vsock_multiplexer_path
            .as_ref()
            .ok_or(VsockHttpError::VsockNotConfigured)?;
        let stream = HyperFirecrackerStream::connect(uds_path, guest_port)
            .await
            .map_err(VsockHttpError::CannotConnect)?;
        let (send_request, connection) = hyper::client::conn::http1::handshake::<_, Full<Bytes>>(stream)
            .await
            .map_err(VsockHttpError::CannotHandshake)?;
        let _ = R::Executor::spawn(connection);

        Ok(send_request)
    }

    fn vsock_create_http_connection_pool<X>(
        &self,
        guest_port: u32,
        hyper_executor: X,
    ) -> Result<VsockHttpPool, VsockHttpError>
    where
        X: hyper::rt::Executor<Pin<Box<dyn Future<Output = ()> + Send>>> + Send + Sync + Clone + 'static,
    {
        let client = hyper_util::client::legacy::Client::builder(hyper_executor).build(HyperFirecrackerConnector);
        let socket_path = self
            .get_accessible_paths()
            .vsock_multiplexer_path
            .clone()
            .ok_or(VsockHttpError::VsockNotConfigured)?;
        Ok(VsockHttpPool {
            client,
            socket_path,
            guest_port,
        })
    }
}
