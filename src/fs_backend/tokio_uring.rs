use std::{
    future::Future,
    path::{Path, PathBuf},
};

use tokio::sync::mpsc;

use super::FsBackend;

pub struct TokioUringFsBackend {
    request_tx: mpsc::UnboundedSender<UringRequest>,
    response_rx: flume::Receiver<UringResponse>,
    join_handle: std::thread::JoinHandle<()>,
}

pub enum UringRequest {
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
}

pub enum UringResponse {
    Empty(Result<(), std::io::Error>),
    Bool(Result<bool, std::io::Error>),
}

impl TokioUringFsBackend {
    pub fn start(builder: tokio_uring::Builder) -> Self {
        let (request_tx, request_rx) = mpsc::unbounded_channel();
        let (response_tx, response_rx) = flume::unbounded();
        let join_handle = std::thread::spawn(move || builder.start(serve_uring_requests(request_rx, response_tx)));

        Self {
            request_tx,
            response_rx,
            join_handle,
        }
    }

    pub fn start_with_defaults() -> Self {
        Self::start(tokio_uring::builder())
    }

    pub fn thread_join_handle(&self) -> &std::thread::JoinHandle<()> {
        &self.join_handle
    }

    fn send_request(&self, request: UringRequest) -> Result<(), std::io::Error> {
        if let Err(_) = self.request_tx.send(request) {
            return Err(std::io::Error::other("Channel send failed for boolean request"));
        }
        Ok(())
    }

    async fn request_bool(&self, request: UringRequest) -> Result<bool, std::io::Error> {
        self.send_request(request)?;
        match self.response_rx.recv_async().await {
            Ok(UringResponse::Bool(result)) => result,
            Ok(_) => Err(std::io::Error::other("Channel recv emitted unexpected response")),
            Err(_) => Err(std::io::Error::other("Channel recv failed")),
        }
    }

    async fn request_empty(&self, request: UringRequest) -> Result<(), std::io::Error> {
        self.send_request(request)?;
        match self.response_rx.recv_async().await {
            Ok(UringResponse::Empty(result)) => result,
            Ok(_) => Err(std::io::Error::other("Channel recv an emitted unexpected response")),
            Err(_) => Err(std::io::Error::other("Channel recv failed")),
        }
    }
}

impl FsBackend for TokioUringFsBackend {
    fn check_exists(&self, path: &Path) -> impl Future<Output = Result<bool, std::io::Error>> + Send {
        self.request_bool(UringRequest::CheckExists(path.to_owned()))
    }

    fn remove_file(&self, path: &Path) -> impl Future<Output = Result<(), std::io::Error>> + Send {
        self.request_empty(UringRequest::RemoveFile(path.to_owned()))
    }

    fn create_dir_all(&self, path: &Path) -> impl Future<Output = Result<(), std::io::Error>> + Send {
        self.request_empty(UringRequest::CreateDirAll(path.to_owned()))
    }

    fn create_file(&self, path: &Path) -> impl Future<Output = Result<(), std::io::Error>> + Send {
        self.request_empty(UringRequest::CreateFile(path.to_owned()))
    }

    fn write_all_to_file(
        &self,
        path: &Path,
        content: String,
    ) -> impl Future<Output = Result<(), std::io::Error>> + Send {
        self.request_empty(UringRequest::WriteToFile {
            path: path.to_owned(),
            content,
        })
    }

    fn rename_file(
        &self,
        source_path: &Path,
        destination_path: &Path,
    ) -> impl Future<Output = Result<(), std::io::Error>> + Send {
        self.request_empty(UringRequest::RenameFile {
            source_path: source_path.to_owned(),
            destination_path: destination_path.to_owned(),
        })
    }

    fn remove_dir_all(&self, path: &Path) -> impl Future<Output = Result<(), std::io::Error>> + Send {
        self.request_empty(UringRequest::RemoveDirAll(path.to_owned()))
    }

    fn copy(
        &self,
        source_path: &Path,
        destination_path: &Path,
    ) -> impl Future<Output = Result<(), std::io::Error>> + Send {
        self.request_empty(UringRequest::Copy {
            source_path: source_path.to_owned(),
            destination_path: destination_path.to_owned(),
        })
    }

    fn hard_link(
        &self,
        source_path: &Path,
        destination_path: &Path,
    ) -> impl Future<Output = Result<(), std::io::Error>> + Send {
        self.request_empty(UringRequest::HardLink {
            source_path: source_path.to_owned(),
            destination_path: destination_path.to_owned(),
        })
    }
}

async fn serve_uring_requests(
    mut request_rx: mpsc::UnboundedReceiver<UringRequest>,
    response_tx: flume::Sender<UringResponse>,
) {
    while let Some(request) = request_rx.recv().await {
        let _ = response_tx.send(match request {
            UringRequest::CheckExists(path_buf) => {
                let exists = tokio_uring::fs::File::open(path_buf).await.is_ok();
                UringResponse::Bool(Ok(exists))
            }
            UringRequest::RemoveFile(path_buf) => UringResponse::Empty(tokio_uring::fs::remove_file(path_buf).await),
            UringRequest::CreateDirAll(path_buf) => {
                UringResponse::Empty(tokio_uring::fs::create_dir_all(path_buf).await)
            }
            UringRequest::CreateFile(path_buf) => {
                UringResponse::Empty(tokio_uring::fs::File::create(path_buf).await.map(|_| ()))
            }
            UringRequest::WriteToFile { path, content } => {
                match tokio_uring::fs::OpenOptions::new().write(true).open(path).await {
                    Err(err) => UringResponse::Empty(Err(err)),
                    Ok(file) => match file.write_all_at(Vec::from(content), 0).await.0 {
                        Ok(_) => UringResponse::Empty(Ok(())),
                        Err(err) => UringResponse::Empty(Err(err)),
                    },
                }
            }
            UringRequest::RenameFile {
                source_path,
                destination_path,
            } => UringResponse::Empty(tokio_uring::fs::rename(source_path, destination_path).await),
            UringRequest::RemoveDirAll(path_buf) => {
                // remove_dir_all is unimplemented in tokio_uring, thus falling back to blocking I/O
                UringResponse::Empty(tokio::fs::remove_dir_all(path_buf).await)
            }
            UringRequest::Copy {
                source_path,
                destination_path,
            } => UringResponse::Empty(tokio::fs::copy(source_path, destination_path).await.map(|_| ())),
            UringRequest::HardLink {
                source_path,
                destination_path,
            } => {
                // hard_link is unimplemented in tokio_uring, thus falling back to blocking I/O
                UringResponse::Empty(tokio::fs::hard_link(source_path, destination_path).await)
            }
        });
    }
}

const BUF_SIZE: usize = 1024;

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
