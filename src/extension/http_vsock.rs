use std::{future::Future, path::PathBuf};

use bytes::Bytes;
use http::{Request, Response, Uri};
use http_body_util::Full;
use hyper::{body::Incoming, client::conn::http1::SendRequest};
use hyper_client_sockets::{connector::FirecrackerConnector, uri::FirecrackerUri};

use crate::{
    process_spawner::ProcessSpawner,
    runtime::{util::RuntimeHyperExecutor, Runtime},
    vm::Vm,
    vmm::executor::VmmExecutor,
};

/// An error that can be emitted by the HTTP-over-vsock extension.
#[derive(Debug)]
pub enum VsockHttpError {
    VsockNotConfigured,
    CannotConnect(std::io::Error),
    CannotHandshake(hyper::Error),
    VsockResourceUninitialized,
}

impl std::error::Error for VsockHttpError {}

impl std::fmt::Display for VsockHttpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VsockHttpError::VsockNotConfigured => write!(f, "A vsock device was not configured for this VM"),
            VsockHttpError::CannotConnect(err) => write!(f, "Could not connect to the vsock socket: {err}"),
            VsockHttpError::CannotHandshake(err) => {
                write!(f, "Could not perform an HTTP handshake with the vsock socket: {err}")
            }
            VsockHttpError::VsockResourceUninitialized => write!(f, "The vsock resource was uninitialized"),
        }
    }
}

/// An error that can be emitted by the [VsockHttpPool] HTTP connection pool.
#[derive(Debug)]
pub enum VsockHttpPoolError {
    InvalidUri { uri: String, error: http::uri::InvalidUri },
    RequestError(hyper_util::client::legacy::Error),
}

impl std::error::Error for VsockHttpPoolError {}

impl std::fmt::Display for VsockHttpPoolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VsockHttpPoolError::InvalidUri { uri, error } => write!(f, "The vsock URI \"{uri}\" is invalid: {error}"),
            VsockHttpPoolError::RequestError(err) => write!(f, "The connection to the vsock device failed: {err}"),
        }
    }
}

/// A managed HTTP connection pool using vsock. Currently the underlying implementation is backed by hyper-util,
/// but this may change in the future without any changes to the exposed API.
pub struct VsockHttpPool<B: hyper_client_sockets::Backend + Send + Sync + 'static> {
    client: hyper_util::client::legacy::Client<FirecrackerConnector<B>, Full<Bytes>>,
    socket_path: PathBuf,
    guest_port: u32,
}

impl<B: hyper_client_sockets::Backend + Send + Sync + 'static> VsockHttpPool<B> {
    /// Send a HTTP request via this pool. Since this is a pool and not a single connection, only shared access to
    /// the pool is needed.
    pub async fn send_request<U: AsRef<str> + Send>(
        &self,
        uri: U,
        mut request: Request<Full<Bytes>>,
    ) -> Result<Response<Incoming>, VsockHttpPoolError> {
        let uri = uri.as_ref();

        let actual_uri = Uri::firecracker(&self.socket_path, self.guest_port, uri).map_err(|error| {
            VsockHttpPoolError::InvalidUri {
                uri: uri.to_owned(),
                error,
            }
        })?;
        *request.uri_mut() = actual_uri;

        self.client
            .request(request)
            .await
            .map_err(VsockHttpPoolError::RequestError)
    }
}

/// An extension that allows connecting to guest applications that expose a plain-HTTP (REST or any other) server being tunneled over
/// the Firecracker vsock device.
pub trait VsockHttpExt {
    /// The [hyper_client_sockets::Backend] used for connection pooling by this extension.
    type SocketBackend: hyper_client_sockets::Backend + Send + Sync;

    /// Eagerly make a single HTTP connection to the given guest port, without support for connection pooling.
    fn vsock_connect_over_http(
        &self,
        guest_port: u32,
    ) -> impl Future<Output = Result<SendRequest<Full<Bytes>>, VsockHttpError>> + Send;

    /// Make a lazy HTTP connection pool (backed by hyper-util) that pools multiple connections internally to the given guest port.
    fn vsock_create_http_connection_pool(
        &self,
        guest_port: u32,
    ) -> Result<VsockHttpPool<Self::SocketBackend>, VsockHttpError>;
}

impl<E: VmmExecutor, S: ProcessSpawner, R: Runtime> VsockHttpExt for Vm<E, S, R> {
    type SocketBackend = R::SocketBackend;

    async fn vsock_connect_over_http(&self, guest_port: u32) -> Result<SendRequest<Full<Bytes>>, VsockHttpError> {
        let socket_path = self
            .get_configuration()
            .data()
            .vsock_device
            .as_ref()
            .ok_or(VsockHttpError::VsockNotConfigured)?
            .uds
            .get_effective_path()
            .ok_or(VsockHttpError::VsockResourceUninitialized)?;
        let stream = <R::SocketBackend as hyper_client_sockets::Backend>::connect_to_firecracker_socket(
            &socket_path,
            guest_port,
        )
        .await
        .map_err(VsockHttpError::CannotConnect)?;

        let (send_request, connection) = hyper::client::conn::http1::handshake::<_, Full<Bytes>>(stream)
            .await
            .map_err(VsockHttpError::CannotHandshake)?;
        self.vmm_process.resource_system.runtime.spawn_task(connection);

        Ok(send_request)
    }

    fn vsock_create_http_connection_pool(
        &self,
        guest_port: u32,
    ) -> Result<VsockHttpPool<R::SocketBackend>, VsockHttpError> {
        let client = hyper_util::client::legacy::Client::builder(RuntimeHyperExecutor(
            self.vmm_process.resource_system.runtime.clone(),
        ))
        .build(FirecrackerConnector::<R::SocketBackend>::new());
        let socket_path = self
            .get_configuration()
            .data()
            .vsock_device
            .as_ref()
            .ok_or(VsockHttpError::VsockNotConfigured)?
            .uds
            .get_effective_path()
            .ok_or(VsockHttpError::VsockResourceUninitialized)?;

        Ok(VsockHttpPool {
            client,
            socket_path,
            guest_port,
        })
    }
}
