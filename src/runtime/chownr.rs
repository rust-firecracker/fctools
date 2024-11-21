use std::path::Path;

use crate::vmm::with_c_path_ptr;

pub fn chownr_recursive(path: &Path, uid: u32, gid: u32) -> Result<(), std::io::Error> {
    if path.is_dir() {
        for entry in std::fs::read_dir(path)? {
            let entry = entry?;
            chownr_recursive(entry.path().as_path(), uid, gid)?;
        }
    }

    let ret = with_c_path_ptr(path, |ptr| unsafe { libc::chown(ptr, uid, gid) });

    if ret < 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}
