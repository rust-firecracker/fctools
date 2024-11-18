use std::{future::Future, path::Path};

use nix::unistd::{Gid, Uid};
use tokio::process::{Child, ChildStderr, ChildStdin, ChildStdout};
use tokio_util::compat::{Compat, TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};

use super::{Runtime, RuntimeExecutor, RuntimeFilesystem, RuntimeProcess};

pub struct TokioRuntime;

impl Runtime for TokioRuntime {
    type Executor = TokioRuntimeExecutor;
    type Filesystem = TokioRuntimeFilesystem;
    type Process = TokioProcess;
}

pub struct TokioRuntimeExecutor;

impl RuntimeExecutor for TokioRuntimeExecutor {
    type JoinError = tokio::task::JoinError;

    type JoinHandle<O> = tokio::task::JoinHandle<O>;

    fn spawn<F, O>(future: F) -> Self::JoinHandle<O>
    where
        F: Future<Output = O> + Send + 'static,
        O: Send + 'static,
    {
        tokio::spawn(future)
    }

    fn spawn_blocking<F, O>(function: F) -> Self::JoinHandle<O>
    where
        F: FnOnce() -> O + Send + 'static,
        O: Send + 'static,
    {
        tokio::task::spawn_blocking(function)
    }
}

pub struct TokioRuntimeFilesystem;

impl RuntimeFilesystem for TokioRuntimeFilesystem {
    fn check_exists(path: &Path) -> impl Future<Output = Result<bool, std::io::Error>> + Send {
        tokio::fs::try_exists(path)
    }

    fn remove_file(path: &Path) -> impl Future<Output = Result<(), std::io::Error>> + Send {
        tokio::fs::remove_file(path)
    }

    fn create_dir_all(path: &Path) -> impl Future<Output = Result<(), std::io::Error>> + Send {
        tokio::fs::create_dir_all(path)
    }

    async fn create_file(path: &Path) -> Result<(), std::io::Error> {
        tokio::fs::File::create(path).await.map(|_| ())
    }

    fn write_file(path: &Path, content: String) -> impl Future<Output = Result<(), std::io::Error>> + Send {
        tokio::fs::write(path, content)
    }

    fn read_to_string(path: &Path) -> impl Future<Output = Result<String, std::io::Error>> + Send {
        tokio::fs::read_to_string(path)
    }

    fn rename_file(
        &self,
        source_path: &Path,
        destination_path: &Path,
    ) -> impl Future<Output = Result<(), std::io::Error>> + Send {
        tokio::fs::rename(source_path, destination_path)
    }

    fn remove_dir_all(path: &Path) -> impl Future<Output = Result<(), std::io::Error>> + Send {
        tokio::fs::remove_dir_all(path)
    }

    async fn copy(source_path: &Path, destination_path: &Path) -> Result<(), std::io::Error> {
        tokio::fs::copy(source_path, destination_path).await.map(|_| ())
    }

    async fn chownr(path: &Path, uid: Uid, gid: Gid) -> Result<(), std::io::Error> {
        let path = path.to_owned();
        match tokio::task::spawn_blocking(move || chownr_impl(&path, uid, gid)).await {
            Ok(result) => result,
            Err(_) => Err(std::io::Error::other("chownr_impl blocking task panicked")),
        }
    }

    fn hard_link(
        &self,
        source_path: &Path,
        destination_path: &Path,
    ) -> impl Future<Output = Result<(), std::io::Error>> + Send {
        tokio::fs::hard_link(source_path, destination_path)
    }
}

fn chownr_impl(path: &Path, uid: Uid, gid: Gid) -> Result<(), std::io::Error> {
    if path.is_dir() {
        for entry in std::fs::read_dir(path)? {
            let entry = entry?;
            chownr_impl(entry.path().as_path(), uid, gid)?;
        }
    }

    if nix::unistd::chown(path, Some(uid), Some(gid)).is_err() {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

pub struct TokioProcess(Child);

impl RuntimeProcess for TokioProcess {
    type Stdout = Compat<ChildStdout>;

    type Stderr = Compat<ChildStderr>;

    type Stdin = Compat<ChildStdin>;

    fn spawn(command: std::process::Command) -> Result<Self, std::io::Error> {
        tokio::process::Command::from(command).spawn().map(|child| Self(child))
    }

    fn try_wait(&mut self) -> Result<Option<std::process::ExitStatus>, std::io::Error> {
        self.0.try_wait()
    }

    fn wait(&mut self) -> impl Future<Output = Result<std::process::ExitStatus, std::io::Error>> {
        self.0.wait()
    }

    fn kill(&mut self) -> Result<(), std::io::Error> {
        self.0.start_kill()
    }

    fn take_stdout(&mut self) -> Option<Self::Stdout> {
        self.0.stdout.take().map(|stdout| stdout.compat())
    }

    fn take_stderr(&mut self) -> Option<Self::Stderr> {
        self.0.stderr.take().map(|stderr| stderr.compat())
    }

    fn take_stdin(&mut self) -> Option<Self::Stdin> {
        self.0.stdin.take().map(|stdin| stdin.compat_write())
    }
}
