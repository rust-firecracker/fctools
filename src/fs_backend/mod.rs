use std::{future::Future, path::Path};

use tokio::io::{AsyncRead, AsyncSeek, AsyncWrite};

#[cfg(feature = "blocking-fs-backend")]
pub mod blocking;

/// A trait for a file handle emitted by filesystem backend that is Send and Unpin and can be read from,
/// written to and seeked. Serves to hide the underlying File struct behind a Pin<Box<dyn FsFileHandle>>
/// trait object.
pub trait FsFileHandle: AsyncRead + AsyncSeek + AsyncWrite + Send + Unpin {}

/// A filesystem backend provides fctools with filesystem operations on the host OS. The primary two viable
/// implementations of a filesystem backend on a modern Linux system are either blocking epoll wrapped in
/// Tokio's spawn_blocking, or asynchronous io-uring.
pub trait FsBackend: Send + Sync + 'static {
    fn check_exists(&self, path: &Path) -> impl Future<Output = Result<bool, std::io::Error>> + Send;

    fn remove_file(&self, path: &Path) -> impl Future<Output = Result<(), std::io::Error>> + Send;

    fn create_dir_all(&self, path: &Path) -> impl Future<Output = Result<(), std::io::Error>> + Send;

    fn create_file(&self, path: &Path) -> impl Future<Output = Result<(), std::io::Error>> + Send;

    fn write_all_to_file(
        &self,
        path: &Path,
        content: String,
    ) -> impl Future<Output = Result<(), std::io::Error>> + Send;

    fn rename_file(
        &self,
        source_path: &Path,
        destination_path: &Path,
    ) -> impl Future<Output = Result<(), std::io::Error>> + Send;

    fn remove_dir_all(&self, path: &Path) -> impl Future<Output = Result<(), std::io::Error>> + Send;

    fn copy(
        &self,
        source_path: &Path,
        destination_path: &Path,
    ) -> impl Future<Output = Result<(), std::io::Error>> + Send;

    fn hard_link(
        &self,
        source_path: &Path,
        destination_path: &Path,
    ) -> impl Future<Output = Result<(), std::io::Error>> + Send;
}
