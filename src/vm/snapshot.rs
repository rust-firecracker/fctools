use std::path::PathBuf;

use crate::runtime::{Runtime, RuntimeFilesystem};

use super::{
    configuration::{VmConfiguration, VmConfigurationData},
    models::{LoadSnapshot, MemoryBackend, MemoryBackendType},
};

/// The data associated with a snapshot created for a [Vm](crate::vm::Vm).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotData {
    pub snapshot_path: PathBuf,
    pub mem_file_path: PathBuf,
    pub configuration_data: VmConfigurationData,
}

impl SnapshotData {
    /// Copy over the data of this snapshot to the given destinations using the given [FsBackend],
    /// also modifying the data to refer to these new destinations.
    pub async fn copy<R: Runtime>(
        &mut self,
        new_snapshot_path: PathBuf,
        new_mem_file_path: PathBuf,
    ) -> Result<(), std::io::Error> {
        futures_util::try_join!(
            R::Filesystem::copy(&self.snapshot_path, &new_snapshot_path),
            R::Filesystem::copy(&self.mem_file_path, &new_mem_file_path),
        )?;

        self.snapshot_path = new_snapshot_path;
        self.mem_file_path = new_mem_file_path;
        Ok(())
    }

    /// Move out the data of this snapshot to the given destinations, mitigating the overhead of copying
    /// when acceptable and also modifying references to these new destinations.
    pub async fn move_to<R: Runtime>(
        &mut self,
        new_snapshot_path: PathBuf,
        new_mem_file_path: PathBuf,
    ) -> Result<(), std::io::Error> {
        futures_util::try_join!(
            R::Filesystem::rename_file(&self.snapshot_path, &new_snapshot_path),
            R::Filesystem::rename_file(&self.mem_file_path, &new_mem_file_path),
        )?;

        self.snapshot_path = new_snapshot_path;
        self.mem_file_path = new_mem_file_path;
        Ok(())
    }

    /// Remove the data of this snapshot.
    pub async fn remove<R: Runtime>(self) -> Result<(), std::io::Error> {
        futures_util::try_join!(
            R::Filesystem::remove_file(&self.snapshot_path),
            R::Filesystem::remove_file(&self.mem_file_path),
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
}
