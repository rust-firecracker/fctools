use std::{
    os::{
        fd::{AsFd, FromRawFd, OwnedFd, RawFd},
        unix::process::ExitStatusExt,
    },
    process::ExitStatus,
};

use nix::{
    sys::{
        signal::Signal,
        wait::{Id, WaitPidFlag, WaitStatus},
    },
    unistd::Pid,
};
use tokio::{
    io::unix::AsyncFd,
    process::{Child, ChildStderr, ChildStdin, ChildStdout},
};

pub struct ProcessHandle(ProcessHandleInner);

pub struct RawPipes {
    pub stdout: ChildStdout,
    pub stderr: ChildStderr,
    pub stdin: ChildStdin,
}

pub enum RawPipesError {
    ProcessIsDetached,
    PipesWereDropped,
    PipeWasMissing,
}

enum ProcessHandleInner {
    Attached { child: Child, pipes_dropped: bool },
    Detached { pid: Pid, pidfd: AsyncFd<OwnedFd> },
}

impl ProcessHandle {
    pub fn attached(child: Child, pipes_dropped: bool) -> Self {
        Self(ProcessHandleInner::Attached { child, pipes_dropped })
    }

    pub fn detached(pid: Pid) -> Result<Self, std::io::Error> {
        let ret = unsafe { nix::libc::syscall(nix::libc::SYS_pidfd_open, pid, 0) };

        if ret == -1 {
            return Err(std::io::Error::last_os_error());
        }

        let pidfd = AsyncFd::new(unsafe { OwnedFd::from_raw_fd(ret as RawFd) })?;
        Ok(Self(ProcessHandleInner::Detached { pid, pidfd }))
    }

    pub fn kill(&mut self) -> Result<(), std::io::Error> {
        match self.0 {
            ProcessHandleInner::Attached {
                ref mut child,
                pipes_dropped: _,
            } => child.start_kill(),
            ProcessHandleInner::Detached { pid, pidfd: _ } => {
                if nix::sys::signal::kill(pid, Signal::SIGKILL).is_err() {
                    Err(std::io::Error::last_os_error())
                } else {
                    Ok(())
                }
            }
        }
    }

    pub async fn wait(&mut self) -> Result<ExitStatus, std::io::Error> {
        match self.0 {
            ProcessHandleInner::Attached {
                ref mut child,
                pipes_dropped: _,
            } => child.wait().await,
            ProcessHandleInner::Detached { pid: _, ref pidfd } => {
                pidfd.readable().await?.retain_ready();

                let result = match nix::sys::wait::waitid(Id::PIDFd(pidfd.as_fd()), WaitPidFlag::WEXITED) {
                    Ok(wait_status) => match wait_status {
                        WaitStatus::Exited(_, exit_status) => Ok(exit_status),
                        _ => Err(std::io::Error::other(
                            "waitid on WEXITED returned something other than EXITED",
                        )),
                    },
                    Err(_) => Err(std::io::Error::last_os_error()),
                }?;

                Ok(ExitStatus::from_raw(result))
            }
        }
    }

    pub fn try_wait(&mut self) -> Result<Option<ExitStatus>, std::io::Error> {
        match self.0 {
            ProcessHandleInner::Attached {
                ref mut child,
                pipes_dropped: _,
            } => child.try_wait(),
            ProcessHandleInner::Detached { pid: _, ref pidfd } => {
                let result =
                    match nix::sys::wait::waitid(Id::PIDFd(pidfd.as_fd()), WaitPidFlag::WNOHANG | WaitPidFlag::WEXITED)
                    {
                        Ok(wait_status) => match wait_status {
                            WaitStatus::Exited(_, exit_status) => Ok(Some(exit_status)),
                            WaitStatus::StillAlive => Ok(None),
                            _ => Err(std::io::Error::other(
                                "waitid on WNOHANG and WEXITED returned something unexpected",
                            )),
                        },
                        Err(_) => Err(std::io::Error::last_os_error()),
                    }?;

                Ok(result.map(ExitStatus::from_raw))
            }
        }
    }

    pub fn get_pipes(&mut self) -> Result<RawPipes, RawPipesError> {
        match self.0 {
            ProcessHandleInner::Detached { pid: _, pidfd: _ } => Err(RawPipesError::ProcessIsDetached),
            ProcessHandleInner::Attached {
                ref mut child,
                pipes_dropped,
            } => {
                if pipes_dropped {
                    return Err(RawPipesError::PipesWereDropped);
                }

                let stdout = child.stdout.take().ok_or(RawPipesError::PipeWasMissing)?;
                let stderr = child.stderr.take().ok_or(RawPipesError::PipeWasMissing)?;
                let stdin = child.stdin.take().ok_or(RawPipesError::PipeWasMissing)?;

                Ok(RawPipes { stdout, stderr, stdin })
            }
        }
    }
}