//! A runtime implementation using Tokio's different features for all of its components.

use std::{future::Future, os::fd::OwnedFd, path::Path, process::Stdio, time::Duration};

use tokio::{
    io::unix::AsyncFd,
    process::{Child, ChildStderr, ChildStdin, ChildStdout},
    task::JoinHandle,
};
use tokio_util::compat::{Compat, TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};

use super::{chownr::chownr_recursive, Runtime, RuntimeAsyncFd, RuntimeChild, RuntimeTask};

#[derive(Clone)]
pub struct TokioRuntime;

impl Runtime for TokioRuntime {
    type Task<O: Send + 'static> = TokioRuntimeTask<O>;
    type TimeoutError = tokio::time::error::Elapsed;
    type File = Compat<tokio::fs::File>;
    type AsyncFd = TokioRuntimeAsyncFd;
    type Child = TokioRuntimeChild;

    #[cfg(feature = "vmm-process")]
    type HyperExecutor = hyper_util::rt::TokioExecutor;

    #[cfg(feature = "vmm-process")]
    fn get_hyper_executor(&self) -> Self::HyperExecutor {
        hyper_util::rt::TokioExecutor::new()
    }

    #[cfg(feature = "vmm-process")]
    fn get_hyper_client_sockets_backend(&self) -> hyper_client_sockets::Backend {
        hyper_client_sockets::Backend::Tokio
    }

    fn spawn_task<F, O>(&self, future: F) -> Self::Task<O>
    where
        F: Future<Output = O> + Send + 'static,
        O: Send + 'static,
    {
        TokioRuntimeTask(tokio::task::spawn(future))
    }

    fn timeout<F, O>(&self, duration: Duration, future: F) -> impl Future<Output = Result<O, Self::TimeoutError>> + Send
    where
        F: Future<Output = O> + Send,
        O: Send,
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
        match tokio::task::spawn_blocking(move || chownr_recursive(&path, uid, gid)).await {
            Ok(result) => result,
            Err(_) => Err(std::io::Error::other("chownr_impl blocking task panicked")),
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

    fn spawn_child(
        &self,
        command: std::process::Command,
        stdout: Stdio,
        stderr: Stdio,
        stdin: Stdio,
    ) -> Result<Self::Child, std::io::Error> {
        let mut child = tokio::process::Command::from(command)
            .stdout(stdout)
            .stderr(stderr)
            .stdin(stdin)
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

    fn run_child(
        &self,
        command: std::process::Command,
    ) -> impl Future<Output = Result<std::process::Output, std::io::Error>> + Send {
        tokio::process::Command::from(command).output()
    }
}

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
}

pub struct TokioRuntimeAsyncFd(AsyncFd<OwnedFd>);

impl RuntimeAsyncFd for TokioRuntimeAsyncFd {
    async fn readable(&self) -> Result<(), std::io::Error> {
        let mut guard = self.0.readable().await?;
        guard.retain_ready();
        Ok(())
    }
}

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

    fn stdout(&mut self) -> &mut Option<Self::Stdout> {
        &mut self.stdout
    }

    fn stdin(&mut self) -> &mut Option<Self::Stdin> {
        &mut self.stdin
    }

    fn stderr(&mut self) -> &mut Option<Self::Stderr> {
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
