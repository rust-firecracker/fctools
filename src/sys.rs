#![allow(unused)]

use std::path::Path;

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

#[inline(always)]
fn rustix_op<T>(result: Result<T, rustix::io::Errno>) -> Result<T, std::io::Error> {
    result.map_err(|errno| std::io::Error::from_raw_os_error(errno.raw_os_error()))
}

#[inline(always)]
fn nix_op<T>(result: Result<T, nix::Error>) -> Result<T, std::io::Error> {
    result.map_err(|_| std::io::Error::last_os_error())
}
