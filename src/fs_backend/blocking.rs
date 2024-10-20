use std::path::Path;

use super::{FsBackend, FsBackendError};

/// An [FsBackend] that uses [tokio::fs] internally, which simply wraps blocking I/O in Tokio's
/// [tokio::task::spawn_blocking] thread-pool. The performance of async-ified blocking I/O is proven to
/// be less than that of io-uring solutions that are fully async, but those are currently unacceptably
/// immature in the Rust ecosystem, so the [BlockingFsBackend] is still the recommended option.
pub struct BlockingFsBackend;

impl FsBackend for BlockingFsBackend {
    async fn check_exists(&self, path: &Path) -> Result<bool, FsBackendError> {
        tokio::fs::try_exists(path).await.map_err(FsBackendError::Owned)
    }

    async fn remove_file(&self, path: &Path) -> Result<(), FsBackendError> {
        tokio::fs::remove_file(path).await.map_err(FsBackendError::Owned)
    }

    async fn create_dir_all(&self, path: &Path) -> Result<(), FsBackendError> {
        tokio::fs::create_dir_all(path).await.map_err(FsBackendError::Owned)
    }

    async fn create_file(&self, path: &Path) -> Result<(), FsBackendError> {
        tokio::fs::File::create(path)
            .await
            .map(|_| ())
            .map_err(FsBackendError::Owned)
    }

    async fn write_file(&self, path: &Path, content: String) -> Result<(), FsBackendError> {
        tokio::fs::write(path, content).await.map_err(FsBackendError::Owned)
    }

    async fn rename_file(&self, source_path: &Path, destination_path: &Path) -> Result<(), FsBackendError> {
        tokio::fs::rename(source_path, destination_path)
            .await
            .map_err(FsBackendError::Owned)
    }

    async fn remove_dir_all(&self, path: &Path) -> Result<(), FsBackendError> {
        tokio::fs::remove_dir_all(path).await.map_err(FsBackendError::Owned)
    }

    async fn copy(&self, source_path: &Path, destination_path: &Path) -> Result<(), FsBackendError> {
        tokio::fs::copy(source_path, destination_path)
            .await
            .map(|_| ())
            .map_err(FsBackendError::Owned)
    }

    async fn hard_link(&self, source_path: &Path, destination_path: &Path) -> Result<(), FsBackendError> {
        tokio::fs::hard_link(source_path, destination_path)
            .await
            .map_err(FsBackendError::Owned)
    }
}
