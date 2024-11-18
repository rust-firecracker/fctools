use std::{future::Future, path::Path, process::ExitStatus};

use futures_io::{AsyncRead, AsyncWrite};
use nix::unistd::{Gid, Uid};

pub mod tokio;

pub trait Runtime {
    type Executor: RuntimeExecutor;
    type Filesystem: RuntimeFilesystem;
    type Process: RuntimeProcess;
}

pub trait RuntimeExecutor {
    type JoinError: std::error::Error + std::fmt::Debug;
    type JoinHandle<O>: Future<Output = Result<O, Self::JoinError>>;

    fn spawn<F, O>(future: F) -> Self::JoinHandle<O>
    where
        F: Future<Output = O> + Send + 'static,
        O: Send + 'static;

    fn spawn_blocking<F, O>(function: F) -> Self::JoinHandle<O>
    where
        F: FnOnce() -> O + Send + 'static,
        O: Send + 'static;
}

pub trait RuntimeFilesystem {
    fn check_exists(path: &Path) -> impl Future<Output = Result<bool, std::io::Error>> + Send;

    fn remove_file(path: &Path) -> impl Future<Output = Result<(), std::io::Error>> + Send;

    fn create_dir_all(path: &Path) -> impl Future<Output = Result<(), std::io::Error>> + Send;

    fn create_file(path: &Path) -> impl Future<Output = Result<(), std::io::Error>> + Send;

    fn write_file(path: &Path, content: String) -> impl Future<Output = Result<(), std::io::Error>> + Send;

    fn read_to_string(path: &Path) -> impl Future<Output = Result<String, std::io::Error>> + Send;

    fn rename_file(
        &self,
        source_path: &Path,
        destination_path: &Path,
    ) -> impl Future<Output = Result<(), std::io::Error>> + Send;

    fn remove_dir_all(path: &Path) -> impl Future<Output = Result<(), std::io::Error>> + Send;

    fn copy(source_path: &Path, destination_path: &Path) -> impl Future<Output = Result<(), std::io::Error>> + Send;

    fn chownr(path: &Path, uid: Uid, gid: Gid) -> impl Future<Output = Result<(), std::io::Error>> + Send;

    fn hard_link(
        &self,
        source_path: &Path,
        destination_path: &Path,
    ) -> impl Future<Output = Result<(), std::io::Error>> + Send;
}

pub trait RuntimeProcess: Sized {
    type Stdout: AsyncRead + Unpin;
    type Stderr: AsyncRead + Unpin;
    type Stdin: AsyncWrite + Unpin;

    fn spawn(command: std::process::Command) -> Result<Self, std::io::Error>;

    fn try_wait(&mut self) -> Result<Option<ExitStatus>, std::io::Error>;

    fn wait(&mut self) -> impl Future<Output = Result<ExitStatus, std::io::Error>>;

    fn kill(&mut self) -> Result<(), std::io::Error>;

    fn take_stdout(&mut self) -> Option<Self::Stdout>;

    fn take_stderr(&mut self) -> Option<Self::Stderr>;

    fn take_stdin(&mut self) -> Option<Self::Stdin>;
}
