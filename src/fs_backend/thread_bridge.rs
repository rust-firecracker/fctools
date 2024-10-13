use std::future::Future;

use tokio::sync::{broadcast, mpsc};
use uuid::Uuid;

use super::{FsBackend, FsBackendError};

pub struct ThreadBridgeFsBackend {
    request_sender: mpsc::Sender<BridgeRequest>,
    response_receiver: broadcast::Receiver<BridgeResponse>,
    thread_join_handle: std::thread::JoinHandle<()>,
}

pub struct BridgeServer<S: SpawnTask, F: FsBackend> {
    request_receiver: mpsc::Receiver<BridgeRequest>,
    response_sender: broadcast::Sender<BridgeResponse>,
    spawn_task: S,
    fs_backend: F,
}

impl<S: SpawnTask, F: FsBackend> BridgeServer<S, F> {
    pub async fn run(&mut self) {
        while let Some(request) = self.request_receiver.recv().await {
            match request.body {
                BridgeRequestBody::Ping => {
                    let sender = self.response_sender.clone();
                    self.spawn_task
                        .spawn(async move { sender.send(BridgeResponse::new(request, BridgeResponseBody::Pong)) });
                }
                BridgeRequestBody::Terminate => {
                    let _ = self
                        .response_sender
                        .send(BridgeResponse::new(request, BridgeResponseBody::Terminated));
                    break;
                }
            }
        }
    }
}

struct BridgeRequest {
    id: Uuid,
    body: BridgeRequestBody,
}

enum BridgeRequestBody {
    Ping,
    Terminate,
}

#[derive(Clone)]
struct BridgeResponse {
    id: Uuid,
    body: BridgeResponseBody,
}

impl BridgeResponse {
    fn new(request: BridgeRequest, body: BridgeResponseBody) -> Self {
        Self { id: request.id, body }
    }
}

#[derive(Clone)]
enum BridgeResponseBody {
    Pong,
    Terminated,
}

pub trait SpawnTask {
    fn spawn<F>(&self, future: F)
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static;
}

pub struct TokioSpawnTask;

impl SpawnTask for TokioSpawnTask {
    fn spawn<F>(&self, future: F)
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        tokio::task::spawn(future);
    }
}

impl ThreadBridgeFsBackend {
    pub fn new<
        F: FsBackend + Send + 'static,
        S: SpawnTask + Send + 'static,
        R: Send + 'static + FnOnce(BridgeServer<S, F>) -> (),
    >(
        capacity: usize,
        fs_backend: F,
        spawn_task: S,
        runner: R,
    ) -> Self {
        let (request_sender, request_receiver) = mpsc::channel(capacity);
        let (response_sender, response_receiver) = broadcast::channel(capacity);

        let thread_join_handle = std::thread::spawn(move || {
            let bridge_server = BridgeServer {
                request_receiver,
                response_sender,
                spawn_task,
                fs_backend,
            };

            runner(bridge_server);
        });

        Self {
            request_sender,
            response_receiver,
            thread_join_handle,
        }
    }

    pub async fn ping(&self) -> Result<(), FsBackendError> {
        match self.request_core(BridgeRequestBody::Ping).await? {
            BridgeResponseBody::Pong => Ok(()),
            _ => Err(FsBackendError::Owned(std::io::Error::other(
                "An unexpected response was emitted by the bridge server",
            ))),
        }
    }

    pub async fn terminate(&self) -> Result<(), FsBackendError> {
        match self.request_core(BridgeRequestBody::Terminate).await? {
            BridgeResponseBody::Terminated => Ok(()),
            _ => Err(FsBackendError::Owned(std::io::Error::other(
                "An unexpected response was emitted by the bridge server",
            ))),
        }
    }

    pub fn thread_join_handle(&self) -> &std::thread::JoinHandle<()> {
        &self.thread_join_handle
    }

    async fn request_core(&self, request_body: BridgeRequestBody) -> Result<BridgeResponseBody, FsBackendError> {
        let request_id = Uuid::new_v4();
        let mut receiver = self.response_receiver.resubscribe();

        if let Err(_) = self
            .request_sender
            .send(BridgeRequest {
                id: request_id,
                body: request_body,
            })
            .await
        {
            return Err(FsBackendError::Owned(std::io::Error::new(
                std::io::ErrorKind::ConnectionRefused,
                "Request send to bridge thread failed",
            )));
        }

        loop {
            match receiver.recv().await {
                Ok(response) if response.id == request_id => {
                    return Ok(response.body);
                }
                Err(_) => {
                    return Err(FsBackendError::Owned(std::io::Error::new(
                        std::io::ErrorKind::ConnectionRefused,
                        "Response recv from the bridge thread failed",
                    )))
                }
                _ => continue,
            }
        }
    }
}
