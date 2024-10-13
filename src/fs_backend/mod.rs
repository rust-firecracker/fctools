use std::{
    future::{Future, IntoFuture},
    path::Path,
    pin::Pin,
};

use tokio::task::JoinSet;

#[cfg(feature = "blocking-fs-backend")]
pub mod blocking;

pub trait FsOperation<R: Send + 'static>:
    IntoFuture<
        Output = Result<R, std::io::Error>,
        IntoFuture = Pin<Box<dyn Future<Output = Result<R, std::io::Error>> + Send>>,
    > + Sized
{
    fn offload<E: From<std::io::Error> + Send + 'static>(self, join_set: &mut JoinSet<Result<R, E>>);
}

pub trait FsBackend: Send + Sync + 'static {
    fn check_exists(&self, path: &Path) -> impl FsOperation<bool>;

    fn remove_file(&self, path: &Path) -> impl FsOperation<()>;

    fn create_dir_all(&self, path: &Path) -> impl FsOperation<()>;

    fn create_file(&self, path: &Path) -> impl FsOperation<()>;

    fn write_all_to_file(&self, path: &Path, content: String) -> impl FsOperation<()>;

    fn remove_dir_all(&self, path: &Path) -> impl FsOperation<()>;

    fn copy(&self, source_path: &Path, destination_path: &Path) -> impl FsOperation<()>;

    fn hard_link(&self, source_path: &Path, destination_path: &Path) -> impl FsOperation<()>;
}
