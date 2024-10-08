use std::path::PathBuf;

use super::{
    configuration::{VmConfiguration, VmConfigurationData},
    models::{LoadSnapshot, MemoryBackend, MemoryBackendType},
};

/// The data associated with a snapshot created for a VM.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotData {
    pub(super) snapshot_path: PathBuf,
    pub(super) mem_file_path: PathBuf,
    pub(super) configuration_data: VmConfigurationData,
}

impl SnapshotData {
    /// Copy over the data of this snapshot to the given destinations, also modifying the data to refer
    /// to these new destinations.
    pub async fn copy(
        &mut self,
        new_snapshot_path: PathBuf,
        new_mem_file_path: PathBuf,
    ) -> Result<(), tokio::io::Error> {
        tokio::try_join!(
            tokio::fs::copy(&self.snapshot_path, &new_snapshot_path),
            tokio::fs::copy(&self.mem_file_path, &new_mem_file_path)
        )?;

        self.snapshot_path = new_snapshot_path;
        self.mem_file_path = new_mem_file_path;
        Ok(())
    }

    /// Move out the data of this snapshot to the given destinations, mitigating the overhead of copying
    /// when acceptable and also modifying references to these new destinations.
    pub async fn move_out(
        &mut self,
        new_snapshot_path: PathBuf,
        new_mem_file_path: PathBuf,
    ) -> Result<(), tokio::io::Error> {
        tokio::try_join!(
            tokio::fs::rename(&self.snapshot_path, &new_snapshot_path),
            tokio::fs::rename(&self.mem_file_path, &new_mem_file_path)
        )?;

        self.snapshot_path = new_snapshot_path;
        self.mem_file_path = new_mem_file_path;
        Ok(())
    }

    /// Remove the data of this snapshot.
    pub async fn remove(self) -> Result<(), tokio::io::Error> {
        tokio::try_join!(
            tokio::fs::remove_file(&self.snapshot_path),
            tokio::fs::remove_file(&self.mem_file_path)
        )?;
        Ok(())
    }

    /// Transform this snapshot into a VmConfiguration. A file will be used as the snapshot memory backend and not a
    /// userfaultfd/UFFD, and resume_vm and enable_diff_snapshots can be used to customize the corresponding options.
    pub fn into_configuration(self, resume_vm: Option<bool>, enable_diff_snapshots: Option<bool>) -> VmConfiguration {
        let mut load_snapshot = LoadSnapshot::new(
            self.snapshot_path,
            MemoryBackend::new(MemoryBackendType::File, self.mem_file_path),
        );

        if let Some(resume_vm) = resume_vm {
            load_snapshot = load_snapshot.resume_vm(resume_vm);
        }

        if let Some(enable_diff_snapshots) = enable_diff_snapshots {
            load_snapshot = load_snapshot.enable_diff_snapshots(enable_diff_snapshots);
        }

        VmConfiguration::RestoredFromSnapshot {
            load_snapshot,
            data: self.configuration_data,
        }
    }

    pub fn snapshot_path(&self) -> &PathBuf {
        &self.snapshot_path
    }

    pub fn mem_file_path(&self) -> &PathBuf {
        &self.mem_file_path
    }

    pub fn configuration_data(&self) -> &VmConfigurationData {
        &self.configuration_data
    }
}
