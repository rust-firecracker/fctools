use std::path::PathBuf;

use async_trait::async_trait;
use bytes::Bytes;
use http::{Request, Response, Uri};
use http_body_util::Full;
use hyper::{body::Incoming, client::conn::http1::SendRequest};
use hyper_client_sockets::{FirecrackerUriExt, HyperFirecrackerConnector, HyperFirecrackerStream};
use hyper_util::rt::TokioExecutor;
use tokio::net::UnixListener;

use crate::{executor::VmmExecutor, shell_spawner::ShellSpawner, vm::Vm};

#[derive(Debug, thiserror::Error)]
pub enum VsockError {
    #[error("A vsock device was not configured for this VM")]
    VsockNotConfigured,
    #[error("Could not connect to the vsock socket: `{0}`")]
    CannotConnect(tokio::io::Error),
    #[error("Could not perform an HTTP handshake with the vsock socket: `{0}`")]
    CannotHandshake(hyper::Error),
    #[error("Could not bind to a host-side Unix socket: `{0}`")]
    CannotBind(tokio::io::Error),
}

#[derive(Debug, thiserror::Error)]
pub enum VsockConnectionPoolError {
    #[error("A vsock URI cannot be constructed: `{0}`")]
    UriCannotBeConstructed(Box<dyn std::error::Error>),
    #[error("An error occurred in the hyper-util connection pool: `{0}`")]
    ConnectionPoolError(hyper_util::client::legacy::Error),
}

pub struct VsockConnectionPool {
    client: hyper_util::client::legacy::Client<HyperFirecrackerConnector, Full<Bytes>>,
    socket_path: PathBuf,
    guest_port: u32,
}

impl VsockConnectionPool {
    pub async fn send_request(
        &self,
        uri: impl AsRef<str> + Send,
        mut request: Request<Full<Bytes>>,
    ) -> Result<Response<Incoming>, VsockConnectionPoolError> {
        let actual_uri = Uri::firecracker(&self.socket_path, self.guest_port, uri)
            .map_err(VsockConnectionPoolError::UriCannotBeConstructed)?;
        *request.uri_mut() = actual_uri;
        self.client
            .request(request)
            .await
            .map_err(VsockConnectionPoolError::ConnectionPoolError)
    }
}

#[async_trait]
pub trait VsockExt {
    async fn vsock_connect(&self, guest_port: u32) -> Result<SendRequest<Full<Bytes>>, VsockError>;

    fn vsock_connect_with_pool(&self, guest_port: u32) -> Result<VsockConnectionPool, VsockError>;

    fn vsock_listen(&mut self, host_port: u32) -> Result<UnixListener, VsockError>;
}

#[async_trait]
impl<E: VmmExecutor, S: ShellSpawner> VsockExt for Vm<E, S> {
    async fn vsock_connect(&self, guest_port: u32) -> Result<SendRequest<Full<Bytes>>, VsockError> {
        let uds_path = self
            .get_accessible_paths()
            .vsock_multiplexer_path
            .as_ref()
            .ok_or(VsockError::VsockNotConfigured)?;
        let stream = HyperFirecrackerStream::connect(uds_path, guest_port)
            .await
            .map_err(VsockError::CannotConnect)?;
        let (send_request, connection) = hyper::client::conn::http1::handshake::<_, Full<Bytes>>(stream)
            .await
            .map_err(VsockError::CannotHandshake)?;
        tokio::spawn(connection);

        Ok(send_request)
    }

    fn vsock_connect_with_pool(&self, guest_port: u32) -> Result<VsockConnectionPool, VsockError> {
        let client = hyper_util::client::legacy::Client::builder(TokioExecutor::new()).build(HyperFirecrackerConnector);
        let socket_path = self
            .get_accessible_paths()
            .vsock_multiplexer_path
            .clone()
            .ok_or(VsockError::VsockNotConfigured)?;
        Ok(VsockConnectionPool {
            client,
            socket_path,
            guest_port,
        })
    }

    fn vsock_listen(&mut self, host_port: u32) -> Result<UnixListener, VsockError> {
        let mut socket_path = dbg!(self
            .get_accessible_paths()
            .vsock_multiplexer_path
            .clone()
            .ok_or(VsockError::VsockNotConfigured)?);
        socket_path.as_mut_os_string().push(format!("_{host_port}"));
        self.register_vsock_listener_path(socket_path.clone());
        UnixListener::bind(socket_path).map_err(VsockError::CannotBind)
    }
}
