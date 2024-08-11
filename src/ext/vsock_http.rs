use std::path::PathBuf;

use async_trait::async_trait;
use bytes::Bytes;
use http::{Request, Response, Uri};
use http_body_util::Full;
use hyper::{body::Incoming, client::conn::http1::SendRequest};
use hyper_client_sockets::{FirecrackerUriExt, HyperFirecrackerConnector, HyperFirecrackerStream};
use hyper_util::rt::TokioExecutor;
use tokio::net::UnixListener;

use crate::{executor::VmmExecutor, shell::ShellSpawner, vm::Vm};

pub enum VsockError {
    VsockNotConfigured,
    CannotConnect(tokio::io::Error),
    CannotHandshake(hyper::Error),
    CannotBind(tokio::io::Error),
}

pub enum VsockConnectionPoolError {
    WrongUri(Box<dyn std::error::Error>),
    RequestError(hyper_util::client::legacy::Error),
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
        let actual_uri =
            Uri::firecracker(&self.socket_path, self.guest_port, uri).map_err(VsockConnectionPoolError::WrongUri)?;
        *request.uri_mut() = actual_uri;
        self.client
            .request(request)
            .await
            .map_err(VsockConnectionPoolError::RequestError)
    }
}

#[async_trait]
pub trait VsockExt {
    async fn vsock_connect(&self, guest_port: u32) -> Result<SendRequest<Full<Bytes>>, VsockError>;

    fn vsock_connect_with_pool(&self, guest_port: u32) -> Result<VsockConnectionPool, VsockError>;

    fn vsock_listen(&self, host_port: u32) -> Result<UnixListener, VsockError>;
}

#[async_trait]
impl<E: VmmExecutor + Sync, S: ShellSpawner> VsockExt for Vm<E, S> {
    async fn vsock_connect(&self, guest_port: u32) -> Result<SendRequest<Full<Bytes>>, VsockError> {
        let uds_path = self
            .get_standard_paths()
            .get_vsock_uds_path()
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
            .get_standard_paths()
            .get_vsock_uds_path()
            .ok_or(VsockError::VsockNotConfigured)?
            .clone();
        Ok(VsockConnectionPool {
            client,
            socket_path,
            guest_port,
        })
    }

    fn vsock_listen(&self, host_port: u32) -> Result<UnixListener, VsockError> {
        let mut socket_path = self
            .get_standard_paths()
            .get_vsock_uds_path()
            .ok_or(VsockError::VsockNotConfigured)?
            .clone();
        socket_path.push(format!("_{host_port}"));
        UnixListener::bind(socket_path).map_err(VsockError::CannotBind)
    }
}
