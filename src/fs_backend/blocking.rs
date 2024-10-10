use std::future::Future;

use tokio::task::JoinSet;

use super::{FsBackend, FsOperation};

struct BlockingFsOperation<R: Send + 'static> {
    action: Box<dyn Send + FnOnce() -> Result<R, std::io::Error>>,
}

impl<R: Send + 'static> BlockingFsOperation<R> {
    fn new(action: impl Send + 'static + FnOnce() -> Result<R, std::io::Error>) -> Self {
        Self {
            action: Box::new(action),
        }
    }
}

impl<R: Send + 'static> FsOperation<R> for BlockingFsOperation<R> {
    fn offload<I>(self, join_set: &mut JoinSet<Result<R, std::io::Error>>) {
        join_set.spawn_blocking(self.action);
    }

    fn block_on(self) -> impl Future<Output = Result<R, std::io::Error>> + Send {
        async move {
            match tokio::task::spawn_blocking(self.action).await {
                Ok(result) => result,
                Err(err) => Err(std::io::Error::other(format!(
                    "Joining on a blocking task failed: {err}"
                ))),
            }
        }
    }
}

#[derive(Debug)]
pub struct BlockingFsBackend;

impl FsBackend for BlockingFsBackend {
    fn check_exists(&self, path: &std::path::Path) -> impl FsOperation<bool> {
        let path = path.to_owned();
        BlockingFsOperation::new(move || std::fs::exists(path))
    }
}
