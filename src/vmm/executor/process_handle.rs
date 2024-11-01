use std::{
    os::{fd::RawFd, unix::process::ExitStatusExt},
    process::ExitStatus,
};

use tokio::{
    io::unix::AsyncFd,
    process::{Child, ChildStderr, ChildStdin, ChildStdout},
    sync::oneshot,
};

/// A process handle is a thin abstraction over either an "attached" child process that is a Tokio [Child],
/// or a "detached" certain process that isn't a child and is controlled via a pidfd.
#[derive(Debug)]
pub struct ProcessHandle(ProcessHandleInner);

/// The pipes that are extracted from a [ProcessHandle]. These can only be extracted from attached
/// [ProcessHandle]s that haven't had their pipes dropped to /dev/null.
#[derive(Debug)]
pub struct ProcessHandlePipes {
    pub stdout: ChildStdout,
    pub stderr: ChildStderr,
    pub stdin: ChildStdin,
}

/// An error that didn't allow the extraction of [ProcessHandlePipes] from a [ProcessHandle].
#[derive(Debug, thiserror::Error)]
pub enum ProcessHandlePipesError {
    #[error("The handle points to a detached process outside the PID namespace of the current one")]
    ProcessIsDetached,
    #[error("The pipes of the process were dropped")]
    PipesWereDropped,
    #[error("The pipes were already taken (given ownership of)")]
    PipesWereAlreadyTaken,
}

#[derive(Debug)]
enum ProcessHandleInner {
    Attached {
        child: Child,
        pipes_dropped: bool,
    },
    Detached {
        raw_pidfd: RawFd,
        exited_rx: oneshot::Receiver<()>,
        exited: Option<ExitStatus>,
    },
}

impl ProcessHandle {
    /// Create a [ProcessHandle] from a Tokio [Child] that is attached.
    pub fn attached(child: Child, pipes_dropped: bool) -> Self {
        Self(ProcessHandleInner::Attached { child, pipes_dropped })
    }

    /// Try to create a [ProcessHandle] from an arbitrary detached PID.
    pub fn detached(pid: i32) -> Result<Self, std::io::Error> {
        let raw_pidfd = unsafe { nix::libc::syscall(nix::libc::SYS_pidfd_open, pid, 0) };

        if raw_pidfd == -1 {
            return Err(std::io::Error::last_os_error());
        }

        let raw_pidfd = raw_pidfd as RawFd;
        let (exited_tx, exited_rx) = oneshot::channel();
        let async_pidfd = AsyncFd::new(raw_pidfd)?;

        tokio::task::spawn(async move {
            if async_pidfd.readable().await.is_ok() {
                let _ = exited_tx.send(());
            }
        });

        Ok(Self(ProcessHandleInner::Detached {
            raw_pidfd,
            exited_rx,
            exited: None,
        }))
    }

    /// Send a SIGKILL signal to the process.
    pub fn send_sigkill(&mut self) -> Result<(), std::io::Error> {
        match self.0 {
            ProcessHandleInner::Attached {
                ref mut child,
                pipes_dropped: _,
            } => child.start_kill(),
            ProcessHandleInner::Detached {
                raw_pidfd,
                exited_rx: _,
                exited,
            } => {
                if exited.is_some() {
                    return Err(std::io::Error::other("Trying to send SIGKILL to exited process"));
                }

                let ret = unsafe {
                    nix::libc::syscall(nix::libc::SYS_pidfd_send_signal, raw_pidfd, nix::libc::SIGKILL, 0, 0)
                };

                if ret == -1 {
                    return Err(std::io::Error::last_os_error());
                }

                Ok(())
            }
        }
    }

    /// Wait for the process to have exited.
    pub async fn wait(&mut self) -> Result<ExitStatus, std::io::Error> {
        match self.0 {
            ProcessHandleInner::Attached {
                ref mut child,
                pipes_dropped: _,
            } => child.wait().await,
            ProcessHandleInner::Detached {
                raw_pidfd: _,
                ref mut exited_rx,
                ref mut exited,
            } => {
                if let Some(exited) = exited {
                    return Ok(*exited);
                }

                let _ = exited_rx.await;
                let exit_status = ExitStatus::from_raw(0);
                *exited = Some(exit_status);
                Ok(exit_status)
            }
        }
    }

    /// Check if the process has exited, returning the [ExitStatus] if so or [None] otherwise.
    pub fn try_wait(&mut self) -> Result<Option<ExitStatus>, std::io::Error> {
        match self.0 {
            ProcessHandleInner::Attached {
                ref mut child,
                pipes_dropped: _,
            } => child.try_wait(),
            ProcessHandleInner::Detached {
                raw_pidfd: _,
                ref mut exited_rx,
                ref mut exited,
            } => {
                if let Some(exited) = exited {
                    return Ok(Some(*exited));
                }

                if exited_rx.try_recv().is_ok() {
                    let exit_status = ExitStatus::from_raw(0);
                    *exited = Some(exit_status);
                    Ok(Some(exit_status))
                } else {
                    Ok(None)
                }
            }
        }
    }

    /// Try to get the [ProcessHandlePipes] for this process. Only possible for attached (child)
    /// processes that haven't had their pipes dropped when creating.
    pub fn get_pipes(&mut self) -> Result<ProcessHandlePipes, ProcessHandlePipesError> {
        match self.0 {
            ProcessHandleInner::Detached {
                raw_pidfd: _,
                exited_rx: _,
                exited: _,
            } => Err(ProcessHandlePipesError::ProcessIsDetached),
            ProcessHandleInner::Attached {
                ref mut child,
                pipes_dropped,
            } => {
                if pipes_dropped {
                    return Err(ProcessHandlePipesError::PipesWereDropped);
                }

                let stdout = child
                    .stdout
                    .take()
                    .ok_or(ProcessHandlePipesError::PipesWereAlreadyTaken)?;
                let stderr = child
                    .stderr
                    .take()
                    .ok_or(ProcessHandlePipesError::PipesWereAlreadyTaken)?;
                let stdin = child
                    .stdin
                    .take()
                    .ok_or(ProcessHandlePipesError::PipesWereAlreadyTaken)?;

                Ok(ProcessHandlePipes { stdout, stderr, stdin })
            }
        }
    }
}
