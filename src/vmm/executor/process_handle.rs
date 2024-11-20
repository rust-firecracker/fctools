use std::{
    os::{
        fd::{FromRawFd, OwnedFd, RawFd},
        unix::process::ExitStatusExt,
    },
    path::PathBuf,
    process::ExitStatus,
};

use crate::runtime::{Runtime, RuntimeAsyncFd, RuntimeExecutor, RuntimeFilesystem, RuntimeProcess};

/// A process handle is a thin abstraction over either an "attached" child process that is a [RuntimeProcess],
/// or a "detached" certain process that isn't a child and is controlled via a [RuntimeAsyncFd] wrapping a
/// Linux pidfd.
#[derive(Debug)]
pub struct ProcessHandle<P: RuntimeProcess>(ProcessHandleInner<P>);

/// The pipes that are extracted from a [ProcessHandle]. These can only be extracted from attached
/// [ProcessHandle]s that haven't had their pipes dropped to /dev/null.
#[derive(Debug)]
pub struct ProcessHandlePipes<P: RuntimeProcess> {
    pub stdout: P::Stdout,
    pub stderr: P::Stderr,
    pub stdin: P::Stdin,
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
enum ProcessHandleInner<P: RuntimeProcess> {
    Attached {
        process: P,
        pipes_dropped: bool,
    },
    Detached {
        raw_pidfd: RawFd,
        exited_rx: futures_channel::oneshot::Receiver<ExitStatus>,
        exited: Option<ExitStatus>,
    },
}

impl<P: RuntimeProcess> ProcessHandle<P> {
    /// Create a [ProcessHandle] from a [RuntimeProcess] that is attached.
    pub fn attached(process: P, pipes_dropped: bool) -> Self {
        Self(ProcessHandleInner::Attached { process, pipes_dropped })
    }

    /// Try to create a [ProcessHandle] from an arbitrary detached PID.
    pub fn detached<R: Runtime>(pid: i32) -> Result<Self, std::io::Error> {
        let raw_pidfd = unsafe { nix::libc::syscall(nix::libc::SYS_pidfd_open, pid, 0) };

        if raw_pidfd == -1 {
            return Err(std::io::Error::last_os_error());
        }

        let raw_pidfd = raw_pidfd as RawFd;
        let (exited_tx, exited_rx) = futures_channel::oneshot::channel();
        let async_pidfd = R::Filesystem::create_async_fd(unsafe { OwnedFd::from_raw_fd(raw_pidfd) })?;

        let _ = R::Executor::spawn(async move {
            if async_pidfd.readable().await.is_ok() {
                let mut exit_status = ExitStatus::from_raw(0);

                if let Ok(content) = R::Filesystem::read_to_string(&PathBuf::from(format!("/proc/{pid}/stat"))).await {
                    if let Some(status_raw) = content.split_whitespace().last().and_then(|value| value.parse().ok()) {
                        exit_status = ExitStatus::from_raw(status_raw);
                    }
                }

                let _ = exited_tx.send(exit_status);
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
                ref mut process,
                pipes_dropped: _,
            } => process.kill(),
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
                ref mut process,
                pipes_dropped: _,
            } => process.wait().await,
            ProcessHandleInner::Detached {
                raw_pidfd: _,
                ref mut exited_rx,
                ref mut exited,
            } => {
                if let Some(exited) = exited {
                    return Ok(*exited);
                }

                let exit_status = exited_rx
                    .await
                    .map_err(|_| std::io::Error::other("Could not recv from task waiting on pidfd"))?;
                *exited = Some(exit_status);
                Ok(exit_status)
            }
        }
    }

    /// Check if the process has exited, returning the [ExitStatus] if so or [None] otherwise.
    pub fn try_wait(&mut self) -> Result<Option<ExitStatus>, std::io::Error> {
        match self.0 {
            ProcessHandleInner::Attached {
                ref mut process,
                pipes_dropped: _,
            } => process.try_wait(),
            ProcessHandleInner::Detached {
                raw_pidfd: _,
                ref mut exited_rx,
                ref mut exited,
            } => {
                if let Some(exited) = exited {
                    return Ok(Some(*exited));
                }

                if let Ok(Some(exit_status)) = exited_rx.try_recv() {
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
    pub fn get_pipes(&mut self) -> Result<ProcessHandlePipes<P>, ProcessHandlePipesError> {
        match self.0 {
            ProcessHandleInner::Detached {
                raw_pidfd: _,
                exited_rx: _,
                exited: _,
            } => Err(ProcessHandlePipesError::ProcessIsDetached),
            ProcessHandleInner::Attached {
                process: ref mut child,
                pipes_dropped,
            } => {
                if pipes_dropped {
                    return Err(ProcessHandlePipesError::PipesWereDropped);
                }

                let stdout = child
                    .take_stdout()
                    .ok_or(ProcessHandlePipesError::PipesWereAlreadyTaken)?;
                let stderr = child
                    .take_stderr()
                    .ok_or(ProcessHandlePipesError::PipesWereAlreadyTaken)?;
                let stdin = child
                    .take_stdin()
                    .ok_or(ProcessHandlePipesError::PipesWereAlreadyTaken)?;

                Ok(ProcessHandlePipes { stdout, stderr, stdin })
            }
        }
    }
}
