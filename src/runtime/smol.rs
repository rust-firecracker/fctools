use std::{
    convert::Infallible,
    future::Future,
    os::unix::prelude::OwnedFd,
    path::Path,
    pin::Pin,
    sync::{Arc, OnceLock},
    time::{Duration, Instant},
};

use async_executor::Executor;
use async_io::Timer;
use async_process::{Child, ChildStderr, ChildStdin, ChildStdout};
use futures_util::{future::Either, FutureExt, TryFutureExt};
use nix::unistd::{Gid, Uid};
use smol_hyper::rt::SmolExecutor;

use super::{chownr::chownr_recursive, Runtime, RuntimeAsyncFd, RuntimeExecutor, RuntimeFilesystem, RuntimeProcess};

static EXECUTOR: OnceLock<Arc<Executor>> = OnceLock::new();

pub struct SmolRuntime;

impl SmolRuntime {
    pub fn initialize(executor: impl Into<Arc<Executor<'static>>>) {
        let _ = EXECUTOR.set(executor.into());
    }
}

impl Runtime for SmolRuntime {
    type Executor = SmolRuntimeExecutor;

    type Filesystem = SmolRuntimeFilesystem;

    type Process = SmolRuntimeProcess;

    #[cfg(feature = "vmm-process")]
    type HyperExecutor = SmolExecutor<Arc<Executor<'static>>>;

    #[cfg(feature = "vmm-process")]
    fn get_hyper_executor() -> Self::HyperExecutor {
        SmolExecutor::new(EXECUTOR.get().expect("Executor not initialized").clone())
    }

    #[cfg(feature = "vmm-process")]
    fn get_hyper_client_sockets_backend() -> hyper_client_sockets::Backend {
        hyper_client_sockets::Backend::AsyncIo
    }
}

pub struct SmolRuntimeExecutor;

#[derive(Debug, thiserror::Error)]
pub struct SmolTimeoutError {
    pub instant: Instant,
}

impl std::fmt::Display for SmolTimeoutError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Timed out at {:?}", self.instant)
    }
}

impl RuntimeExecutor for SmolRuntimeExecutor {
    type JoinError = Infallible;

    type JoinHandle<O: Send> = Pin<Box<dyn Future<Output = Result<O, Infallible>> + Send + Sync>>;

    type TimeoutError = SmolTimeoutError;

    fn spawn<F, O>(future: F) -> Self::JoinHandle<O>
    where
        F: Future<Output = O> + Send + 'static,
        O: Send + 'static,
    {
        let task = EXECUTOR.get().expect("Executor not initialized").spawn(future);
        Box::pin(task.map(|ret| Ok(ret)))
    }

    async fn timeout<F, O>(duration: Duration, future: F) -> Result<O, SmolTimeoutError>
    where
        F: Future<Output = O> + Send,
        O: Send,
    {
        let either = futures_util::future::select(
            Box::pin(future),
            Box::pin(async move {
                let instant = Timer::after(duration).await;
                SmolTimeoutError { instant }
            }),
        )
        .await;

        match either {
            Either::Left((output, _)) => Ok(output),
            Either::Right((err, _)) => Err(err),
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

#[derive(Debug)]
pub struct SmolRuntimeProcess {
    child: Child,
    stdout: Option<ChildStdout>,
    stderr: Option<ChildStderr>,
    stdin: Option<ChildStdin>,
}

impl RuntimeProcess for SmolRuntimeProcess {
    type Stdout = ChildStdout;

    type Stderr = ChildStderr;

    type Stdin = ChildStdin;

    fn spawn(command: std::process::Command) -> Result<Self, std::io::Error> {
        let mut child = async_process::Command::from(command).spawn()?;
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let stdin = child.stdin.take();

        Ok(Self {
            child,
            stdout,
            stderr,
            stdin,
        })
    }

    fn output(
        command: std::process::Command,
    ) -> impl Future<Output = Result<std::process::Output, std::io::Error>> + Send {
        async_process::Command::from(command).output()
    }

    fn try_wait(&mut self) -> Result<Option<std::process::ExitStatus>, std::io::Error> {
        self.child.try_status()
    }

    fn wait(&mut self) -> impl Future<Output = Result<std::process::ExitStatus, std::io::Error>> + Send {
        self.child.status()
    }

    fn kill(&mut self) -> Result<(), std::io::Error> {
        self.child.kill()
    }

    fn stdout(&mut self) -> &mut Option<Self::Stdout> {
        &mut self.stdout
    }

    fn stderr(&mut self) -> &mut Option<Self::Stderr> {
        &mut self.stderr
    }

    fn stdin(&mut self) -> &mut Option<Self::Stdin> {
        &mut self.stdin
    }

    fn take_stdout(&mut self) -> Option<Self::Stdout> {
        self.stdout.take()
    }

    fn take_stderr(&mut self) -> Option<Self::Stderr> {
        self.stderr.take()
    }

    fn take_stdin(&mut self) -> Option<Self::Stdin> {
        self.stdin.take()
    }
}
