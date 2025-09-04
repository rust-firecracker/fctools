//! A runtime implementation using Tokio's different features for all of its components.

use std::{
    ffi::{OsStr, OsString},
    future::Future,
    os::fd::OwnedFd,
    path::Path,
    pin::Pin,
    process::{Output, Stdio},
    task::{Context, Poll},
    time::Duration,
};

use tokio::{
    io::unix::AsyncFd,
    process::{Child, ChildStderr, ChildStdin, ChildStdout},
    task::JoinHandle,
};
use tokio_util::compat::{Compat, TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};

use super::{
    Runtime, RuntimeAsyncFd, RuntimeChild, RuntimeTask,
    util::{chown_all_blocking, get_stdio_from_piped},
};

/// The [Runtime] implementation backed by the [tokio] crate. Since [tokio] heavily utilizes thread-local
/// storage, this struct is zero-sized and doesn't store anything.
#[derive(Clone)]
pub struct TokioRuntime;

impl Runtime for TokioRuntime {
    type Task<O: Send + 'static> = TokioRuntimeTask<O>;
    type TimeoutError = tokio::time::error::Elapsed;
    type File = Compat<tokio::fs::File>;
    type AsyncFd = TokioRuntimeAsyncFd;
    type Child = TokioRuntimeChild;

    #[cfg(feature = "vmm-process")]
    #[cfg_attr(docsrs, doc(cfg(feature = "vmm-process")))]
    type SocketBackend = hyper_client_sockets::tokio::TokioBackend;

    fn spawn_task<F>(&self, future: F) -> Self::Task<F::Output>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        TokioRuntimeTask(tokio::task::spawn(future))
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
        tokio::time::timeout(duration, future)
    }

    fn fs_exists(&self, path: &Path) -> impl Future<Output = Result<bool, std::io::Error>> + Send {
        tokio::fs::try_exists(path)
    }

    fn fs_remove_file(&self, path: &Path) -> impl Future<Output = Result<(), std::io::Error>> + Send {
        tokio::fs::remove_file(path)
    }

    fn fs_create_dir_all(&self, path: &Path) -> impl Future<Output = Result<(), std::io::Error>> + Send {
        tokio::fs::create_dir_all(path)
    }

    async fn fs_create_file(&self, path: &Path) -> Result<(), std::io::Error> {
        tokio::fs::File::create(path).await.map(|_| ())
    }

    fn fs_write(&self, path: &Path, content: String) -> impl Future<Output = Result<(), std::io::Error>> + Send {
        tokio::fs::write(path, content)
    }

    fn fs_read_to_string(&self, path: &Path) -> impl Future<Output = Result<String, std::io::Error>> + Send {
        tokio::fs::read_to_string(path)
    }

    fn fs_rename(
        &self,
        source_path: &Path,
        destination_path: &Path,
    ) -> impl Future<Output = Result<(), std::io::Error>> + Send {
        tokio::fs::rename(source_path, destination_path)
    }

    fn fs_remove_dir_all(&self, path: &Path) -> impl Future<Output = Result<(), std::io::Error>> + Send {
        tokio::fs::remove_dir_all(path)
    }

    async fn fs_copy(&self, source_path: &Path, destination_path: &Path) -> Result<(), std::io::Error> {
        tokio::fs::copy(source_path, destination_path).await.map(|_| ())
    }

    async fn fs_chown_all(&self, path: &Path, uid: u32, gid: u32) -> Result<(), std::io::Error> {
        let path = path.to_owned();
        match tokio::task::spawn_blocking(move || chown_all_blocking(&path, uid, gid)).await {
            Ok(result) => result,
            Err(_) => Err(std::io::Error::other("chown_all_blocking blocking task panicked")),
        }
    }

    fn fs_hard_link(
        &self,
        source_path: &Path,
        destination_path: &Path,
    ) -> impl Future<Output = Result<(), std::io::Error>> + Send {
        tokio::fs::hard_link(source_path, destination_path)
    }

    async fn fs_open_file_for_read(&self, path: &Path) -> Result<Self::File, std::io::Error> {
        let mut open_options = tokio::fs::OpenOptions::new();
        open_options.read(true);
        let file = open_options.open(path).await?;
        Ok(file.compat())
    }

    fn create_async_fd(&self, fd: OwnedFd) -> Result<Self::AsyncFd, std::io::Error> {
        Ok(TokioRuntimeAsyncFd(AsyncFd::new(fd)?))
    }

    fn spawn_process(
        &self,
        program: &OsStr,
        args: &[OsString],
        stdout: bool,
        stderr: bool,
        stdin: bool,
    ) -> Result<Self::Child, std::io::Error> {
        let mut child = tokio::process::Command::new(program)
            .args(args)
            .stdout(get_stdio_from_piped(stdout))
            .stderr(get_stdio_from_piped(stderr))
            .stdin(get_stdio_from_piped(stdin))
            .spawn()?;

        let stdout = child.stdout.take().map(|stdout| stdout.compat());
        let stderr = child.stderr.take().map(|stderr| stderr.compat());
        let stdin = child.stdin.take().map(|stdin| stdin.compat_write());

        Ok(TokioRuntimeChild {
            child,
            stdout,
            stdin,
            stderr,
        })
    }

    fn run_process(
        &self,
        program: &OsStr,
        args: &[OsString],
        stdout: bool,
        stderr: bool,
    ) -> impl Future<Output = Result<Output, std::io::Error>> + Send {
        tokio::process::Command::new(program)
            .args(args)
            .stdout(get_stdio_from_piped(stdout))
            .stderr(get_stdio_from_piped(stderr))
            .stdin(Stdio::null())
            .output()
    }
}

/// The [RuntimeTask] implementation for the [TokioRuntime].
pub struct TokioRuntimeTask<O: Send + 'static>(JoinHandle<O>);

impl<O: Send + 'static> RuntimeTask<O> for TokioRuntimeTask<O> {
    fn cancel(self) -> impl Future<Output = Option<O>> {
        self.0.abort();
        self.join()
    }

    async fn join(self) -> Option<O> {
        match self.0.await {
            Ok(output) => Some(output),
            Err(_) => None,
        }
    }

    fn poll_join(&mut self, context: &mut Context) -> Poll<Option<O>> {
        Pin::new(&mut self.0).poll(context).map(|r| r.ok())
    }
}

/// The [RuntimeAsyncFd] implementation for the [TokioRuntime].
pub struct TokioRuntimeAsyncFd(AsyncFd<OwnedFd>);

impl RuntimeAsyncFd for TokioRuntimeAsyncFd {
    async fn readable(&self) -> Result<(), std::io::Error> {
        let mut guard = self.0.readable().await?;
        guard.retain_ready();
        Ok(())
    }
}

/// The [RuntimeChild] implementation for the [TokioRuntime].
#[derive(Debug)]
pub struct TokioRuntimeChild {
    child: Child,
    stdout: Option<Compat<ChildStdout>>,
    stdin: Option<Compat<ChildStdin>>,
    stderr: Option<Compat<ChildStderr>>,
}

impl RuntimeChild for TokioRuntimeChild {
    type Stdout = Compat<ChildStdout>;

    type Stderr = Compat<ChildStderr>;

    type Stdin = Compat<ChildStdin>;

    fn try_wait(&mut self) -> Result<Option<std::process::ExitStatus>, std::io::Error> {
        self.child.try_wait()
    }

    fn wait(&mut self) -> impl Future<Output = Result<std::process::ExitStatus, std::io::Error>> {
        self.child.wait()
    }

    fn kill(&mut self) -> Result<(), std::io::Error> {
        self.child.start_kill()
    }

    fn get_stdout(&mut self) -> &mut Option<Self::Stdout> {
        &mut self.stdout
    }

    fn get_stdin(&mut self) -> &mut Option<Self::Stdin> {
        &mut self.stdin
    }

    fn get_stderr(&mut self) -> &mut Option<Self::Stderr> {
        &mut self.stderr
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
