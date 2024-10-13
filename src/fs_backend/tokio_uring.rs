use std::{
    collections::HashMap,
    future::Future,
    path::{Path, PathBuf},
};

use tokio::{sync::mpsc, task::JoinHandle};
use uuid::Uuid;

use super::FsBackend;

pub struct TokioUringFsBackend {
    request_tx: mpsc::UnboundedSender<UringRequest>,
    loop_sender_tx: mpsc::UnboundedSender<(Uuid, flume::Sender<UringResponse>)>,
    loop_handler_join_handle: JoinHandle<()>,
}

#[derive(Debug)]
struct UringRequest {
    id: Uuid,
    body: UringRequestBody,
}

#[derive(Debug)]
enum UringRequestBody {
    CheckExists(PathBuf),
    RemoveFile(PathBuf),
    CreateDirAll(PathBuf),
    CreateFile(PathBuf),
    WriteToFile {
        path: PathBuf,
        content: String,
    },
    RenameFile {
        source_path: PathBuf,
        destination_path: PathBuf,
    },
    RemoveDirAll(PathBuf),
    Copy {
        source_path: PathBuf,
        destination_path: PathBuf,
    },
    HardLink {
        source_path: PathBuf,
        destination_path: PathBuf,
    },
    Terminate,
}

#[derive(Debug)]
struct UringResponse {
    id: Uuid,
    body: UringResponseBody,
}

impl From<(Uuid, UringResponseBody)> for UringResponse {
    fn from(value: (Uuid, UringResponseBody)) -> Self {
        Self {
            id: value.0,
            body: value.1,
        }
    }
}

#[derive(Debug)]
pub enum UringResponseBody {
    Empty(Result<(), std::io::Error>),
    Bool(Result<bool, std::io::Error>),
    Terminated,
}

impl TokioUringFsBackend {
    pub fn start(builder: tokio_uring::Builder) -> Self {
        let (request_tx, request_rx) = mpsc::unbounded_channel();
        let (response_tx, mut response_rx) = mpsc::unbounded_channel();
        std::thread::spawn(move || builder.start(serve_uring_requests(request_rx, response_tx)));

        let (loop_sender_tx, mut loop_sender_rx) = mpsc::unbounded_channel();
        let loop_handler_join_handle = tokio::task::spawn(async move {
            let mut map: HashMap<Uuid, flume::Sender<UringResponse>> = HashMap::new();

            loop {
                if let Ok((id, tx)) = loop_sender_rx.try_recv() {
                    map.insert(id, tx);
                }

                if let Ok(response) = response_rx.try_recv() {
                    dbg!(&response);

                    if let UringResponseBody::Terminated = response.body {
                        break;
                    }

                    if let Some(tx) = map.remove(&response.id) {
                        let _ = tx.send(response);
                    }
                }
            }
        });

        Self {
            request_tx,
            loop_sender_tx,
            loop_handler_join_handle,
        }
    }

    pub fn start_with_defaults() -> Self {
        Self::start(tokio_uring::builder())
    }

    pub fn terminate(self) -> Result<(), std::io::Error> {
        self.loop_handler_join_handle.abort();
        self.send_request_trailing(UringRequestBody::Terminate)?;
        Ok(())
    }

    fn send_request_trailing(&self, body: UringRequestBody) -> Result<Uuid, std::io::Error> {
        let id = Uuid::new_v4();
        if let Err(_) = self.request_tx.send(UringRequest { id, body }) {
            return Err(std::io::Error::other("Channel send failed for boolean request"));
        }
        Ok(id)
    }

    async fn send_request(&self, body: UringRequestBody) -> Result<UringResponse, std::io::Error> {
        let id = self.send_request_trailing(body)?;

        let (loop_tx, loop_rx) = flume::unbounded();
        if let Err(_) = self.loop_sender_tx.send((id, loop_tx)) {
            return Err(std::io::Error::other("Send on loop sender channel failed"));
        }

        match loop_rx.recv_async().await {
            Ok(r) => Ok(r),
            Err(_) => Err(std::io::Error::other("placeholder")),
        }
    }

    async fn request_bool(&self, request: UringRequestBody) -> Result<bool, std::io::Error> {
        match self.send_request(request).await?.body {
            UringResponseBody::Bool(result) => result,
            _ => Err(std::io::Error::other("Unexpected response recv-ed")),
        }
    }

    async fn request_empty(&self, request: UringRequestBody) -> Result<(), std::io::Error> {
        match self.send_request(request).await?.body {
            UringResponseBody::Empty(result) => result,
            _ => Err(std::io::Error::other("Unexpected response recv-ed")),
        }
    }
}

impl FsBackend for TokioUringFsBackend {
    fn check_exists(&self, path: &Path) -> impl Future<Output = Result<bool, std::io::Error>> + Send {
        self.request_bool(UringRequestBody::CheckExists(path.to_owned()))
    }

    fn remove_file(&self, path: &Path) -> impl Future<Output = Result<(), std::io::Error>> + Send {
        self.request_empty(UringRequestBody::RemoveFile(path.to_owned()))
    }

    fn create_dir_all(&self, path: &Path) -> impl Future<Output = Result<(), std::io::Error>> + Send {
        self.request_empty(UringRequestBody::CreateDirAll(path.to_owned()))
    }

    fn create_file(&self, path: &Path) -> impl Future<Output = Result<(), std::io::Error>> + Send {
        self.request_empty(UringRequestBody::CreateFile(path.to_owned()))
    }

    fn write_all_to_file(
        &self,
        path: &Path,
        content: String,
    ) -> impl Future<Output = Result<(), std::io::Error>> + Send {
        self.request_empty(UringRequestBody::WriteToFile {
            path: path.to_owned(),
            content,
        })
    }

    fn rename_file(
        &self,
        source_path: &Path,
        destination_path: &Path,
    ) -> impl Future<Output = Result<(), std::io::Error>> + Send {
        self.request_empty(UringRequestBody::RenameFile {
            source_path: source_path.to_owned(),
            destination_path: destination_path.to_owned(),
        })
    }

    fn remove_dir_all(&self, path: &Path) -> impl Future<Output = Result<(), std::io::Error>> + Send {
        self.request_empty(UringRequestBody::RemoveDirAll(path.to_owned()))
    }

    fn copy(
        &self,
        source_path: &Path,
        destination_path: &Path,
    ) -> impl Future<Output = Result<(), std::io::Error>> + Send {
        self.request_empty(UringRequestBody::Copy {
            source_path: source_path.to_owned(),
            destination_path: destination_path.to_owned(),
        })
    }

    fn hard_link(
        &self,
        source_path: &Path,
        destination_path: &Path,
    ) -> impl Future<Output = Result<(), std::io::Error>> + Send {
        self.request_empty(UringRequestBody::HardLink {
            source_path: source_path.to_owned(),
            destination_path: destination_path.to_owned(),
        })
    }
}

async fn serve_uring_requests(
    mut request_rx: mpsc::UnboundedReceiver<UringRequest>,
    response_tx: mpsc::UnboundedSender<UringResponse>,
) {
    while let Some(request) = request_rx.recv().await {
        let response_tx = response_tx.clone();

        match request.body {
            UringRequestBody::CheckExists(path_buf) => tokio_uring::spawn(async move {
                let exists = tokio_uring::fs::File::open(path_buf).await.is_ok();
                let _ = response_tx.send((request.id, UringResponseBody::Bool(Ok(exists))).into());
            }),
            UringRequestBody::RemoveFile(path_buf) => tokio_uring::spawn(async move {
                let _ = response_tx.send(
                    (
                        request.id,
                        UringResponseBody::Empty(tokio_uring::fs::remove_file(path_buf).await),
                    )
                        .into(),
                );
            }),
            UringRequestBody::CreateDirAll(path_buf) => tokio_uring::spawn(async move {
                let _ = response_tx.send(
                    (
                        request.id,
                        UringResponseBody::Empty(tokio_uring::fs::create_dir_all(path_buf).await),
                    )
                        .into(),
                );
            }),
            UringRequestBody::CreateFile(path_buf) => tokio_uring::spawn(async move {
                let _ = response_tx.send(
                    (
                        request.id,
                        UringResponseBody::Empty(tokio_uring::fs::File::create(path_buf).await.map(|_| ())),
                    )
                        .into(),
                );
            }),
            UringRequestBody::WriteToFile { path, content } => tokio_uring::spawn(async move {
                let response_body = match tokio_uring::fs::OpenOptions::new().write(true).open(path).await {
                    Err(err) => UringResponseBody::Empty(Err(err)),
                    Ok(file) => match file.write_all_at(Vec::from(content), 0).await.0 {
                        Ok(_) => UringResponseBody::Empty(Ok(())),
                        Err(err) => UringResponseBody::Empty(Err(err)),
                    },
                };
                let _ = response_tx.send((request.id, response_body).into());
            }),
            UringRequestBody::RenameFile {
                source_path,
                destination_path,
            } => tokio_uring::spawn(async move {
                let _ = response_tx.send(
                    (
                        request.id,
                        UringResponseBody::Empty(tokio_uring::fs::rename(source_path, destination_path).await),
                    )
                        .into(),
                );
            }),
            UringRequestBody::RemoveDirAll(path_buf) => tokio_uring::spawn(async move {
                // remove_dir_all is unimplemented in tokio_uring, thus falling back to blocking I/O
                let _ = response_tx.send(
                    (
                        request.id,
                        UringResponseBody::Empty(tokio::fs::remove_dir_all(path_buf).await),
                    )
                        .into(),
                );
            }),
            UringRequestBody::Copy {
                source_path,
                destination_path,
            } => tokio_uring::spawn(async move {
                let _ = response_tx.send(
                    (
                        request.id,
                        UringResponseBody::Empty(tokio::fs::copy(source_path, destination_path).await.map(|_| ())),
                    )
                        .into(),
                );
            }),
            UringRequestBody::HardLink {
                source_path,
                destination_path,
            } => tokio_uring::spawn(async move {
                // hard_link is unimplemented in tokio_uring, thus falling back to blocking I/O
                let _ = response_tx.send(
                    (
                        request.id,
                        UringResponseBody::Empty(tokio::fs::hard_link(source_path, destination_path).await),
                    )
                        .into(),
                );
            }),
            UringRequestBody::Terminate => {
                let _ = response_tx.send((request.id, UringResponseBody::Terminated).into());
                break;
            }
        };
    }
}

#[allow(unused)]
const BUF_SIZE: usize = 1024;

#[allow(unused)]
async fn uring_copy_impl(source_path: PathBuf, destination_path: PathBuf) -> Result<(), std::io::Error> {
    let source_file = tokio_uring::fs::OpenOptions::new().read(true).open(source_path).await?;
    let destination_file = tokio_uring::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .open(destination_path)
        .await?;

    let mut pos = 0;
    let mut buf = Vec::with_capacity(BUF_SIZE);
    loop {
        let (result, read_buf) = source_file.read_at(buf, pos).await;
        if result.is_err() {
            break;
        }

        let pos_change = result? as u64;
        if pos_change == 0 {
            break;
        }
        pos += pos_change;

        destination_file.write_all_at(read_buf, pos).await.0?;
        buf = Vec::with_capacity(BUF_SIZE);
    }

    Ok(())
}
