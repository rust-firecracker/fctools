use std::{
    future::Future,
    path::{Path, PathBuf},
    rc::Rc,
    sync::Arc,
};

use nix::unistd::{Gid, Uid};
use tokio::sync::{broadcast, mpsc};
use uuid::Uuid;

use super::{FsBackend, FsBackendError, UnsendFsBackend};

/// An [FsBackend] that runs an [UnsendFsBackend] on a separate OS thread and wraps the not-[Send] backend
/// in a [Send] context by exchanging requests and responses to and from the OS thread via a channel
/// pair ([tokio::sync::mpsc] and [tokio::sync::broadcast]) used internally.
///
/// A minor change the [UnsendProxy] makes to circumvent [std::io::Error] being not [Clone] is that all owned
/// [FsBackendError]-s emitted by the [UnsendFsBackend] will be converted into [Arc]-ed errors. The [Clone]
/// requirement is needed due to a [tokio::sync::broadcast] channel being used as a response recv channel on this
/// [Send] side.
pub struct UnsendProxyFsBackend {
    request_sender: mpsc::Sender<ProxyRequest>,
    response_receiver: broadcast::Receiver<ProxyResponse>,
    thread_join_handle: std::thread::JoinHandle<()>,
}

/// The not-[Send] end of an [UnsendProxyFsBackend] that is executed on the dedicated thread, and wraps the
/// [UnsendFsBackend] via an in-memory channel server that is accessed through the other end.
///
/// The [UnsendProxy::run] future is not [Send] and must be blocked on via the chosen async runtime on the thread. Usually
/// a Send runtime like Tokio multi_thread should not be used, since your backend would then already be
/// Send and an UnsendProxyFsBackend wrapper would not be needed.
pub struct UnsendProxy<S: SpawnTask, F: UnsendFsBackend> {
    request_receiver: mpsc::Receiver<ProxyRequest>,
    response_sender: broadcast::Sender<ProxyResponse>,
    spawn_task: S,
    fs_backend: Rc<F>,
}

impl<S: SpawnTask, F: UnsendFsBackend + 'static> UnsendProxy<S, F> {
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
                ProxyRequestBody::ReadToString(path) => {
                    let sender = self.response_sender.clone();
                    let fs_backend = self.fs_backend.clone();
                    self.spawn_task.spawn(async move {
                        sender.send(ProxyResponse::new(
                            request.id,
                            ProxyResponseBody::StringAction(
                                fs_backend.read_to_string(&path).await.map_err(|e| e.into_cloneable()),
                            ),
                        ))
                    });
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
                ProxyRequestBody::Chownr { uid, gid, path } => {
                    self.respond(request.id, move |f| async move { f.chownr(&path, uid, gid).await });
                }
                ProxyRequestBody::RemoveDirAll(path) => {
                    self.respond(request.id, |f| async move { f.remove_dir_all(&path).await });
                }
            }
        }
    }

    fn respond<Func: 'static + FnOnce(Rc<F>) -> Fut, Fut: Future<Output = Result<(), FsBackendError>>>(
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
    ReadToString(PathBuf),
    RenameFile {
        source_path: PathBuf,
        destination_path: PathBuf,
    },
    HardLink {
        source_path: PathBuf,
        destination_path: PathBuf,
    },
    Chownr {
        uid: Uid,
        gid: Gid,
        path: PathBuf,
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
    StringAction(Result<String, Arc<std::io::Error>>),
}

/// [SpawnTask] is a lean trait that abstracts over spawning a potentially [Send] future onto an async runtime.
pub trait SpawnTask {
    fn spawn<F>(&self, future: F)
    where
        F: Future + 'static,
        F::Output: 'static;
}

#[inline(always)]
fn wrong_response_error<R>() -> Result<R, FsBackendError> {
    Err(FsBackendError::Owned(std::io::Error::other(
        "An unexpected response was emitted by the proxy thread",
    )))
}

impl UnsendProxyFsBackend {
    /// Create a [UnsendProxyFsBackend] using the given [SpawnTask], proxied [UnsendFsBackend], runner function and channel capacity.
    /// The runner function is expected to instantiate your async runtime of choice that is compatible with the provided [SpawnTask],
    /// and block it on the [UnsendProxy::run] future.
    pub fn new<
        F: UnsendFsBackend + Send + 'static,
        S: SpawnTask + Send + 'static,
        R: Send + 'static + FnOnce(UnsendProxy<S, F>) -> (),
    >(
        capacity: usize,
        fs_backend: F,
        spawn_task: S,
        runner: R,
    ) -> Self {
        let (request_sender, request_receiver) = mpsc::channel(capacity);
        let (response_sender, response_receiver) = broadcast::channel(capacity);

        let thread_join_handle = std::thread::spawn(move || {
            let proxy_server = UnsendProxy {
                request_receiver,
                response_sender,
                spawn_task,
                fs_backend: Rc::new(fs_backend),
            };

            runner(proxy_server);
        });

        Self {
            request_sender,
            response_receiver,
            thread_join_handle,
        }
    }

    /// Ping the proxy thread to determine whether the communication channels are still intact.
    pub async fn ping(&self) -> Result<(), FsBackendError> {
        match self.request(ProxyRequestBody::Ping).await? {
            ProxyResponseBody::Pong => Ok(()),
            _ => wrong_response_error(),
        }
    }

    /// Stop the proxy thread, should be always called to ensure the proxy thread doesn't hang up the whole process.
    pub async fn stop(self) -> Result<(), FsBackendError> {
        match self.request(ProxyRequestBody::Stop).await? {
            ProxyResponseBody::Stopped => Ok(()),
            _ => wrong_response_error(),
        }
    }

    /// Get a reference to the proxy thread's [std::thread::JoinHandle].
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

    async fn string_request(&self, request_body: ProxyRequestBody) -> Result<String, FsBackendError> {
        match self.request(request_body).await? {
            ProxyResponseBody::StringAction(result) => result.map_err(FsBackendError::Arced),
            _ => wrong_response_error(),
        }
    }
}

impl FsBackend for UnsendProxyFsBackend {
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

    fn read_to_string(&self, path: &Path) -> impl Future<Output = Result<String, FsBackendError>> + Send {
        self.string_request(ProxyRequestBody::ReadToString(path.to_owned()))
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

    fn chownr(&self, path: &Path, uid: Uid, gid: Gid) -> impl Future<Output = Result<(), FsBackendError>> + Send {
        self.unit_request(ProxyRequestBody::Chownr {
            uid,
            gid,
            path: path.to_owned(),
        })
    }
}
