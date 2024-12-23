use std::path::PathBuf;

use crate::{
    runtime::Runtime,
    vmm::resource::{
        moved::{MovedVmmResource, VmmResourceMoveMethod},
        produced::ProducedVmmResource,
    },
};

use super::{
    configuration::{VmConfiguration, VmConfigurationData},
    models::{LoadSnapshot, MemoryBackend, MemoryBackendType},
};

/// The data associated with a snapshot created for a [Vm](crate::vm::Vm).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VmSnapshot {
    pub snapshot: ProducedVmmResource,
    pub mem_file: ProducedVmmResource,
    pub configuration_data: VmConfigurationData,
}

impl VmSnapshot {
    pub async fn copy<R: Runtime>(
        &mut self,
        new_snapshot_path: impl Into<PathBuf>,
        new_mem_file_path: impl Into<PathBuf>,
        runtime: &R,
    ) -> Result<(), std::io::Error> {
        futures_util::try_join!(
            self.snapshot.copy(new_snapshot_path, runtime),
            self.mem_file.copy(new_mem_file_path, runtime)
        )
        .map(|_| ())
    }

    pub async fn rename<R: Runtime>(
        &mut self,
        new_snapshot_path: impl Into<PathBuf>,
        new_mem_file_path: impl Into<PathBuf>,
        runtime: &R,
    ) -> Result<(), std::io::Error> {
        futures_util::try_join!(
            self.snapshot.rename(new_snapshot_path, runtime),
            self.mem_file.rename(new_mem_file_path, runtime)
        )
        .map(|_| ())
    }

    pub async fn remove<R: Runtime>(self, runtime: &R) {
        let _ = futures_util::try_join!(self.snapshot.delete(runtime), self.mem_file.delete(runtime));
    }

    pub fn into_configuration(
        self,
        move_method: VmmResourceMoveMethod,
        enable_diff_snapshots: Option<bool>,
        resume_vm: Option<bool>,
    ) -> VmConfiguration {
        let mem_file = MovedVmmResource::new(self.mem_file.effective_path(), move_method);
        let snapshot = MovedVmmResource::new(self.snapshot.effective_path(), move_method);

        let load_snapshot = LoadSnapshot {
            enable_diff_snapshots,
            mem_backend: MemoryBackend {
                backend_type: MemoryBackendType::File,
                backend: mem_file,
            },
            snapshot,
            resume_vm,
        };

        VmConfiguration::RestoredFromSnapshot {
            load_snapshot,
            data: self.configuration_data,
        }
    }
}
