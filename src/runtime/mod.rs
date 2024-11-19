use std::{future::Future, os::fd::OwnedFd, path::Path, pin::Pin, process::ExitStatus, time::Duration};

use futures_io::{AsyncRead, AsyncWrite};
use nix::unistd::{Gid, Uid};

#[cfg(feature = "tokio-runtime")]
pub mod tokio;

#[cfg(feature = "smol-runtime")]
pub mod smol;

#[cfg(any(feature = "tokio-runtime", feature = "smol-runtime"))]
mod chownr;

pub trait Runtime: 'static {
    type Executor: RuntimeExecutor;
    type Filesystem: RuntimeFilesystem;
    type Process: RuntimeProcess;
    type HyperExecutor: hyper::rt::Executor<Pin<Box<dyn Future<Output = ()> + Send>>> + Clone + Send + Sync + 'static;

    fn get_hyper_executor() -> Self::HyperExecutor;

    fn get_hyper_client_sockets_backend() -> hyper_client_sockets::Backend;
}

pub trait RuntimeExecutor {
    type JoinError: std::error::Error + std::fmt::Debug + Send + Sync;
    type JoinHandle<O: Send>: Future<Output = Result<O, Self::JoinError>> + Send + Sync;
    type TimeoutError: std::error::Error + std::fmt::Debug + Send + Sync;

    fn spawn<F, O>(future: F) -> Self::JoinHandle<O>
    where
        F: Future<Output = O> + Send + 'static,
        O: Send + 'static;

    fn timeout<F, O>(duration: Duration, future: F) -> impl Future<Output = Result<O, Self::TimeoutError>> + Send
    where
        F: Future<Output = O> + Send,
        O: Send;
}

pub trait RuntimeFilesystem {
    type File: AsyncRead + AsyncWrite + Send + Unpin;
    type AsyncFd: RuntimeAsyncFd;

    fn check_exists(path: &Path) -> impl Future<Output = Result<bool, std::io::Error>> + Send;

    fn remove_file(path: &Path) -> impl Future<Output = Result<(), std::io::Error>> + Send;

    fn create_dir_all(path: &Path) -> impl Future<Output = Result<(), std::io::Error>> + Send;

    fn create_file(path: &Path) -> impl Future<Output = Result<(), std::io::Error>> + Send;

    fn write_file(path: &Path, content: String) -> impl Future<Output = Result<(), std::io::Error>> + Send;

    fn read_to_string(path: &Path) -> impl Future<Output = Result<String, std::io::Error>> + Send;

    fn rename_file(
        source_path: &Path,
        destination_path: &Path,
    ) -> impl Future<Output = Result<(), std::io::Error>> + Send;

    fn remove_dir_all(path: &Path) -> impl Future<Output = Result<(), std::io::Error>> + Send;

    fn copy(source_path: &Path, destination_path: &Path) -> impl Future<Output = Result<(), std::io::Error>> + Send;

    fn chownr(path: &Path, uid: Uid, gid: Gid) -> impl Future<Output = Result<(), std::io::Error>> + Send;

    fn hard_link(
        source_path: &Path,
        destination_path: &Path,
    ) -> impl Future<Output = Result<(), std::io::Error>> + Send;

    fn open_file_for_read(path: &Path) -> impl Future<Output = Result<Self::File, std::io::Error>> + Send;

    fn create_async_fd(fd: OwnedFd) -> Result<Self::AsyncFd, std::io::Error>;
}

pub trait RuntimeAsyncFd: Send {
    fn readable(&self) -> impl Future<Output = Result<(), std::io::Error>> + Send;
}

pub trait RuntimeProcess: Sized + Send + Sync + std::fmt::Debug {
    type Stdout: AsyncRead + Unpin + Send;
    type Stderr: AsyncRead + Unpin + Send;
    type Stdin: AsyncWrite + Unpin + Send;

    fn spawn(command: std::process::Command) -> Result<Self, std::io::Error>;

    fn output(
        command: std::process::Command,
    ) -> impl Future<Output = Result<std::process::Output, std::io::Error>> + Send;

    fn try_wait(&mut self) -> Result<Option<ExitStatus>, std::io::Error>;

    fn wait(&mut self) -> impl Future<Output = Result<ExitStatus, std::io::Error>> + Send;

    fn kill(&mut self) -> Result<(), std::io::Error>;

    fn stdout(&mut self) -> &mut Option<Self::Stdout>;

    fn stderr(&mut self) -> &mut Option<Self::Stderr>;

    fn stdin(&mut self) -> &mut Option<Self::Stdin>;

    fn take_stdout(&mut self) -> Option<Self::Stdout>;

    fn take_stderr(&mut self) -> Option<Self::Stderr>;

    fn take_stdin(&mut self) -> Option<Self::Stdin>;
}

pub struct RuntimeJoinSet<O: Send + 'static, E: RuntimeExecutor> {
    join_handles: Vec<E::JoinHandle<Result<(), O>>>,
}

impl<O: Send + 'static, E: RuntimeExecutor> RuntimeJoinSet<O, E> {
    pub fn new() -> Self {
        Self {
            join_handles: Vec::new(),
        }
    }

    pub async fn join_two<F1, F2>(future1: F1, future2: F2) -> Result<Result<(), O>, E::JoinError>
    where
        F1: Future<Output = Result<(), O>> + Send + 'static,
        F2: Future<Output = Result<(), O>> + Send + 'static,
    {
        let mut join_set = Self::new();
        join_set.spawn(future1);
        join_set.spawn(future2);
        join_set.wait().await
    }

    pub fn spawn<F>(&mut self, future: F)
    where
        F: Future<Output = Result<(), O>> + Send + 'static,
    {
        self.join_handles.push(E::spawn(future));
    }

    pub async fn wait(self) -> Result<Result<(), O>, E::JoinError> {
        for join_handle in self.join_handles {
            match join_handle.await {
                Ok(result) => match result {
                    Ok(()) => {}
                    Err(err) => return Ok(Err(err)),
                },
                Err(err) => return Err(err),
            }
        }

        Ok(Ok(()))
    }
}
