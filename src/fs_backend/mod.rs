use std::{future::Future, ops::Deref, path::Path, sync::Arc};

use tokio::io::{AsyncRead, AsyncSeek, AsyncWrite};

#[cfg(feature = "blocking-fs-backend")]
pub mod blocking;

#[cfg(feature = "thread-bridge-fs-backend")]
pub mod thread_bridge;

/// A trait for a file handle emitted by filesystem backend that is Send and Unpin and can be read from,
/// written to and seeked. Serves to hide the underlying File struct behind a Pin<Box<dyn FsFileHandle>>
/// trait object.
pub trait FsFileHandle: AsyncRead + AsyncSeek + AsyncWrite + Send + Unpin {}

#[derive(Debug)]
pub enum FsBackendError {
    Owned(std::io::Error),
    Arced(Arc<std::io::Error>),
}

impl FsBackendError {
    pub fn into_cloneable(self) -> Arc<std::io::Error> {
        match self {
            FsBackendError::Owned(error) => Arc::new(error),
            FsBackendError::Arced(arc) => arc,
        }
    }
}

impl Deref for FsBackendError {
    type Target = std::io::Error;

    fn deref(&self) -> &Self::Target {
        match self {
            FsBackendError::Owned(owned_value) => owned_value,
            FsBackendError::Arced(arc) => arc.as_ref(),
        }
    }
}

impl std::fmt::Display for FsBackendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FsBackendError::Owned(error) => error.fmt(f),
            FsBackendError::Arced(arc) => arc.fmt(f),
        }
    }
}

/// A filesystem backend provides fctools with filesystem operations on the host OS. The primary two viable
/// implementations of a filesystem backend on a modern Linux system are either blocking epoll wrapped in
/// Tokio's spawn_blocking, or asynchronous io-uring.
pub trait FsBackend: Send + Sync + 'static {
    fn check_exists(&self, path: &Path) -> impl Future<Output = Result<bool, FsBackendError>> + Send;

    fn remove_file(&self, path: &Path) -> impl Future<Output = Result<(), FsBackendError>> + Send;

    fn create_dir_all(&self, path: &Path) -> impl Future<Output = Result<(), FsBackendError>> + Send;

    fn create_file(&self, path: &Path) -> impl Future<Output = Result<(), FsBackendError>> + Send;

    fn write_all_to_file(
        &self,
        path: &Path,
        content: String,
    ) -> impl Future<Output = Result<(), FsBackendError>> + Send;

    fn rename_file(
        &self,
        source_path: &Path,
        destination_path: &Path,
    ) -> impl Future<Output = Result<(), FsBackendError>> + Send;

    fn remove_dir_all(&self, path: &Path) -> impl Future<Output = Result<(), FsBackendError>> + Send;

    fn copy(
        &self,
        source_path: &Path,
        destination_path: &Path,
    ) -> impl Future<Output = Result<(), FsBackendError>> + Send;

    fn hard_link(
        &self,
        source_path: &Path,
        destination_path: &Path,
    ) -> impl Future<Output = Result<(), FsBackendError>> + Send;
}
