use std::path::PathBuf;

use crate::{
    process_spawner::ProcessSpawner,
    runtime::Runtime,
    vm::models::{LoadSnapshot, MemoryBackend, MemoryBackendType},
    vmm::resource::{
        path::DetachedPath,
        system::{ResourceSystem, ResourceSystemError},
        MovedResourceType,
    },
};

use super::configuration::{VmConfiguration, VmConfigurationData};

/// The data associated with a snapshot created for a [Vm](crate::vm::Vm).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VmSnapshot {
    pub snapshot_path: DetachedPath,
    pub mem_file_path: DetachedPath,
    pub configuration_data: VmConfigurationData,
}

impl VmSnapshot {
    pub async fn copy<R: Runtime>(
        &mut self,
        runtime: &R,
        new_snapshot_path: impl Into<PathBuf>,
        new_mem_file_path: impl Into<PathBuf>,
    ) -> Result<(), std::io::Error> {
        futures_util::try_join!(
            self.snapshot_path.copy(runtime, new_snapshot_path),
            self.mem_file_path.copy(runtime, new_mem_file_path)
        )
        .map(|_| ())
    }

    pub async fn rename<R: Runtime>(
        &mut self,
        runtime: &R,
        new_snapshot_path: impl Into<PathBuf>,
        new_mem_file_path: impl Into<PathBuf>,
    ) -> Result<(), std::io::Error> {
        futures_util::try_join!(
            self.snapshot_path.rename(runtime, new_snapshot_path),
            self.mem_file_path.rename(runtime, new_mem_file_path)
        )
        .map(|_| ())
    }

    pub async fn remove<R: Runtime>(self, runtime: &R) -> Result<(), std::io::Error> {
        futures_util::try_join!(self.snapshot_path.remove(runtime), self.mem_file_path.remove(runtime))
            .map(|_| ())
            .map_err(|(_, err)| err)
    }

    pub fn into_configuration<S: ProcessSpawner, R: Runtime>(
        self,
        resource_system: &mut ResourceSystem<S, R>,
        moved_resource_type: MovedResourceType,
        enable_diff_snapshots: Option<bool>,
        resume_vm: Option<bool>,
    ) -> Result<VmConfiguration, (Self, ResourceSystemError)> {
        let mem_file =
            match resource_system.new_moved_resource(self.mem_file_path.clone().into_inner(), moved_resource_type) {
                Ok(resource) => resource,
                Err(err) => return Err((self, err)),
            };

        let snapshot =
            match resource_system.new_moved_resource(self.snapshot_path.clone().into_inner(), moved_resource_type) {
                Ok(resource) => resource,
                Err(err) => return Err((self, err)),
            };

        let load_snapshot = LoadSnapshot {
            enable_diff_snapshots,
            mem_backend: MemoryBackend {
                backend_type: MemoryBackendType::File,
                backend: mem_file,
            },
            snapshot,
            resume_vm,
        };

        Ok(VmConfiguration::RestoredFromSnapshot {
            load_snapshot,
            data: self.configuration_data,
        })
    }
}
