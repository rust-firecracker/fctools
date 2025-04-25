use std::{future::Future, path::PathBuf, sync::Arc};

use bytes::Bytes;
use futures_util::lock::Mutex;
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
pub enum VmVsockHttpError {
    /// The vsock device is not configured for the VM.
    VsockNotConfigured,
    /// An I/O error occurred while establishing a connection to the vsock application inside the VM.
    ConnectionError(std::io::Error),
    /// An I/O error occurred while performing an HTTP handshake over an established connection to the
    /// vsock application inside the VM.
    HandshakeError(hyper::Error),
    /// The vsock Unix socket resource is uninitialized.
    VsockResourceUninitialized,
}

impl std::error::Error for VmVsockHttpError {}

impl std::fmt::Display for VmVsockHttpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VmVsockHttpError::VsockNotConfigured => write!(f, "A vsock device was not configured for this VM"),
            VmVsockHttpError::ConnectionError(err) => write!(f, "Could not connect to the vsock socket: {err}"),
            VmVsockHttpError::HandshakeError(err) => {
                write!(f, "Could not perform an HTTP handshake over a vsock connection: {err}")
            }
            VmVsockHttpError::VsockResourceUninitialized => write!(f, "The vsock resource was uninitialized"),
        }
    }
}

/// An error that can be emitted by the [VmVsockHttpClient] HTTP client.
#[derive(Debug)]
pub enum VmVsockHttpClientError {
    /// The provided HTTP URI was invalid in accordance with a provided [http::uri::InvalidUri] error.
    InvalidUri { uri: String, error: http::uri::InvalidUri },
    /// An I/O error occurred while making an HTTP request or establishing a connection on the connection
    /// pool. This is internally either a [hyper::Error] or an [hyper_util::client::legacy::Error],
    /// but more variants may be added as the internal implementation changes, thus the boxed opaque type.
    RequestError(Box<dyn std::error::Error + Send + Sync>),
}

impl std::error::Error for VmVsockHttpClientError {}

impl std::fmt::Display for VmVsockHttpClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VmVsockHttpClientError::InvalidUri { uri, error } => {
                write!(f, "The vsock URI \"{uri}\" is invalid: {error}")
            }
            VmVsockHttpClientError::RequestError(err) => write!(
                f,
                "Sending a request to the vsock device or establishing a connection to it failed: {err}"
            ),
        }
    }
}

/// A managed HTTP client to a vsock application inside a VM, backed by either a [hyper_util]
/// HTTP connection pool or a singular [hyper] HTTP connection. This client is cloneable cheaply
/// when using a connection pool, but, when using a single connection, cloning will introduce
/// locking contention, as only one clone will be able to make a request at time, while others
/// wait for the internal [Mutex] holding the connection to unlock. To avoid this issue, using
/// a connection pool-backed [VmVsockHttpClient] is recommended if multiple simultaneous HTTP
/// requests are expected to be sent over the [VmVsockHttpClient].
#[derive(Debug, Clone)]
pub struct VmVsockHttpClient<B: hyper_client_sockets::Backend + Send + Sync + 'static>(VmVsockHttpClientInner<B>);

#[derive(Debug, Clone)]
enum VmVsockHttpClientInner<B: hyper_client_sockets::Backend + Send + Sync + 'static> {
    Connection(Arc<Mutex<SendRequest<Full<Bytes>>>>),
    ConnectionPool {
        client: hyper_util::client::legacy::Client<FirecrackerConnector<B>, Full<Bytes>>,
        socket_path: PathBuf,
        guest_port: u32,
    },
}

impl<B: hyper_client_sockets::Backend + Send + Sync + 'static> VmVsockHttpClient<B> {
    /// Send a HTTP request via this client, only requiring a shared reference of the client.
    /// The provided [Request] must have a an application (non-Firecracker) URI set in order to be valid.
    /// With a connection pool, this is cheap, but a connection will be waiting on an internal [Mutex]
    /// to unlock.
    pub async fn send_request(
        &self,
        mut request: Request<Full<Bytes>>,
    ) -> Result<Response<Incoming>, VmVsockHttpClientError> {
        match self.0 {
            VmVsockHttpClientInner::Connection(ref send_request) => send_request
                .lock()
                .await
                .send_request(request)
                .await
                .map_err(|err| VmVsockHttpClientError::RequestError(Box::new(err))),
            VmVsockHttpClientInner::ConnectionPool {
                ref client,
                ref socket_path,
                guest_port,
            } => {
                let uri = request.uri().to_string();

                let actual_uri = Uri::firecracker(socket_path, guest_port, &uri).map_err(|error| {
                    VmVsockHttpClientError::InvalidUri {
                        uri: uri.to_owned(),
                        error,
                    }
                })?;
                *request.uri_mut() = actual_uri;

                client
                    .request(request)
                    .await
                    .map_err(|err| VmVsockHttpClientError::RequestError(Box::new(err)))
            }
        }
    }
}

/// An extension that allows connecting to guest applications that expose a plain-HTTP (REST or any other) server
/// being tunneled over the Firecracker vsock device. Only unencrypted HTTP/1 connections are supported, as, due to
/// the extensive security already provided by Firecracker's VMM when performing vsock connections, TLS encryption
/// is largely redundant.
pub trait VmVsockHttp {
    /// The [hyper_client_sockets::Backend] used for establishing vsock connections by this extension.
    type SocketBackend: hyper_client_sockets::Backend + Send + Sync + 'static;

    /// Establish a single HTTP-over-vsock connection to the given guest port and create a
    /// [VmVsockHttpClient] backed by it.
    fn connect_to_http_over_vsock(
        &self,
        guest_port: u32,
    ) -> impl Future<Output = Result<VmVsockHttpClient<Self::SocketBackend>, VmVsockHttpError>> + Send;

    /// Create a [VmVsockHttpClient] backed by an HTTP-over-vsock connection pool to the
    /// given guest port.
    fn connect_to_http_over_vsock_via_pool(
        &self,
        guest_port: u32,
    ) -> Result<VmVsockHttpClient<Self::SocketBackend>, VmVsockHttpError>;
}

impl<E: VmmExecutor, S: ProcessSpawner, R: Runtime> VmVsockHttp for Vm<E, S, R> {
    type SocketBackend = R::SocketBackend;

    async fn connect_to_http_over_vsock(
        &self,
        guest_port: u32,
    ) -> Result<VmVsockHttpClient<Self::SocketBackend>, VmVsockHttpError> {
        let socket_path = self
            .get_configuration()
            .get_data()
            .vsock_device
            .as_ref()
            .ok_or(VmVsockHttpError::VsockNotConfigured)?
            .uds
            .get_effective_path()
            .ok_or(VmVsockHttpError::VsockResourceUninitialized)?;
        let stream = <R::SocketBackend as hyper_client_sockets::Backend>::connect_to_firecracker_socket(
            &socket_path,
            guest_port,
        )
        .await
        .map_err(VmVsockHttpError::ConnectionError)?;

        let (send_request, connection) = hyper::client::conn::http1::handshake::<_, Full<Bytes>>(stream)
            .await
            .map_err(VmVsockHttpError::HandshakeError)?;
        self.vmm_process.resource_system.runtime.spawn_task(connection);

        Ok(VmVsockHttpClient(VmVsockHttpClientInner::Connection(Arc::new(
            Mutex::new(send_request),
        ))))
    }

    fn connect_to_http_over_vsock_via_pool(
        &self,
        guest_port: u32,
    ) -> Result<VmVsockHttpClient<R::SocketBackend>, VmVsockHttpError> {
        let client = hyper_util::client::legacy::Client::builder(RuntimeHyperExecutor(
            self.vmm_process.resource_system.runtime.clone(),
        ))
        .build(FirecrackerConnector::<R::SocketBackend>::new());
        let socket_path = self
            .get_configuration()
            .get_data()
            .vsock_device
            .as_ref()
            .ok_or(VmVsockHttpError::VsockNotConfigured)?
            .uds
            .get_effective_path()
            .ok_or(VmVsockHttpError::VsockResourceUninitialized)?
            .to_owned();

        Ok(VmVsockHttpClient(VmVsockHttpClientInner::ConnectionPool {
            client,
            socket_path,
            guest_port,
        }))
    }
}
