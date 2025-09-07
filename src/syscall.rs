#![allow(unused)]

#[cfg(all(feature = "nix-syscall-backend", not(feature = "rustix-syscall-backend")))]
mod imp_nix {
    #![allow(unused)]

    use std::{
        os::fd::{FromRawFd, OwnedFd, RawFd},
        path::Path,
    };

    use nix::{
        sys::stat::Mode,
        unistd::{Gid, Uid},
    };

    #[inline]
    pub fn chown(path: &Path, uid: u32, gid: u32) -> Result<(), std::io::Error> {
        nix::unistd::chown(path, Some(Uid::from(uid)), Some(Gid::from(gid)))
            .map_err(|_| std::io::Error::last_os_error())
    }

    #[inline]
    pub fn geteuid() -> u32 {
        nix::unistd::geteuid().as_raw()
    }

    #[inline]
    pub fn getegid() -> u32 {
        nix::unistd::getegid().as_raw()
    }

    #[inline]
    pub fn mkfifo(path: &Path) -> Result<(), std::io::Error> {
        nix::unistd::mkfifo(path, Mode::S_IROTH | Mode::S_IWOTH | Mode::S_IRUSR | Mode::S_IWUSR)
            .map_err(|_| std::io::Error::last_os_error())
    }

    #[inline]
    pub fn pidfd_open(pid: i32) -> Result<OwnedFd, std::io::Error> {
        // pidfd_open isn't wrapped in nix or libc, so a libc-wrapped syscall is needed
        let fd = unsafe { nix::libc::syscall(nix::libc::SYS_pidfd_open, pid, 0) };

        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }

        Ok(unsafe { OwnedFd::from_raw_fd(fd as i32) })
    }

    #[inline]
    pub fn pidfd_send_sigkill(fd: RawFd) -> Result<(), std::io::Error> {
        // pidfd_send_signal isn't wrapped in nix or libc, so a libc-wrapped syscall is needed
        let ret = unsafe { nix::libc::syscall(nix::libc::SYS_pidfd_send_signal, fd, nix::libc::SIGKILL, 0, 0) };

        if ret < 0 {
            return Err(std::io::Error::last_os_error());
        }

        Ok(())
    }
}

#[cfg(feature = "rustix-syscall-backend")]
mod imp_rustix {
    #![allow(unused)]

    use std::{
        os::fd::{BorrowedFd, OwnedFd, RawFd},
        path::Path,
    };

    use rustix::fs::Mode;

    #[inline]
    pub fn chown(path: &Path, uid: u32, gid: u32) -> Result<(), std::io::Error> {
        rustix::fs::chown(
            path,
            Some(rustix::fs::Uid::from_raw(uid)),
            Some(rustix::fs::Gid::from_raw(gid)),
        )
        .map_err(|errno| std::io::Error::from_raw_os_error(errno.raw_os_error()))
    }

    #[inline]
    pub fn geteuid() -> u32 {
        rustix::process::geteuid().as_raw()
    }

    #[inline]
    pub fn getegid() -> u32 {
        rustix::process::getegid().as_raw()
    }

    #[inline]
    pub fn mkfifo(path: &Path) -> Result<(), std::io::Error> {
        rustix::fs::mknodat(
            unsafe { BorrowedFd::borrow_raw(0) },
            path,
            rustix::fs::FileType::Fifo,
            Mode::ROTH | Mode::WOTH | Mode::RUSR | Mode::WUSR,
            u64::MAX,
        )
        .map_err(|errno| std::io::Error::from_raw_os_error(errno.raw_os_error()))
    }

    #[inline]
    pub fn pidfd_open(pid: i32) -> Result<OwnedFd, std::io::Error> {
        rustix::process::pidfd_open(
            rustix::process::Pid::from_raw(pid).ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "The provided PID for pidfd_open was negative",
                )
            })?,
            rustix::process::PidfdFlags::empty(),
        )
        .map_err(|errno| std::io::Error::from_raw_os_error(errno.raw_os_error()))
    }

    #[inline]
    pub fn pidfd_send_sigkill(fd: RawFd) -> Result<(), std::io::Error> {
        rustix::process::pidfd_send_signal(unsafe { BorrowedFd::borrow_raw(fd) }, rustix::process::Signal::KILL)
            .map_err(|errno| std::io::Error::from_raw_os_error(errno.raw_os_error()))
    }
}

#[cfg(not(any(feature = "nix-syscall-backend", feature = "rustix-syscall-backend")))]
mod imp_dummy {
    use std::{
        os::fd::{OwnedFd, RawFd},
        path::Path,
    };

    #[inline]
    pub fn chown(path: &Path, uid: u32, gid: u32) -> Result<(), std::io::Error> {
        panic!("No syscall backend was enabled for fctools");
    }

    #[inline]
    pub fn geteuid() -> u32 {
        panic!("No syscall backend was enabled for fctools");
    }

    #[inline]
    pub fn getegid() -> u32 {
        panic!("No syscall backend was enabled for fctools");
    }

    #[inline]
    pub fn mkfifo(path: &Path) -> Result<(), std::io::Error> {
        panic!("No syscall backend was enabled for fctools");
    }

    #[inline]
    pub fn pidfd_open(pid: i32) -> Result<OwnedFd, std::io::Error> {
        panic!("No syscall backend was enabled for fctools");
    }

    #[inline]
    pub fn pidfd_send_sigkill(fd: RawFd) -> Result<(), std::io::Error> {
        panic!("No syscall backend was enabled for fctools");
    }
}

#[cfg(all(feature = "nix-syscall-backend", not(feature = "rustix-syscall-backend")))]
pub use imp_nix::*;

#[cfg(feature = "rustix-syscall-backend")]
pub use imp_rustix::*;

#[cfg(not(any(feature = "nix-syscall-backend", feature = "rustix-syscall-backend")))]
pub use imp_dummy::*;
