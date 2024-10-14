use std::{future::Future, ops::Deref, path::Path, sync::Arc};

#[cfg(feature = "blocking-fs-backend")]
pub mod blocking;

#[cfg(feature = "unsend-proxy-fs-backend")]
pub mod unsend_proxy;

/// An error emitted by an FS backend, being either an owned or an arced value of a std::io::Error.
#[derive(Debug)]
pub enum FsBackendError {
    Owned(std::io::Error),
    Arced(Arc<std::io::Error>),
}

impl FsBackendError {
    /// Consume this FcBackendError into an arc that can further be cloned.
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

/// A filesystem backend provides fctools with filesystem operations on the host OS. This trait represents
/// a Send backend, as all returned futures have a Send bound. The !Send variant is the UnsendFsBackend, and
/// compatibility in one direction is provided: UnsendFsBackend is implemented for any FsBackend.
pub trait FsBackend: Send + Sync + 'static {
    fn check_exists(&self, path: &Path) -> impl Future<Output = Result<bool, FsBackendError>> + Send;

    fn remove_file(&self, path: &Path) -> impl Future<Output = Result<(), FsBackendError>> + Send;

    fn create_dir_all(&self, path: &Path) -> impl Future<Output = Result<(), FsBackendError>> + Send;

    fn create_file(&self, path: &Path) -> impl Future<Output = Result<(), FsBackendError>> + Send;

    fn write_file(&self, path: &Path, content: String) -> impl Future<Output = Result<(), FsBackendError>> + Send;

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

/// A filesystem backend variant that is !Send and thus compatible with tokio-uring, monoio, glommio and
/// other !Send async runtimes. An UnsendFsBackend, unlike FsBackend, can't be properly passed to an fctools
/// executor, VMM process or VM as they operate in a tokio Send environment, but they can be proxied and
/// used from a Send context via UnsendProxyFsBackend.
#[cfg(feature = "fs-backend-unsend")]
pub trait UnsendFsBackend {
    fn check_exists(&self, path: &Path) -> impl Future<Output = Result<bool, FsBackendError>>;

    fn remove_file(&self, path: &Path) -> impl Future<Output = Result<(), FsBackendError>>;

    fn create_dir_all(&self, path: &Path) -> impl Future<Output = Result<(), FsBackendError>>;

    fn create_file(&self, path: &Path) -> impl Future<Output = Result<(), FsBackendError>>;

    fn write_file(&self, path: &Path, content: String) -> impl Future<Output = Result<(), FsBackendError>>;

    fn rename_file(
        &self,
        source_path: &Path,
        destination_path: &Path,
    ) -> impl Future<Output = Result<(), FsBackendError>>;

    fn remove_dir_all(&self, path: &Path) -> impl Future<Output = Result<(), FsBackendError>>;

    fn copy(&self, source_path: &Path, destination_path: &Path) -> impl Future<Output = Result<(), FsBackendError>>;

    fn hard_link(
        &self,
        source_path: &Path,
        destination_path: &Path,
    ) -> impl Future<Output = Result<(), FsBackendError>>;
}

#[cfg(feature = "fs-backend-unsend")]
impl<F: FsBackend> UnsendFsBackend for F {
    fn check_exists(&self, path: &Path) -> impl Future<Output = Result<bool, FsBackendError>> {
        self.check_exists(path)
    }

    fn remove_file(&self, path: &Path) -> impl Future<Output = Result<(), FsBackendError>> {
        self.remove_file(path)
    }

    fn create_dir_all(&self, path: &Path) -> impl Future<Output = Result<(), FsBackendError>> {
        self.create_dir_all(path)
    }

    fn create_file(&self, path: &Path) -> impl Future<Output = Result<(), FsBackendError>> {
        self.create_file(path)
    }

    fn write_file(&self, path: &Path, content: String) -> impl Future<Output = Result<(), FsBackendError>> {
        self.write_file(path, content)
    }

    fn rename_file(
        &self,
        source_path: &Path,
        destination_path: &Path,
    ) -> impl Future<Output = Result<(), FsBackendError>> {
        self.rename_file(source_path, destination_path)
    }

    fn remove_dir_all(&self, path: &Path) -> impl Future<Output = Result<(), FsBackendError>> {
        self.remove_dir_all(path)
    }

    fn copy(&self, source_path: &Path, destination_path: &Path) -> impl Future<Output = Result<(), FsBackendError>> {
        self.copy(source_path, destination_path)
    }

    fn hard_link(
        &self,
        source_path: &Path,
        destination_path: &Path,
    ) -> impl Future<Output = Result<(), FsBackendError>> {
        self.hard_link(source_path, destination_path)
    }
}
