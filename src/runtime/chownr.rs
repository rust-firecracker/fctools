use std::path::Path;

use nix::unistd::{Gid, Uid};

pub fn chownr_recursive(path: &Path, uid: Uid, gid: Gid) -> Result<(), std::io::Error> {
    if path.is_dir() {
        for entry in std::fs::read_dir(path)? {
            let entry = entry?;
            chownr_recursive(entry.path().as_path(), uid, gid)?;
        }
    }

    if nix::unistd::chown(path, Some(uid), Some(gid)).is_err() {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}
