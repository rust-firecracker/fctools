use std::{
    future::{Future, IntoFuture},
    path::Path,
    pin::Pin,
};

use tokio::task::JoinSet;

use super::{FsBackend, FsOperation};

struct BlockingFsOperation<R: Send + 'static, F: Send + 'static + FnOnce() -> Result<R, std::io::Error>>(F);

impl<R: Send + 'static, F: Send + 'static + FnOnce() -> Result<R, std::io::Error>> FsOperation<R>
    for BlockingFsOperation<R, F>
{
    fn offload<E: From<std::io::Error> + Send + 'static>(self, join_set: &mut JoinSet<Result<R, E>>) {
        join_set.spawn_blocking(move || (self.0)().map_err(E::from));
    }
}

impl<R: Send + 'static, F: Send + 'static + FnOnce() -> Result<R, std::io::Error>> IntoFuture
    for BlockingFsOperation<R, F>
{
    type Output = Result<R, std::io::Error>;

    type IntoFuture = Pin<Box<dyn Future<Output = Self::Output> + Send + 'static>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move {
            match tokio::task::spawn_blocking(self.0).await {
                Ok(result) => result,
                Err(err) => Err(std::io::Error::other(format!(
                    "Joining on a blocking task failed: {err}"
                ))),
            }
        })
    }
}

#[derive(Debug)]
pub struct BlockingFsBackend;

impl FsBackend for BlockingFsBackend {
    fn check_exists(&self, path: &Path) -> impl FsOperation<bool> {
        let path = path.to_owned();
        BlockingFsOperation(move || std::fs::exists(path))
    }

    fn remove_file(&self, path: &Path) -> impl FsOperation<()> {
        let path = path.to_owned();
        BlockingFsOperation(move || std::fs::remove_file(path))
    }

    fn create_dir_all(&self, path: &Path) -> impl FsOperation<()> {
        let path = path.to_owned();
        BlockingFsOperation(move || std::fs::create_dir_all(path))
    }

    fn create_file(&self, path: &Path) -> impl FsOperation<()> {
        let path = path.to_owned();
        BlockingFsOperation(move || std::fs::File::create(path).map(|_| ()))
    }

    fn write_all_to_file(&self, path: &Path, content: String) -> impl FsOperation<()> {
        let path = path.to_owned();
        BlockingFsOperation(move || std::fs::write(path, content))
    }

    fn rename_file(&self, source_path: &Path, destination_path: &Path) -> impl FsOperation<()> {
        let (source_path, destination_path) = (source_path.to_owned(), destination_path.to_owned());
        BlockingFsOperation(move || std::fs::rename(source_path, destination_path))
    }

    fn remove_dir_all(&self, path: &Path) -> impl FsOperation<()> {
        let path = path.to_owned();
        BlockingFsOperation(move || std::fs::remove_dir_all(path))
    }

    fn copy(&self, source_path: &Path, destination_path: &Path) -> impl FsOperation<()> {
        let (source_path, destination_path) = (source_path.to_owned(), destination_path.to_owned());
        BlockingFsOperation(move || std::fs::copy(source_path, destination_path).map(|_| ()))
    }

    fn hard_link(&self, source_path: &Path, destination_path: &Path) -> impl FsOperation<()> {
        let (source_path, destination_path) = (source_path.to_owned(), destination_path.to_owned());
        BlockingFsOperation(move || std::fs::hard_link(source_path, destination_path))
    }
}
