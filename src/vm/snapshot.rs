use std::path::PathBuf;

use crate::{
    runtime::Runtime,
    vmm::resource::{MovedVmmResource, ProducedVmmResource, VmmResourceMoveMethod},
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
    ) -> Result<(), std::io::Error> {
        futures_util::try_join!(
            self.snapshot.copy::<R>(new_snapshot_path),
            self.mem_file.copy::<R>(new_mem_file_path)
        )
        .map(|_| ())
    }

    pub async fn remove<R: Runtime>(self) -> Result<(), std::io::Error> {
        futures_util::try_join!(self.snapshot.remove::<R>(), self.mem_file.remove::<R>())
            .map(|_| ())
            .map_err(|errs| errs.0)
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
