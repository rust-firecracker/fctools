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
                write!(f, "Could not perform an HTTP handshake with the vsock socket: {err}")
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
    /// An I/O error occurred while performing the HTTP request over the [hyper_util] connection pool.
    RequestError(hyper_util::client::legacy::Error),
}

impl std::error::Error for VmVsockHttpClientError {}

impl std::fmt::Display for VmVsockHttpClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VmVsockHttpClientError::InvalidUri { uri, error } => {
                write!(f, "The vsock URI \"{uri}\" is invalid: {error}")
            }
            VmVsockHttpClientError::RequestError(err) => write!(f, "The connection to the vsock device failed: {err}"),
        }
    }
}

/// A managed HTTP client to a vsock application inside a VM, backed by a [hyper_util] connection pool.
pub struct VmVsockHttpClient<B: hyper_client_sockets::Backend + Send + Sync + 'static> {
    client: hyper_util::client::legacy::Client<FirecrackerConnector<B>, Full<Bytes>>,
    socket_path: PathBuf,
    guest_port: u32,
}

impl<B: hyper_client_sockets::Backend + Send + Sync + 'static> VmVsockHttpClient<B> {
    /// Send a HTTP request via this pool. Since this is a pool and not a single connection, only shared access to
    /// the pool is needed.
    pub async fn send_request<U: AsRef<str> + Send>(
        &self,
        uri: U,
        mut request: Request<Full<Bytes>>,
    ) -> Result<Response<Incoming>, VmVsockHttpClientError> {
        let uri = uri.as_ref();

        let actual_uri = Uri::firecracker(&self.socket_path, self.guest_port, uri).map_err(|error| {
            VmVsockHttpClientError::InvalidUri {
                uri: uri.to_owned(),
                error,
            }
        })?;
        *request.uri_mut() = actual_uri;

        self.client
            .request(request)
            .await
            .map_err(VmVsockHttpClientError::RequestError)
    }
}

/// An extension that allows connecting to guest applications that expose a plain-HTTP (REST or any other) server
/// being tunneled over the Firecracker vsock device. Only unencrypted HTTP/1 connections are supported, as, due to
/// the extensive security already provided by Firecracker's VMM when performing vsock connections, TLS encryption
/// is largely redundant.
pub trait VmVsockHttp {
    /// The [hyper_client_sockets::Backend] used for establishing vsock connections by this extension.
    type SocketBackend: hyper_client_sockets::Backend + Send + Sync;

    /// Eagerly establish a single HTTP-over-vsock connection to the given guest port, without creating
    /// a [VmVsockHttpClient] connection pool.
    fn connect_to_http_over_vsock(
        &self,
        guest_port: u32,
    ) -> impl Future<Output = Result<SendRequest<Full<Bytes>>, VmVsockHttpError>> + Send;

    /// Create a [VmVsockHttpClient] connection pool lazily, with it establishing HTTP-over-vsock
    /// connections to the provided guest port.
    fn create_http_over_vsock_client(
        &self,
        guest_port: u32,
    ) -> Result<VmVsockHttpClient<Self::SocketBackend>, VmVsockHttpError>;
}

impl<E: VmmExecutor, S: ProcessSpawner, R: Runtime> VmVsockHttp for Vm<E, S, R> {
    type SocketBackend = R::SocketBackend;

    async fn connect_to_http_over_vsock(&self, guest_port: u32) -> Result<SendRequest<Full<Bytes>>, VmVsockHttpError> {
        let socket_path = self
            .get_configuration()
            .data()
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

        Ok(send_request)
    }

    fn create_http_over_vsock_client(
        &self,
        guest_port: u32,
    ) -> Result<VmVsockHttpClient<R::SocketBackend>, VmVsockHttpError> {
        let client = hyper_util::client::legacy::Client::builder(RuntimeHyperExecutor(
            self.vmm_process.resource_system.runtime.clone(),
        ))
        .build(FirecrackerConnector::<R::SocketBackend>::new());
        let socket_path = self
            .get_configuration()
            .data()
            .vsock_device
            .as_ref()
            .ok_or(VmVsockHttpError::VsockNotConfigured)?
            .uds
            .get_effective_path()
            .ok_or(VmVsockHttpError::VsockResourceUninitialized)?;

        Ok(VmVsockHttpClient {
            client,
            socket_path,
            guest_port,
        })
    }
}
