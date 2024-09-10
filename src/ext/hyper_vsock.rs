use std::path::PathBuf;

use async_trait::async_trait;
use bytes::Bytes;
use http::{Request, Response, Uri};
use http_body_util::Full;
use hyper::{body::Incoming, client::conn::http1::SendRequest};
use hyper_client_sockets::{FirecrackerUriExt, HyperFirecrackerConnector, HyperFirecrackerStream};
use hyper_util::rt::TokioExecutor;

use crate::{executor::VmmExecutor, shell_spawner::ShellSpawner, vm::Vm};

#[derive(Debug, thiserror::Error)]
pub enum VsockHttpError {
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
pub enum VsockHttpPoolError {
    #[error("A vsock URI cannot be constructed: `{0}`")]
    UriCannotBeConstructed(Box<dyn std::error::Error>),
    #[error("An error occurred in the hyper-util connection pool: `{0}`")]
    ConnectionPoolError(hyper_util::client::legacy::Error),
}

pub struct VsockHttpPool {
    client: hyper_util::client::legacy::Client<HyperFirecrackerConnector, Full<Bytes>>,
    socket_path: PathBuf,
    guest_port: u32,
}

impl VsockHttpPool {
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

#[async_trait]
pub trait VsockHttpExt {
    async fn vsock_connect_over_http(&self, guest_port: u32) -> Result<SendRequest<Full<Bytes>>, VsockHttpError>;

    fn vsock_create_http_connection_pool(&self, guest_port: u32) -> Result<VsockHttpPool, VsockHttpError>;
}

#[async_trait]
impl<E: VmmExecutor, S: ShellSpawner> VsockHttpExt for Vm<E, S> {
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
        tokio::spawn(connection);

        Ok(send_request)
    }

    fn vsock_create_http_connection_pool(&self, guest_port: u32) -> Result<VsockHttpPool, VsockHttpError> {
        let client = hyper_util::client::legacy::Client::builder(TokioExecutor::new()).build(HyperFirecrackerConnector);
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
