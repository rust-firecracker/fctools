//! A runtime implementation using modular async-* crates, which are part of the Smol ecosystem,
//! for its components.

use std::{
    ffi::{OsStr, OsString},
    future::Future,
    os::unix::prelude::OwnedFd,
    path::Path,
    pin::Pin,
    process::{ExitStatus, Stdio},
    sync::Arc,
    task::{Context, Poll},
    time::{Duration, Instant},
};

use async_io::Timer;
use async_process::{Child, ChildStderr, ChildStdin, ChildStdout};
use pin_project_lite::pin_project;

use crate::runtime::util::get_stdio_from_piped;

use super::{Runtime, RuntimeAsyncFd, RuntimeChild, RuntimeTask, util::chown_all_blocking};

#[derive(Clone)]
enum MaybeStaticExecutor {
    NonStatic(Arc<async_executor::Executor<'static>>),
    Static(&'static async_executor::StaticExecutor),
}

/// The [Runtime] implementation backed by the "async-*" family of crates.
#[derive(Clone)]
pub struct SmolRuntime(MaybeStaticExecutor);

impl SmolRuntime {
    /// Create a [SmolRuntime] from a potentially [Arc]ed statically lifetimed [async_executor::Executor],
    /// not taking advantage of [async_executor::StaticExecutor] optimizations.
    pub fn with_executor<E: Into<Arc<async_executor::Executor<'static>>>>(executor: E) -> Self {
        Self(MaybeStaticExecutor::NonStatic(executor.into()))
    }

    /// Create a [SmolRuntime] from a static reference to an optimized [async_executor::StaticExecutor].
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

    fn spawn_task<F>(&self, future: F) -> Self::Task<F::Output>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        let task = match self.0 {
            MaybeStaticExecutor::NonStatic(ref executor) => executor.spawn(future),
            MaybeStaticExecutor::Static(ref executor) => executor.spawn(future),
        };

        SmolRuntimeTask(Some(task))
    }

    fn timeout<F>(
        &self,
        duration: Duration,
        future: F,
    ) -> impl Future<Output = Result<F::Output, Self::TimeoutError>> + Send
    where
        F: Future + Send,
        F::Output: Send,
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

    fn spawn_process(
        &self,
        program: &OsStr,
        args: &[OsString],
        stdout: bool,
        stderr: bool,
        stdin: bool,
    ) -> Result<Self::Child, std::io::Error> {
        let mut command = async_process::Command::new(program);
        command
            .args(args)
            .stdout(get_stdio_from_piped(stdout))
            .stderr(get_stdio_from_piped(stderr))
            .stdin(get_stdio_from_piped(stdin));

        Ok(SmolRuntimeChild(command.spawn()?))
    }

    fn run_process(
        &self,
        program: &OsStr,
        args: &[OsString],
        stdout: bool,
        stderr: bool,
    ) -> impl Future<Output = Result<std::process::Output, std::io::Error>> + Send {
        async_process::Command::new(program)
            .args(args)
            .stdout(get_stdio_from_piped(stdout))
            .stderr(get_stdio_from_piped(stderr))
            .stdin(Stdio::null())
            .output()
    }
}

/// The [RuntimeTask] implementation for the [SmolRuntime].
pub struct SmolRuntimeTask<O: Send + 'static>(Option<async_task::Task<O>>);

impl<O: Send + 'static> RuntimeTask<O> for SmolRuntimeTask<O> {
    fn cancel(mut self) -> impl Future<Output = Option<O>> {
        self.0.take().expect("Task option can't be None before drop").cancel()
    }

    fn poll_join(&mut self, context: &mut Context) -> Poll<Option<O>> {
        match self.0 {
            Some(ref mut task) => Pin::new(task).poll(context).map(Some),
            _ => Poll::Ready(None),
        }
    }
}

impl<O: Send + 'static> Drop for SmolRuntimeTask<O> {
    fn drop(&mut self) {
        if let Some(task) = self.0.take() {
            task.detach();
        }
    }
}

/// The timeout error yielded by the [SmolRuntime].
#[derive(Debug)]
pub struct TimeoutError {
    /// The [Instant] at which the timeout occurred.
    pub instant: Instant,
}

impl std::error::Error for TimeoutError {}

impl std::fmt::Display for TimeoutError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "A future timed out at instant: {:?}", self.instant)
    }
}

pin_project! {
    struct TimeoutFuture<F: Future> {
        #[pin]
        timer: Timer,
        #[pin]
        future: F
    }
}

impl<F: Future> Future for TimeoutFuture<F> {
    type Output = Result<F::Output, TimeoutError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
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

/// The [RuntimeAsyncFd] implementation for the [SmolRuntime].
pub struct SmolRuntimeAsyncFd(async_io::Async<OwnedFd>);

impl RuntimeAsyncFd for SmolRuntimeAsyncFd {
    fn readable(&self) -> impl Future<Output = Result<(), std::io::Error>> + Send {
        self.0.readable()
    }
}

/// The [RuntimeChild] implementation for the [SmolRuntime].
#[derive(Debug)]
pub struct SmolRuntimeChild(Child);

impl RuntimeChild for SmolRuntimeChild {
    type Stdout = ChildStdout;

    type Stderr = ChildStderr;

    type Stdin = ChildStdin;

    fn try_wait(&mut self) -> Result<Option<ExitStatus>, std::io::Error> {
        self.0.try_status()
    }

    fn wait(&mut self) -> impl Future<Output = Result<ExitStatus, std::io::Error>> + Send {
        self.0.status()
    }

    fn kill(&mut self) -> Result<(), std::io::Error> {
        self.0.kill()
    }

    fn get_stdout(&mut self) -> &mut Option<Self::Stdout> {
        &mut self.0.stdout
    }

    fn get_stderr(&mut self) -> &mut Option<Self::Stderr> {
        &mut self.0.stderr
    }

    fn get_stdin(&mut self) -> &mut Option<Self::Stdin> {
        &mut self.0.stdin
    }

    fn take_stdout(&mut self) -> Option<Self::Stdout> {
        self.0.stdout.take()
    }

    fn take_stderr(&mut self) -> Option<Self::Stderr> {
        self.0.stderr.take()
    }

    fn take_stdin(&mut self) -> Option<Self::Stdin> {
        self.0.stdin.take()
    }
}
