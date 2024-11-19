//! A runtime implementation using modular async-* crates, which are part of the Smol ecosystem,
//! for its components.

use std::{
    future::Future,
    os::unix::prelude::OwnedFd,
    path::Path,
    pin::Pin,
    process::Stdio,
    sync::{Arc, OnceLock},
    task::Poll,
    time::{Duration, Instant},
};

use async_executor::Executor;
use async_io::Timer;
use async_process::{Child, ChildStderr, ChildStdin, ChildStdout};
use nix::unistd::{Gid, Uid};
use pin_project_lite::pin_project;
use smol_hyper::rt::SmolExecutor;

use super::{
    chownr::chownr_recursive, Runtime, RuntimeAsyncFd, RuntimeExecutor, RuntimeFilesystem, RuntimeProcess, RuntimeTask,
};

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

impl std::fmt::Display for TimeoutError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Timed out at {:?}", self.instant)
    }
}

impl RuntimeExecutor for SmolRuntimeExecutor {
    type Task<O: Send + 'static> = SmolRuntimeTask<O>;

    type TimeoutError = TimeoutError;

    fn spawn<F, O>(future: F) -> Self::Task<O>
    where
        F: Future<Output = O> + Send + 'static,
        O: Send + 'static,
    {
        let task = EXECUTOR.get().expect("Executor not initialized").spawn(future);
        SmolRuntimeTask(Some(task))
    }

    fn timeout<F, O>(duration: Duration, future: F) -> impl Future<Output = Result<O, Self::TimeoutError>> + Send
    where
        F: Future<Output = O> + Send,
        O: Send,
    {
        TimeoutFuture {
            timer: Timer::after(duration),
            future,
        }
    }
}

pub struct SmolRuntimeTask<O: Send + 'static>(Option<async_task::Task<O>>);

impl<O: Send + 'static> RuntimeTask<O> for SmolRuntimeTask<O> {
    fn cancel(mut self) -> impl Future<Output = Option<O>> {
        self.0.take().expect("Task option can't be None before drop").cancel()
    }

    async fn join(mut self) -> Option<O> {
        Some(self.0.take().expect("Task option can't be None before drop").await)
    }
}

impl<O: Send + 'static> Drop for SmolRuntimeTask<O> {
    fn drop(&mut self) {
        if let Some(task) = self.0.take() {
            task.detach();
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub struct TimeoutError {
    pub instant: Instant,
}

pin_project! {
    struct TimeoutFuture<F: Future<Output = O>, O> {
        #[pin]
        timer: Timer,
        #[pin]
        future: F
    }
}

impl<F: Future<Output = O>, O> Future for TimeoutFuture<F, O> {
    type Output = Result<O, TimeoutError>;

    fn poll(self: Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> Poll<Self::Output> {
        let proj = self.project();

        if let Poll::Ready(instant) = proj.timer.poll(cx) {
            return Poll::Ready(Err(TimeoutError { instant }));
        }

        if let Poll::Ready(output) = proj.future.poll(cx) {
            return Poll::Ready(Ok(output));
        }

        Poll::Pending
    }
}

pub struct SmolRuntimeFilesystem;

impl RuntimeFilesystem for SmolRuntimeFilesystem {
    type File = async_fs::File;

    type AsyncFd = SmolRuntimeAsyncFd;

    fn check_exists(path: &Path) -> impl Future<Output = Result<bool, std::io::Error>> + Send {
        let path = path.to_owned();
        blocking::unblock(move || std::fs::exists(&path))
    }

    fn remove_file(path: &Path) -> impl Future<Output = Result<(), std::io::Error>> + Send {
        async_fs::remove_file(path)
    }

    fn create_dir_all(path: &Path) -> impl Future<Output = Result<(), std::io::Error>> + Send {
        async_fs::create_dir_all(path)
    }

    async fn create_file(path: &Path) -> Result<(), std::io::Error> {
        async_fs::File::create(path).await.map(|_| ())
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

    async fn copy(source_path: &Path, destination_path: &Path) -> Result<(), std::io::Error> {
        async_fs::copy(source_path, destination_path).await.map(|_| ())
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

    fn spawn(
        command: std::process::Command,
        stdout: Stdio,
        stderr: Stdio,
        stdin: Stdio,
    ) -> Result<Self, std::io::Error> {
        let mut child = async_process::Command::from(command)
            .stdout(stdout)
            .stderr(stderr)
            .stdin(stdin)
            .spawn()?;
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
