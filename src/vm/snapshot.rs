use std::path::PathBuf;

use crate::{
    process_spawner::ProcessSpawner,
    runtime::Runtime,
    vm::models::{LoadSnapshot, MemoryBackend, MemoryBackendType},
    vmm::resource::{
        system::{ResourceSystem, ResourceSystemError},
        template::ResourceTemplate,
        MovedResourceType, ResourceState, ResourceType,
    },
};

use super::configuration::{VmConfiguration, VmConfigurationData};

/// The data associated with a snapshot created for a [Vm](crate::vm::Vm).
#[derive(Debug, Clone)]
pub struct VmSnapshot {
    pub snapshot: ResourceTemplate,
    pub mem_file: ResourceTemplate,
    pub configuration_data: VmConfigurationData,
}

// TODO: ensure remove and into_configuration give back ownership in error case

impl VmSnapshot {
    pub async fn copy<P: Into<PathBuf>, Q: Into<PathBuf>, R: Runtime>(
        &mut self,
        runtime: &R,
        new_snapshot_path: P,
        new_mem_file_path: Q,
    ) -> Result<(), ResourceSystemError> {
        futures_util::try_join!(
            self.snapshot.copy(runtime, new_snapshot_path),
            self.mem_file.copy(runtime, new_mem_file_path)
        )
        .map(|_| ())
    }

    pub async fn remove<R: Runtime>(self, runtime: &R) -> Result<(), ResourceSystemError> {
        futures_util::try_join!(self.snapshot.remove(runtime), self.mem_file.remove(runtime))
            .map(|_| ())
            .map_err(|(_, err)| err)
    }

    pub fn into_configuration<S: ProcessSpawner, R: Runtime>(
        mut self,
        resource_system: &mut ResourceSystem<S, R>,
        moved_resource_type: MovedResourceType,
        enable_diff_snapshots: Option<bool>,
        resume_vm: Option<bool>,
    ) -> Result<VmConfiguration, ResourceSystemError> {
        if !self.mem_file.clean(ResourceType::Moved(moved_resource_type))
            || !self.snapshot.clean(ResourceType::Moved(moved_resource_type))
        {
            return Err(ResourceSystemError::IncorrectState(ResourceState::Uninitialized));
        }

        let mem_file = resource_system.create_resource_from_template(self.mem_file)?;
        let snapshot = resource_system.create_resource_from_template(self.snapshot)?;

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
