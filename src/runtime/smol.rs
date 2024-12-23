//! A runtime implementation using modular async-* crates, which are part of the Smol ecosystem,
//! for its components.

use std::{
    future::Future,
    os::unix::prelude::OwnedFd,
    path::Path,
    pin::Pin,
    process::{ExitStatus, Stdio},
    sync::Arc,
    task::Poll,
    time::{Duration, Instant},
};

use async_io::Timer;
use async_process::{Child, ChildStderr, ChildStdin, ChildStdout};
use pin_project_lite::pin_project;

use super::{util::chown_all_blocking, Runtime, RuntimeAsyncFd, RuntimeChild, RuntimeTask};

#[derive(Clone)]
enum MaybeStaticExecutor {
    NonStatic(Arc<async_executor::Executor<'static>>),
    Static(&'static async_executor::StaticExecutor),
}

#[derive(Clone)]
pub struct SmolRuntime(MaybeStaticExecutor);

impl SmolRuntime {
    pub fn with_executor(executor: impl Into<Arc<async_executor::Executor<'static>>>) -> Self {
        Self(MaybeStaticExecutor::NonStatic(executor.into()))
    }

    pub fn with_static_executor(executor: &'static async_executor::StaticExecutor) -> Self {
        Self(MaybeStaticExecutor::Static(executor))
    }
}

impl Runtime for SmolRuntime {
    type Task<O: Send + 'static> = SmolRuntimeTask<O>;
    type TimeoutError = TimeoutError;
    type File = async_fs::File;
    type AsyncFd = SmolRuntimeAsyncFd;
    type Child = SmolRuntimeChild;

    #[cfg(feature = "vmm-process")]
    #[cfg_attr(docsrs, doc(cfg(feature = "vmm-process")))]
    type SocketBackend = hyper_client_sockets::async_io::AsyncIoBackend;

    fn spawn_task<F, O>(&self, future: F) -> Self::Task<O>
    where
        F: Future<Output = O> + Send + 'static,
        O: Send + 'static,
    {
        let task = match self.0 {
            MaybeStaticExecutor::NonStatic(ref executor) => executor.spawn(future),
            MaybeStaticExecutor::Static(ref executor) => executor.spawn(future),
        };

        SmolRuntimeTask(Some(task))
    }

    fn timeout<F, O>(&self, duration: Duration, future: F) -> impl Future<Output = Result<O, Self::TimeoutError>> + Send
    where
        F: Future<Output = O> + Send,
        O: Send,
    {
        TimeoutFuture {
            timer: Timer::after(duration),
            future,
        }
    }

    fn fs_exists(&self, path: &Path) -> impl Future<Output = Result<bool, std::io::Error>> + Send {
        let path = path.to_owned();
        blocking::unblock(move || std::fs::exists(&path))
    }

    fn fs_remove_file(&self, path: &Path) -> impl Future<Output = Result<(), std::io::Error>> + Send {
        async_fs::remove_file(path)
    }

    fn fs_create_dir_all(&self, path: &Path) -> impl Future<Output = Result<(), std::io::Error>> + Send {
        async_fs::create_dir_all(path)
    }

    async fn fs_create_file(&self, path: &Path) -> Result<(), std::io::Error> {
        async_fs::File::create(path).await.map(|_| ())
    }

    fn fs_write(&self, path: &Path, content: String) -> impl Future<Output = Result<(), std::io::Error>> + Send {
        async_fs::write(path, content)
    }

    fn fs_read_to_string(&self, path: &Path) -> impl Future<Output = Result<String, std::io::Error>> + Send {
        async_fs::read_to_string(path)
    }

    fn fs_rename(
        &self,
        source_path: &Path,
        destination_path: &Path,
    ) -> impl Future<Output = Result<(), std::io::Error>> + Send {
        async_fs::rename(source_path, destination_path)
    }

    fn fs_remove_dir_all(&self, path: &Path) -> impl Future<Output = Result<(), std::io::Error>> + Send {
        async_fs::remove_dir_all(path)
    }

    async fn fs_copy(&self, source_path: &Path, destination_path: &Path) -> Result<(), std::io::Error> {
        async_fs::copy(source_path, destination_path).await.map(|_| ())
    }

    fn fs_chown_all(&self, path: &Path, uid: u32, gid: u32) -> impl Future<Output = Result<(), std::io::Error>> + Send {
        let path = path.to_owned();
        blocking::unblock(move || chown_all_blocking(&path, uid, gid))
    }

    fn fs_hard_link(
        &self,
        source_path: &Path,
        destination_path: &Path,
    ) -> impl Future<Output = Result<(), std::io::Error>> + Send {
        async_fs::hard_link(source_path, destination_path)
    }

    fn fs_open_file_for_read(&self, path: &Path) -> impl Future<Output = Result<Self::File, std::io::Error>> + Send {
        let mut open_options = async_fs::OpenOptions::new();
        open_options.read(true);
        open_options.open(path)
    }

    fn create_async_fd(&self, fd: OwnedFd) -> Result<Self::AsyncFd, std::io::Error> {
        Ok(SmolRuntimeAsyncFd(async_io::Async::new(fd)?))
    }

    fn spawn_child(
        &self,
        command: std::process::Command,
        stdout: Stdio,
        stderr: Stdio,
        stdin: Stdio,
    ) -> Result<Self::Child, std::io::Error> {
        let mut child = async_process::Command::from(command)
            .stdout(stdout)
            .stderr(stderr)
            .stdin(stdin)
            .spawn()?;
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let stdin = child.stdin.take();

        Ok(SmolRuntimeChild {
            child,
            stdout,
            stderr,
            stdin,
        })
    }

    fn run_child(
        &self,
        command: std::process::Command,
    ) -> impl Future<Output = Result<std::process::Output, std::io::Error>> + Send {
        async_process::Command::from(command).output()
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

#[derive(Debug)]
pub struct TimeoutError {
    pub instant: Instant,
}

impl std::error::Error for TimeoutError {}

impl std::fmt::Display for TimeoutError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "A future timed out at instant: {:?}", self.instant)
    }
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

pub struct SmolRuntimeAsyncFd(async_io::Async<OwnedFd>);

impl RuntimeAsyncFd for SmolRuntimeAsyncFd {
    fn readable(&self) -> impl Future<Output = Result<(), std::io::Error>> + Send {
        self.0.readable()
    }
}

#[derive(Debug)]
pub struct SmolRuntimeChild {
    child: Child,
    stdout: Option<ChildStdout>,
    stderr: Option<ChildStderr>,
    stdin: Option<ChildStdin>,
}

impl RuntimeChild for SmolRuntimeChild {
    type Stdout = ChildStdout;

    type Stderr = ChildStderr;

    type Stdin = ChildStdin;

    fn try_wait(&mut self) -> Result<Option<ExitStatus>, std::io::Error> {
        self.child.try_status()
    }

    fn wait(&mut self) -> impl Future<Output = Result<ExitStatus, std::io::Error>> + Send {
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
