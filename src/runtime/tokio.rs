use std::{future::Future, path::Path, time::Duration};

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

    type JoinHandle<O: Send> = tokio::task::JoinHandle<O>;

    type TimeoutError = tokio::time::error::Elapsed;

    fn spawn<F, O>(future: F) -> Self::JoinHandle<O>
    where
        F: Future<Output = O> + Send + 'static,
        O: Send + 'static,
    {
        tokio::spawn(future)
    }

    async fn timeout<F, O>(duration: Duration, future: F) -> Result<O, ()>
    where
        F: Future<Output = O> + Send,
        O: Send,
    {
        tokio::time::timeout(duration, future).await.map_err(|_| ())
    }
}

pub struct TokioRuntimeFilesystem;

impl RuntimeFilesystem for TokioRuntimeFilesystem {
    type File = Compat<tokio::fs::File>;

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
        source_path: &Path,
        destination_path: &Path,
    ) -> impl Future<Output = Result<(), std::io::Error>> + Send {
        tokio::fs::hard_link(source_path, destination_path)
    }

    async fn open_file(path: &Path, open_options: std::fs::OpenOptions) -> Result<Self::File, std::io::Error> {
        let open_options = tokio::fs::OpenOptions::from(open_options);
        let file = open_options.open(path).await?;
        Ok(file.compat())
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

#[derive(Debug)]
pub struct TokioProcess {
    child: Child,
    stdout: Option<Compat<ChildStdout>>,
    stdin: Option<Compat<ChildStdin>>,
    stderr: Option<Compat<ChildStderr>>,
}

impl RuntimeProcess for TokioProcess {
    type Stdout = Compat<ChildStdout>;

    type Stderr = Compat<ChildStderr>;

    type Stdin = Compat<ChildStdin>;

    fn spawn(command: std::process::Command) -> Result<Self, std::io::Error> {
        tokio::process::Command::from(command).spawn().map(|mut child| {
            let stdout = child.stdout.take().map(|stdout| stdout.compat());
            let stderr = child.stderr.take().map(|stderr| stderr.compat());
            let stdin = child.stdin.take().map(|stdin| stdin.compat_write());
            Self {
                child,
                stdout,
                stdin,
                stderr,
            }
        })
    }

    fn try_wait(&mut self) -> Result<Option<std::process::ExitStatus>, std::io::Error> {
        self.child.try_wait()
    }

    fn wait(&mut self) -> impl Future<Output = Result<std::process::ExitStatus, std::io::Error>> {
        self.child.wait()
    }

    fn wait_with_output(self) -> impl Future<Output = Result<std::process::Output, std::io::Error>> + Send {
        self.child.wait_with_output()
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
