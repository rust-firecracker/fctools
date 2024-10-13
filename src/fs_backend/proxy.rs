use std::{
    future::Future,
    path::{Path, PathBuf},
    sync::Arc,
};

use tokio::sync::{broadcast, mpsc};
use uuid::Uuid;

use super::{FsBackend, FsBackendError};

pub struct ProxyFsBackend {
    request_sender: mpsc::Sender<ProxyRequest>,
    response_receiver: broadcast::Receiver<ProxyResponse>,
    thread_join_handle: std::thread::JoinHandle<()>,
}

pub struct ProxyServer<S: SpawnTask + Send, F: FsBackend> {
    request_receiver: mpsc::Receiver<ProxyRequest>,
    response_sender: broadcast::Sender<ProxyResponse>,
    spawn_task: S,
    fs_backend: Arc<F>,
}

impl<S: SpawnTask + Send, F: FsBackend> ProxyServer<S, F> {
    pub async fn run(&mut self) {
        while let Some(request) = self.request_receiver.recv().await {
            match request.body {
                ProxyRequestBody::Ping => {
                    let sender = self.response_sender.clone();
                    self.spawn_task
                        .spawn(async move { sender.send(ProxyResponse::new(request.id, ProxyResponseBody::Pong)) });
                }
                ProxyRequestBody::Stop => {
                    let _ = self
                        .response_sender
                        .send(ProxyResponse::new(request.id, ProxyResponseBody::Stopped));
                    break;
                }
                ProxyRequestBody::CheckExists(path) => {
                    let sender = self.response_sender.clone();
                    let fs_backend = self.fs_backend.clone();
                    self.spawn_task.spawn(async move {
                        sender.send(ProxyResponse::new(
                            request.id,
                            ProxyResponseBody::BoolAction(
                                fs_backend.check_exists(&path).await.map_err(|e| e.into_cloneable()),
                            ),
                        ))
                    });
                }
                ProxyRequestBody::RemoveFile(path) => {
                    self.respond(request.id, |f| async move { f.remove_file(&path).await });
                }
                ProxyRequestBody::CreateDirAll(path) => {
                    self.respond(request.id, |f| async move { f.create_dir_all(&path).await });
                }
                ProxyRequestBody::CreateFile(path) => {
                    self.respond(request.id, |f| async move { f.create_file(&path).await });
                }
                ProxyRequestBody::Copy {
                    source_path,
                    destination_path,
                } => {
                    self.respond(
                        request.id,
                        |f| async move { f.copy(&source_path, &destination_path).await },
                    );
                }
                ProxyRequestBody::WriteFile { path, content } => {
                    self.respond(request.id, |f| async move { f.write_file(&path, content).await });
                }
                ProxyRequestBody::RenameFile {
                    source_path,
                    destination_path,
                } => {
                    self.respond(request.id, |f| async move {
                        f.rename_file(&source_path, &destination_path).await
                    });
                }
                ProxyRequestBody::HardLink {
                    source_path,
                    destination_path,
                } => {
                    self.respond(request.id, |f| async move {
                        f.hard_link(&source_path, &destination_path).await
                    });
                }
                ProxyRequestBody::RemoveDirAll(path) => {
                    self.respond(request.id, |f| async move { f.remove_dir_all(&path).await });
                }
            }
        }
    }

    fn respond<
        Func: Send + 'static + FnOnce(Arc<F>) -> Fut,
        Fut: Future<Output = Result<(), FsBackendError>> + Send,
    >(
        &self,
        request_id: Uuid,
        function: Func,
    ) {
        let sender = self.response_sender.clone();
        let fs_backend = self.fs_backend.clone();
        self.spawn_task.spawn(async move {
            let _ = sender.send(ProxyResponse::new(
                request_id,
                ProxyResponseBody::UnitAction(function(fs_backend).await.map_err(|e| e.into_cloneable())),
            ));
        });
    }
}

struct ProxyRequest {
    id: Uuid,
    body: ProxyRequestBody,
}

enum ProxyRequestBody {
    Ping,
    Stop,
    CheckExists(PathBuf),
    RemoveFile(PathBuf),
    CreateDirAll(PathBuf),
    CreateFile(PathBuf),
    Copy {
        source_path: PathBuf,
        destination_path: PathBuf,
    },
    WriteFile {
        path: PathBuf,
        content: String,
    },
    RenameFile {
        source_path: PathBuf,
        destination_path: PathBuf,
    },
    HardLink {
        source_path: PathBuf,
        destination_path: PathBuf,
    },
    RemoveDirAll(PathBuf),
}

#[derive(Clone)]
struct ProxyResponse {
    id: Uuid,
    body: ProxyResponseBody,
}

impl ProxyResponse {
    fn new(id: Uuid, body: ProxyResponseBody) -> Self {
        Self { id, body }
    }
}

#[derive(Clone)]
enum ProxyResponseBody {
    Pong,
    Stopped,
    BoolAction(Result<bool, Arc<std::io::Error>>),
    UnitAction(Result<(), Arc<std::io::Error>>),
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

#[inline(always)]
fn wrong_response_error<R>() -> Result<R, FsBackendError> {
    Err(FsBackendError::Owned(std::io::Error::other(
        "An unexpected response was emitted by the proxy thread",
    )))
}

impl ProxyFsBackend {
    pub fn new<
        F: FsBackend + Send + 'static,
        S: SpawnTask + Send + 'static,
        R: Send + 'static + FnOnce(ProxyServer<S, F>) -> (),
    >(
        capacity: usize,
        fs_backend: F,
        spawn_task: S,
        runner: R,
    ) -> Self {
        let (request_sender, request_receiver) = mpsc::channel(capacity);
        let (response_sender, response_receiver) = broadcast::channel(capacity);

        let thread_join_handle = std::thread::spawn(move || {
            let proxy_server = ProxyServer {
                request_receiver,
                response_sender,
                spawn_task,
                fs_backend: Arc::new(fs_backend),
            };

            runner(proxy_server);
        });

        Self {
            request_sender,
            response_receiver,
            thread_join_handle,
        }
    }

    pub async fn ping(&self) -> Result<(), FsBackendError> {
        match self.request(ProxyRequestBody::Ping).await? {
            ProxyResponseBody::Pong => Ok(()),
            _ => wrong_response_error(),
        }
    }

    pub async fn stop(self) -> Result<(), FsBackendError> {
        match self.request(ProxyRequestBody::Stop).await? {
            ProxyResponseBody::Stopped => Ok(()),
            _ => wrong_response_error(),
        }
    }

    pub fn thread_join_handle(&self) -> &std::thread::JoinHandle<()> {
        &self.thread_join_handle
    }

    async fn request(&self, request_body: ProxyRequestBody) -> Result<ProxyResponseBody, FsBackendError> {
        let request_id = Uuid::new_v4();
        let mut receiver = self.response_receiver.resubscribe();

        if let Err(_) = self
            .request_sender
            .send(ProxyRequest {
                id: request_id,
                body: request_body,
            })
            .await
        {
            return Err(FsBackendError::Owned(std::io::Error::new(
                std::io::ErrorKind::ConnectionRefused,
                "Request send to the proxy thread failed",
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
                        "Response recv from the proxy thread failed",
                    )))
                }
                _ => continue,
            }
        }
    }

    async fn bool_request(&self, request_body: ProxyRequestBody) -> Result<bool, FsBackendError> {
        match self.request(request_body).await? {
            ProxyResponseBody::BoolAction(result) => result.map_err(FsBackendError::Arced),
            _ => wrong_response_error(),
        }
    }

    async fn unit_request(&self, request_body: ProxyRequestBody) -> Result<(), FsBackendError> {
        match self.request(request_body).await? {
            ProxyResponseBody::UnitAction(result) => result.map_err(FsBackendError::Arced),
            _ => wrong_response_error(),
        }
    }
}

impl FsBackend for ProxyFsBackend {
    fn check_exists(&self, path: &Path) -> impl Future<Output = Result<bool, FsBackendError>> + Send {
        self.bool_request(ProxyRequestBody::CheckExists(path.to_owned()))
    }

    fn remove_file(&self, path: &Path) -> impl Future<Output = Result<(), FsBackendError>> + Send {
        self.unit_request(ProxyRequestBody::RemoveFile(path.to_owned()))
    }

    fn create_dir_all(&self, path: &Path) -> impl Future<Output = Result<(), FsBackendError>> + Send {
        self.unit_request(ProxyRequestBody::CreateDirAll(path.to_owned()))
    }

    fn create_file(&self, path: &Path) -> impl Future<Output = Result<(), FsBackendError>> + Send {
        self.unit_request(ProxyRequestBody::CreateFile(path.to_owned()))
    }

    fn copy(
        &self,
        source_path: &Path,
        destination_path: &Path,
    ) -> impl Future<Output = Result<(), FsBackendError>> + Send {
        self.unit_request(ProxyRequestBody::Copy {
            source_path: source_path.to_owned(),
            destination_path: destination_path.to_owned(),
        })
    }

    fn write_file(&self, path: &Path, content: String) -> impl Future<Output = Result<(), FsBackendError>> + Send {
        self.unit_request(ProxyRequestBody::WriteFile {
            path: path.to_owned(),
            content,
        })
    }

    fn rename_file(
        &self,
        source_path: &Path,
        destination_path: &Path,
    ) -> impl Future<Output = Result<(), FsBackendError>> + Send {
        self.unit_request(ProxyRequestBody::RenameFile {
            source_path: source_path.to_owned(),
            destination_path: destination_path.to_owned(),
        })
    }

    fn hard_link(
        &self,
        source_path: &Path,
        destination_path: &Path,
    ) -> impl Future<Output = Result<(), FsBackendError>> + Send {
        self.unit_request(ProxyRequestBody::HardLink {
            source_path: source_path.to_owned(),
            destination_path: destination_path.to_owned(),
        })
    }

    fn remove_dir_all(&self, path: &Path) -> impl Future<Output = Result<(), FsBackendError>> + Send {
        self.unit_request(ProxyRequestBody::RemoveDirAll(path.to_owned()))
    }
}
