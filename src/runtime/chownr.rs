use std::path::Path;

pub fn chownr_recursive(path: &Path, uid: u32, gid: u32) -> Result<(), std::io::Error> {
    if path.is_dir() {
        for entry in std::fs::read_dir(path)? {
            let entry = entry?;
            chownr_recursive(entry.path().as_path(), uid, gid)?;
        }
    }

    crate::sys::chown(path, uid, gid)
}
