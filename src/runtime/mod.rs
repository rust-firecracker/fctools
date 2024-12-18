//! Provides a set of traits that need to be implemented to have compatibility between fctools and an async runtime.
//! Two built-in implementations are provided behind feature gates that are both disabled by default:
//! - `tokio-runtime` using Tokio.
//! - `smol-runtime` using the async-* crates (async-io, async-fs, async-process, async-task, async-executor).
//!
//! Extra utilities that are used internally by certain layers of fctools and which are helpful for third-party runtime
//! implementors are available via the optional `runtime-util` feature.

use std::{
    future::Future,
    os::fd::OwnedFd,
    path::Path,
    process::{ExitStatus, Stdio},
    time::Duration,
};

use futures_io::{AsyncRead, AsyncWrite};

#[cfg(feature = "tokio-runtime")]
#[cfg_attr(docsrs, doc(cfg(feature = "tokio-runtime")))]
pub mod tokio;

#[cfg(feature = "smol-runtime")]
#[cfg_attr(docsrs, doc(cfg(feature = "smol-runtime")))]
pub mod smol;

#[cfg(feature = "runtime-util")]
#[cfg_attr(docsrs, doc(cfg(feature = "runtime-util")))]
pub mod util;

/// An async runtime platform used by fctools. Instances of a [Runtime] are highly frequently cloned by fctools,
/// so the [Clone] implementation is expected to be cheap and fast, meaning that the underlying structure of a [Runtime]
/// implementation should either be a ZST or an [Arc](std::sync::Arc) of an inner shared type.
pub trait Runtime: Clone + Send + Sync + 'static {
    type Task<O: Send + 'static>: RuntimeTask<O>;
    type TimeoutError: std::error::Error + std::fmt::Debug + Send + Sync;
    type File: AsyncRead + AsyncWrite + Send + Unpin;
    type AsyncFd: RuntimeAsyncFd;
    type Child: RuntimeChild;

    #[cfg(feature = "vmm-process")]
    #[cfg_attr(docsrs, doc(cfg(feature = "vmm-process")))]
    fn get_hyper_client_sockets_backend(&self) -> hyper_client_sockets::Backend;

    fn spawn_task<F, O>(&self, future: F) -> Self::Task<O>
    where
        F: Future<Output = O> + Send + 'static,
        O: Send + 'static;

    fn timeout<F, O>(
        &self,
        duration: Duration,
        future: F,
    ) -> impl Future<Output = Result<O, Self::TimeoutError>> + Send
    where
        F: Future<Output = O> + Send,
        O: Send;

    fn fs_exists(&self, path: &Path) -> impl Future<Output = Result<bool, std::io::Error>> + Send;

    fn fs_remove_file(&self, path: &Path) -> impl Future<Output = Result<(), std::io::Error>> + Send;

    fn fs_create_dir_all(&self, path: &Path) -> impl Future<Output = Result<(), std::io::Error>> + Send;

    fn fs_create_file(&self, path: &Path) -> impl Future<Output = Result<(), std::io::Error>> + Send;

    fn fs_write(&self, path: &Path, content: String) -> impl Future<Output = Result<(), std::io::Error>> + Send;

    fn fs_read_to_string(&self, path: &Path) -> impl Future<Output = Result<String, std::io::Error>> + Send;

    fn fs_rename(
        &self,
        source_path: &Path,
        destination_path: &Path,
    ) -> impl Future<Output = Result<(), std::io::Error>> + Send;

    fn fs_remove_dir_all(&self, path: &Path) -> impl Future<Output = Result<(), std::io::Error>> + Send;

    fn fs_copy(
        &self,
        source_path: &Path,
        destination_path: &Path,
    ) -> impl Future<Output = Result<(), std::io::Error>> + Send;

    fn fs_chown_all(&self, path: &Path, uid: u32, gid: u32) -> impl Future<Output = Result<(), std::io::Error>> + Send;

    fn fs_hard_link(
        &self,
        source_path: &Path,
        destination_path: &Path,
    ) -> impl Future<Output = Result<(), std::io::Error>> + Send;

    fn fs_open_file_for_read(&self, path: &Path) -> impl Future<Output = Result<Self::File, std::io::Error>> + Send;

    fn create_async_fd(&self, fd: OwnedFd) -> Result<Self::AsyncFd, std::io::Error>;

    fn spawn_child(
        &self,
        command: std::process::Command,
        stdout: Stdio,
        stderr: Stdio,
        stdin: Stdio,
    ) -> Result<Self::Child, std::io::Error>;

    fn run_child(
        &self,
        command: std::process::Command,
    ) -> impl Future<Output = Result<std::process::Output, std::io::Error>> + Send;
}

/// An async task that is detached on drop, can be cancelled and joined on.
pub trait RuntimeTask<O: Send + 'static>: Send {
    fn cancel(self) -> impl Future<Output = Option<O>> + Send;

    fn join(self) -> impl Future<Output = Option<O>> + Send;
}

/// An async file descriptor in the runtime that can be polled for the "readable" interest. Used by
/// the detached (pidfd) backend in process handles.
pub trait RuntimeAsyncFd: Send {
    fn readable(&self) -> impl Future<Output = Result<(), std::io::Error>> + Send;
}

/// An async child process in the runtime. Used by the attached backend in process handles.
pub trait RuntimeChild: Sized + Send + Sync + std::fmt::Debug {
    type Stdout: AsyncRead + Unpin + Send;
    type Stderr: AsyncRead + Unpin + Send;
    type Stdin: AsyncWrite + Unpin + Send;

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
