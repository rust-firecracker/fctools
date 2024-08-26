use std::{collections::HashMap, path::PathBuf};

use super::{
    configuration::{VmConfiguration, VmConfigurationData},
    models::{VmLoadSnapshot, VmMemoryBackend, VmMemoryBackendType},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VmStandardPaths {
    pub(crate) drive_sockets: HashMap<String, PathBuf>,
    pub(crate) metrics_path: Option<PathBuf>,
    pub(crate) log_path: Option<PathBuf>,
    pub(crate) vsock_multiplexer_path: Option<PathBuf>,
    pub(crate) vsock_listener_paths: Vec<PathBuf>,
}

impl VmStandardPaths {
    pub fn add_vsock_listener_path(&mut self, socket_path: impl Into<PathBuf>) {
        self.vsock_listener_paths.push(socket_path.into());
    }

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

    pub fn get_vsock_multiplexer_path(&self) -> Option<&PathBuf> {
        self.vsock_multiplexer_path.as_ref()
    }

    pub fn get_vsock_listener_paths(&self) -> &Vec<PathBuf> {
        &self.vsock_listener_paths
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VmSnapshotResult {
    pub snapshot_path: PathBuf,
    pub mem_file_path: PathBuf,
    pub configuration_data: VmConfigurationData,
}

impl VmSnapshotResult {
    pub fn into_configuration(self, resume_vm: bool) -> VmConfiguration {
        VmConfiguration::RestoredFromSnapshot {
            load_snapshot: VmLoadSnapshot::new(
                self.snapshot_path,
                VmMemoryBackend::new(VmMemoryBackendType::File, self.mem_file_path),
            )
            .resume_vm(resume_vm),
            data: self.configuration_data,
        }
    }
}
