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
    task::{Context, Poll},
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
    /// The [RuntimeTask] implementation used by this [Runtime].
    type Task<O: Send + 'static>: RuntimeTask<O>;

    /// The error this [Runtime] yields when a future times out in a controlled manner.
    type TimeoutError: std::error::Error + std::fmt::Debug + Send + Sync;

    /// The I/O object representing an opened asynchronously readable file within this [Runtime].
    type File: AsyncRead + Send + Unpin;

    /// The [RuntimeAsyncFd] implementation used by this [Runtime].
    type AsyncFd: RuntimeAsyncFd;

    /// The [RuntimeChild] implementation used by this [Runtime].
    type Child: RuntimeChild;

    /// The [hyper_client_sockets::Backend] of this [Runtime], used by fctools to establish connections over
    /// Unix or Firecracker sockets.
    #[cfg(feature = "vmm-process")]
    #[cfg_attr(docsrs, doc(cfg(feature = "vmm-process")))]
    type SocketBackend: hyper_client_sockets::Backend + Send + Sync + std::fmt::Debug;

    /// Spawn a static [Send] future returning a static [Send] type onto this [Runtime] and return its joinable task.
    fn spawn_task<F>(&self, future: F) -> Self::Task<F::Output>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static;

    /// Await a [Send] future returning a [Send] type, cancelling it after the giving [Duration] and returning a timeout
    /// error.
    fn timeout<F>(
        &self,
        duration: Duration,
        future: F,
    ) -> impl Future<Output = Result<F::Output, Self::TimeoutError>> + Send
    where
        F: Future + Send,
        F::Output: Send;

    /// Check if the given [Path] exists on the filesystem.
    fn fs_exists(&self, path: &Path) -> impl Future<Output = Result<bool, std::io::Error>> + Send;

    /// Remove the given [Path] as a file from the filesystem.
    fn fs_remove_file(&self, path: &Path) -> impl Future<Output = Result<(), std::io::Error>> + Send;

    /// Recursively create a directory tree at the given [Path] on the filesystem.
    fn fs_create_dir_all(&self, path: &Path) -> impl Future<Output = Result<(), std::io::Error>> + Send;

    /// Create a file at the given [Path] on the filesystem.
    fn fs_create_file(&self, path: &Path) -> impl Future<Output = Result<(), std::io::Error>> + Send;

    /// Write the provided [String] blob to the given [Path] on the filesystem.
    fn fs_write(&self, path: &Path, content: String) -> impl Future<Output = Result<(), std::io::Error>> + Send;

    /// Read the contents of the file at the given [Path] on the filesystem to a [String] blob.
    fn fs_read_to_string(&self, path: &Path) -> impl Future<Output = Result<String, std::io::Error>> + Send;

    /// Rename the provided source [Path] to the provided destination [Path] on the filesystem.
    fn fs_rename(
        &self,
        source_path: &Path,
        destination_path: &Path,
    ) -> impl Future<Output = Result<(), std::io::Error>> + Send;

    /// Recursively remove the directory and its contents at the given [Path] on the filesystem.
    fn fs_remove_dir_all(&self, path: &Path) -> impl Future<Output = Result<(), std::io::Error>> + Send;

    /// Copy the file at the source [Path] on the filesystem to the destination [Path].
    fn fs_copy(
        &self,
        source_path: &Path,
        destination_path: &Path,
    ) -> impl Future<Output = Result<(), std::io::Error>> + Send;

    /// Recursively change the ownership of the given [Path] on the filesystem to the given PAM UID and GID.
    fn fs_chown_all(&self, path: &Path, uid: u32, gid: u32) -> impl Future<Output = Result<(), std::io::Error>> + Send;

    /// Hard-link the given source [Path] on the filesystem to the given destination [Path].
    fn fs_hard_link(
        &self,
        source_path: &Path,
        destination_path: &Path,
    ) -> impl Future<Output = Result<(), std::io::Error>> + Send;

    /// Open the file at the given [Path] on the filesystem in read-only mode, returning an I/O object used for
    /// asynchronously reading its contents.
    fn fs_open_file_for_read(&self, path: &Path) -> impl Future<Output = Result<Self::File, std::io::Error>> + Send;

    /// Create an asynchronous file descriptor from the given [OwnedFd], tying it to this [Runtime]'s I/O reactor.
    fn create_async_fd(&self, fd: OwnedFd) -> Result<Self::AsyncFd, std::io::Error>;

    /// Spawn a child process asynchronously on this [Runtime] from a [std::process::Command].
    fn spawn_child(
        &self,
        command: std::process::Command,
        stdout: Stdio,
        stderr: Stdio,
        stdin: Stdio,
    ) -> Result<Self::Child, std::io::Error>;

    /// Run a child process asynchronously on this [Runtime] until completion from a [std::process::Command].
    fn run_child(
        &self,
        command: std::process::Command,
    ) -> impl Future<Output = Result<std::process::Output, std::io::Error>> + Send;
}

/// An async task that is detached on drop, can be cancelled and joined on.
pub trait RuntimeTask<O: Send + 'static>: Send + Sized {
    /// Asynchronously cancel the execution of this task, optionally returning its output.
    fn cancel(self) -> impl Future<Output = Option<O>> + Send;

    /// Poll whether this task can be joined on. If joinable, the task either yields its
    /// output within [Some], or returns [None] due to a join error having occurred.
    fn poll_join(&mut self, context: &mut Context) -> Poll<Option<O>>;

    /// Asynchronously join on this task. The default implementation of this function simply
    /// returns a [std::future::poll_fn] future backed by [RuntimeTask::poll_join].
    fn join(mut self) -> impl Future<Output = Option<O>> + Send {
        std::future::poll_fn(move |cx| self.poll_join(cx))
    }
}

/// An async file descriptor in the runtime that can be polled for the "readable" interest. Used by
/// the detached (pidfd) backend in process handles.
pub trait RuntimeAsyncFd: Send {
    /// Asynchronously wait for this file descriptor to have the "readable" interest, i.e. be readable.
    fn readable(&self) -> impl Future<Output = Result<(), std::io::Error>> + Send;
}

/// An async child process in the runtime. Used by the attached backend in process handles.
pub trait RuntimeChild: Sized + Send + Sync + std::fmt::Debug {
    /// The I/O object representing the asynchronously readable stdout pipe of a child process.
    type Stdout: AsyncRead + Unpin + Send;

    /// The I/O object representing the asynchronously readable stderr pipe of a child process.
    type Stderr: AsyncRead + Unpin + Send;

    /// The I/O object representing the asynchronously writable stdin pipe of a child process.
    type Stdin: AsyncWrite + Unpin + Send;

    /// Try to yield an [ExitStatus] from this child process if it has already completed.
    fn try_wait(&mut self) -> Result<Option<ExitStatus>, std::io::Error>;

    /// Asynchronously wait for the completion and yielding of an [ExitStatus] from this child
    /// process.
    fn wait(&mut self) -> impl Future<Output = Result<ExitStatus, std::io::Error>> + Send;

    /// Immediately terminate the execution of this child process.
    fn kill(&mut self) -> Result<(), std::io::Error>;

    /// Get the stdout pipe of this child process.
    fn get_stdout(&mut self) -> &mut Option<Self::Stdout>;

    /// Get the stderr pipe of this child process.
    fn get_stderr(&mut self) -> &mut Option<Self::Stderr>;

    /// Get the stdin pipe of this child process.
    fn get_stdin(&mut self) -> &mut Option<Self::Stdin>;

    /// Take out the stdout pipe of this child process.
    fn take_stdout(&mut self) -> Option<Self::Stdout>;

    /// Take out the stderr pipe of this child process.
    fn take_stderr(&mut self) -> Option<Self::Stderr>;

    /// Take out the stdin pipe of this child process.
    fn take_stdin(&mut self) -> Option<Self::Stdin>;
}
