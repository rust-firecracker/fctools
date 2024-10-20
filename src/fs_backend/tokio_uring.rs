use std::path::Path;

use super::{FsBackendError, UnsendFsBackend};

/// An [UnsendFsBackend] that uses tokio-uring with io-uring for all its operations except
/// remove_dir_all, copy, hard_link, where it falls back to [tokio::fs] (which is the same as the
/// blocking Send backend), as these operations are unimplemented in tokio-uring.
pub struct TokioUringFsBackend;

impl UnsendFsBackend for TokioUringFsBackend {
    async fn check_exists(&self, path: &Path) -> Result<bool, FsBackendError> {
        Ok(tokio_uring::fs::statx(path).await.is_ok())
    }

    async fn remove_file(&self, path: &Path) -> Result<(), FsBackendError> {
        tokio_uring::fs::remove_file(path).await.map_err(FsBackendError::Owned)
    }

    async fn create_dir_all(&self, path: &Path) -> Result<(), FsBackendError> {
        tokio_uring::fs::create_dir_all(path)
            .await
            .map_err(FsBackendError::Owned)
    }

    async fn create_file(&self, path: &Path) -> Result<(), FsBackendError> {
        let file = tokio_uring::fs::File::create(path)
            .await
            .map_err(FsBackendError::Owned)?;
        file.close().await.map_err(FsBackendError::Owned)
    }

    async fn rename_file(&self, source_path: &Path, destination_path: &Path) -> Result<(), FsBackendError> {
        tokio_uring::fs::rename(source_path, destination_path)
            .await
            .map_err(FsBackendError::Owned)
    }

    async fn write_file(&self, path: &Path, content: String) -> Result<(), FsBackendError> {
        let file = tokio_uring::fs::OpenOptions::new()
            .truncate(true)
            .write(true)
            .create(true)
            .open(path)
            .await
            .map_err(FsBackendError::Owned)?;
        file.write_all_at(content.into_bytes(), 0)
            .await
            .0
            .map_err(FsBackendError::Owned)?;
        file.close().await.map_err(FsBackendError::Owned)
    }

    async fn remove_dir_all(&self, path: &Path) -> Result<(), FsBackendError> {
        // remove_dir_all is currently not supported by tokio-uring
        tokio::fs::remove_dir_all(path).await.map_err(FsBackendError::Owned)
    }

    async fn copy(&self, source_path: &Path, destination_path: &Path) -> Result<(), FsBackendError> {
        // copy is currently not supported by tokio-uring
        tokio::fs::copy(source_path, destination_path)
            .await
            .map(|_| ())
            .map_err(FsBackendError::Owned)
    }

    async fn hard_link(&self, source_path: &Path, destination_path: &Path) -> Result<(), FsBackendError> {
        // hard_link is currently not supported by tokio-uring
        tokio::fs::hard_link(source_path, destination_path)
            .await
            .map_err(FsBackendError::Owned)
    }
}
