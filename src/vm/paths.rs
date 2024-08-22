use std::{collections::HashMap, path::PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VmStandardPaths {
    pub(crate) drive_sockets: HashMap<String, PathBuf>,
    pub(crate) metrics_path: Option<PathBuf>,
    pub(crate) log_path: Option<PathBuf>,
    pub(crate) vsock_uds_path: Option<PathBuf>,
}

impl VmStandardPaths {
    pub fn get_drive_sockets(&self) -> &HashMap<String, PathBuf> {
        &self.drive_sockets
    }

    pub fn get_drive_socket(&self, drive_id: impl AsRef<str>) -> Option<&PathBuf> {
        self.drive_sockets.get(drive_id.as_ref())
    }

    pub fn get_metrics_path(&self) -> Option<&PathBuf> {
        self.metrics_path.as_ref()
    }

    pub fn get_log_path(&self) -> Option<&PathBuf> {
        self.log_path.as_ref()
    }

    pub fn get_vsock_uds_path(&self) -> Option<&PathBuf> {
        self.vsock_uds_path.as_ref()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct VmSnapshotPaths {
    pub(crate) snapshot_path: PathBuf,
    pub(crate) mem_file_path: PathBuf,
}

impl VmSnapshotPaths {
    pub fn get_snapshot_path(&self) -> &PathBuf {
        &self.snapshot_path
    }

    pub fn get_mem_file_path(&self) -> &PathBuf {
        &self.mem_file_path
    }
}
