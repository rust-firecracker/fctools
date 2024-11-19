use std::{
    convert::Infallible,
    future::Future,
    os::unix::prelude::OwnedFd,
    path::Path,
    pin::Pin,
    sync::{Arc, OnceLock},
    time::Duration,
};

use async_executor::Executor;
use async_io::Timer;
use futures_util::{future::Either, FutureExt, TryFutureExt};
use nix::unistd::{Gid, Uid};

use super::{chownr::chownr_recursive, RuntimeAsyncFd, RuntimeExecutor, RuntimeFilesystem};

static EXECUTOR: OnceLock<Arc<Executor>> = OnceLock::new();

pub struct SmolRuntime;

impl SmolRuntime {
    pub fn initialize(executor: impl Into<Arc<Executor<'static>>>) {
        let _ = EXECUTOR.set(executor.into());
    }
}

// impl Runtime for SmolRuntime {}

pub struct SmolRuntimeExecutor;

impl RuntimeExecutor for SmolRuntimeExecutor {
    type JoinError = Infallible;

    type JoinHandle<O: Send> = Pin<Box<dyn Future<Output = Result<O, Infallible>> + Send + Sync>>;

    type TimeoutError = Infallible;

    fn spawn<F, O>(future: F) -> Self::JoinHandle<O>
    where
        F: Future<Output = O> + Send + 'static,
        O: Send + 'static,
    {
        let task = EXECUTOR.get().expect("Executor not initialized").spawn(future);
        Box::pin(task.map(|ret| Ok(ret)))
    }

    async fn timeout<F, O>(duration: Duration, future: F) -> Result<O, ()>
    where
        F: Future<Output = O> + Send,
        O: Send,
    {
        let either = futures_util::future::select(
            Box::pin(future),
            Box::pin(async move {
                Timer::after(duration).await;
                ()
            }),
        )
        .await;

        match either {
            Either::Left((output, _)) => Ok(output),
            Either::Right(_) => Err(()),
        }
    }
}

pub struct SmolRuntimeFilesystem;

impl RuntimeFilesystem for SmolRuntimeFilesystem {
    type File = async_fs::File;

    type AsyncFd = SmolRuntimeAsyncFd;

    async fn check_exists(path: &Path) -> Result<bool, std::io::Error> {
        Ok(async_fs::metadata(path).await.is_ok())
    }

    fn remove_file(path: &Path) -> impl Future<Output = Result<(), std::io::Error>> + Send {
        async_fs::remove_file(path)
    }

    fn create_dir_all(path: &Path) -> impl Future<Output = Result<(), std::io::Error>> + Send {
        async_fs::create_dir_all(path)
    }

    fn create_file(path: &Path) -> impl Future<Output = Result<(), std::io::Error>> + Send {
        async_fs::File::create(path).map_ok(|_| ())
    }

    fn write_file(path: &Path, content: String) -> impl Future<Output = Result<(), std::io::Error>> + Send {
        async_fs::write(path, content)
    }

    fn read_to_string(path: &Path) -> impl Future<Output = Result<String, std::io::Error>> + Send {
        async_fs::read_to_string(path)
    }

    fn rename_file(
        source_path: &Path,
        destination_path: &Path,
    ) -> impl Future<Output = Result<(), std::io::Error>> + Send {
        async_fs::rename(source_path, destination_path)
    }

    fn remove_dir_all(path: &Path) -> impl Future<Output = Result<(), std::io::Error>> + Send {
        async_fs::remove_dir_all(path)
    }

    fn copy(source_path: &Path, destination_path: &Path) -> impl Future<Output = Result<(), std::io::Error>> + Send {
        async_fs::copy(source_path, destination_path).map_ok(|_| ())
    }

    fn chownr(path: &Path, uid: Uid, gid: Gid) -> impl Future<Output = Result<(), std::io::Error>> + Send {
        let path = path.to_owned();
        blocking::unblock(move || chownr_recursive(&path, uid, gid))
    }

    fn hard_link(
        source_path: &Path,
        destination_path: &Path,
    ) -> impl Future<Output = Result<(), std::io::Error>> + Send {
        async_fs::hard_link(source_path, destination_path)
    }

    fn open_file_for_read(path: &Path) -> impl Future<Output = Result<Self::File, std::io::Error>> + Send {
        let mut open_options = async_fs::OpenOptions::new();
        open_options.read(true);
        open_options.open(path)
    }

    fn create_async_fd(fd: OwnedFd) -> Result<Self::AsyncFd, std::io::Error> {
        Ok(SmolRuntimeAsyncFd(async_io::Async::new(fd)?))
    }
}

pub struct SmolRuntimeAsyncFd(async_io::Async<OwnedFd>);

impl RuntimeAsyncFd for SmolRuntimeAsyncFd {
    fn readable(&self) -> impl Future<Output = Result<(), std::io::Error>> + Send {
        self.0.readable()
    }
}
