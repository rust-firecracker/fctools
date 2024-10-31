use std::{
    os::{
        fd::{AsFd, FromRawFd, OwnedFd, RawFd},
        unix::process::ExitStatusExt,
    },
    process::ExitStatus,
};

use nix::{
    sys::wait::{Id, WaitPidFlag, WaitStatus},
    unistd::Pid,
};
use tokio::{
    io::unix::AsyncFd,
    process::{Child, ChildStderr, ChildStdin, ChildStdout},
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
        pidfd: AsyncFd<OwnedFd>,
        reaped_exit_status: Option<ExitStatus>,
    },
}

impl ProcessHandle {
    /// Create a [ProcessHandle] from a Tokio [Child] that is attached.
    pub fn attached(child: Child, pipes_dropped: bool) -> Self {
        Self(ProcessHandleInner::Attached { child, pipes_dropped })
    }

    /// Try to create a [ProcessHandle] from an arbitrary detached PID.
    pub fn detached(pid: Pid) -> Result<Self, std::io::Error> {
        let ret = unsafe { nix::libc::syscall(nix::libc::SYS_pidfd_open, pid, 0) };

        if ret == -1 {
            return Err(std::io::Error::last_os_error());
        }

        let pidfd = AsyncFd::new(unsafe { OwnedFd::from_raw_fd(ret as RawFd) })?;
        Ok(Self(ProcessHandleInner::Detached {
            pidfd,
            reaped_exit_status: None,
        }))
    }

    pub fn send_sigkill(&mut self) -> Result<(), std::io::Error> {
        match self.0 {
            ProcessHandleInner::Attached {
                ref mut child,
                pipes_dropped: _,
            } => child.start_kill(),
            ProcessHandleInner::Detached {
                ref pidfd,
                ref mut reaped_exit_status,
            } => {
                if reaped_exit_status.is_some() {
                    return Err(std::io::Error::other("Tried to kill process which already exited"));
                }

                let ret = unsafe {
                    nix::libc::syscall(
                        nix::libc::SYS_pidfd_send_signal,
                        pidfd.as_fd(),
                        nix::libc::SIGKILL,
                        0,
                        0,
                    )
                };

                if ret == -1 {
                    return Err(std::io::Error::last_os_error());
                }

                Ok(())
            }
        }
    }

    pub async fn wait(&mut self) -> Result<ExitStatus, std::io::Error> {
        match self.0 {
            ProcessHandleInner::Attached {
                ref mut child,
                pipes_dropped: _,
            } => child.wait().await,
            ProcessHandleInner::Detached {
                ref pidfd,
                ref mut reaped_exit_status,
            } => {
                if let Some(reaped_exit_status) = reaped_exit_status {
                    return Ok(*reaped_exit_status);
                }

                pidfd.readable().await?.retain_ready();

                match nix::sys::wait::waitid(Id::PIDFd(pidfd.as_fd()), WaitPidFlag::WEXITED) {
                    Ok(wait_status) => match wait_status {
                        WaitStatus::Exited(_, wait_status) => {
                            let exit_status = ExitStatus::from_raw(wait_status);
                            *reaped_exit_status = Some(exit_status);
                            Ok(exit_status)
                        }
                        WaitStatus::Signaled(_, signal, _) => {
                            let exit_status = ExitStatus::from_raw(signal as i32);
                            *reaped_exit_status = Some(exit_status);
                            Ok(exit_status)
                        }
                        _ => Err(std::io::Error::other(
                            "waitid on WEXITED returned something other than exited or signaled",
                        )),
                    },
                    Err(_) => Err(std::io::Error::last_os_error()),
                }
            }
        }
    }

    pub fn try_wait(&mut self) -> Result<Option<ExitStatus>, std::io::Error> {
        match self.0 {
            ProcessHandleInner::Attached {
                ref mut child,
                pipes_dropped: _,
            } => child.try_wait(),
            ProcessHandleInner::Detached {
                ref pidfd,
                ref mut reaped_exit_status,
            } => {
                if let Some(reaped_exit_status) = reaped_exit_status {
                    return Ok(Some(*reaped_exit_status));
                }

                match nix::sys::wait::waitid(Id::PIDFd(pidfd.as_fd()), WaitPidFlag::WNOHANG | WaitPidFlag::WEXITED) {
                    Ok(wait_status) => match wait_status {
                        WaitStatus::Exited(_, wait_status) => {
                            let exit_status = ExitStatus::from_raw(wait_status);
                            *reaped_exit_status = Some(exit_status);
                            Ok(Some(exit_status))
                        }
                        _ => Ok(None),
                    },
                    Err(_) => Err(std::io::Error::last_os_error()),
                }
            }
        }
    }

    pub fn get_pipes(&mut self) -> Result<ProcessHandlePipes, ProcessHandlePipesError> {
        match self.0 {
            ProcessHandleInner::Detached {
                pidfd: _,
                reaped_exit_status: _,
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
