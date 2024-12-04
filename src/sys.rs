#![allow(unused)]

use std::os::fd::{AsFd, BorrowedFd, OwnedFd, RawFd};
use std::path::Path;

#[cfg(feature = "sys-nix")]
use nix::sys::stat::Mode as NixMode;
#[cfg(feature = "sys-rustix")]
use rustix::fs::Mode as RustixMode;
#[cfg(feature = "sys-rustix")]
use std::os::fd::FromRawFd;

#[inline]
pub fn chown(path: &Path, uid: u32, gid: u32) -> Result<(), std::io::Error> {
    if cfg!(feature = "sys-rustix") {
        rustix_op(rustix::fs::chown(
            path,
            Some(unsafe { rustix::fs::Uid::from_raw(uid) }),
            Some(unsafe { rustix::fs::Gid::from_raw(gid) }),
        ))
    } else {
        nix_op(nix::unistd::chown(path, Some(uid.into()), Some(gid.into())))
    }
}

#[inline]
pub fn geteuid() -> u32 {
    if cfg!(feature = "sys-rustix") {
        rustix::process::geteuid().as_raw()
    } else {
        nix::unistd::geteuid().as_raw()
    }
}

#[inline]
pub fn getegid() -> u32 {
    if cfg!(feature = "sys-rustix") {
        rustix::process::getegid().as_raw()
    } else {
        nix::unistd::getegid().as_raw()
    }
}

#[inline]
pub fn mkfifo(path: &Path) -> Result<(), std::io::Error> {
    if cfg!(feature = "sys-rustix") {
        rustix_op(rustix::fs::mknodat(
            unsafe { BorrowedFd::borrow_raw(0) },
            path,
            rustix::fs::FileType::Fifo,
            RustixMode::ROTH | RustixMode::WOTH | RustixMode::RUSR | RustixMode::WUSR,
            u64::MAX,
        ))
    } else {
        nix_op(nix::unistd::mkfifo(
            path,
            NixMode::S_IROTH | NixMode::S_IWOTH | NixMode::S_IRUSR | NixMode::S_IWUSR,
        ))
    }
}

#[inline]
pub fn pidfd_open(pid: i32) -> Result<OwnedFd, std::io::Error> {
    if cfg!(feature = "sys-rustix") {
        rustix_op(rustix::process::pidfd_open(
            rustix::process::Pid::from_raw(pid).ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "The provided PID for pidfd_open was negative",
                )
            })?,
            rustix::process::PidfdFlags::empty(),
        ))
    } else {
        // pidfd_open isn't wrapped in nix or libc, so a libc-wrapped syscall is needed
        let fd = unsafe { nix::libc::syscall(nix::libc::SYS_pidfd_open, pid, 0) };

        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }

        Ok(unsafe { OwnedFd::from_raw_fd(fd as i32) })
    }
}

#[inline]
pub fn pidfd_send_sigkill(fd: RawFd) -> Result<(), std::io::Error> {
    if cfg!(feature = "sys-rustix") {
        rustix_op(rustix::process::pidfd_send_signal(
            unsafe { BorrowedFd::borrow_raw(fd) },
            rustix::process::Signal::Kill,
        ))
    } else {
        // pidfd_send_signal isn't wrapped in nix or libc, so a libc-wrapped syscall is needed
        let ret = unsafe { nix::libc::syscall(nix::libc::SYS_pidfd_send_signal, fd, nix::libc::SIGKILL, 0, 0) };

        if ret < 0 {
            return Err(std::io::Error::last_os_error());
        }

        Ok(())
    }
}

#[inline(always)]
fn rustix_op<T>(result: Result<T, rustix::io::Errno>) -> Result<T, std::io::Error> {
    result.map_err(|errno| std::io::Error::from_raw_os_error(errno.raw_os_error()))
}

#[inline(always)]
fn nix_op<T>(result: Result<T, nix::Error>) -> Result<T, std::io::Error> {
    result.map_err(|_| std::io::Error::last_os_error())
}
