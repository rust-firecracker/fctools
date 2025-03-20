use crate::{
    process_spawner::ProcessSpawner,
    runtime::Runtime,
    vm::models::{LoadSnapshot, MemoryBackend, MemoryBackendType},
    vmm::resource::{
        detached::DetachedResource,
        system::{ResourceSystem, ResourceSystemError},
        MovedResourceType, ResourceState, ResourceType,
    },
};

use super::configuration::{VmConfiguration, VmConfigurationData};

/// The data associated with a snapshot created for a [Vm](crate::vm::Vm).
#[derive(Debug, Clone)]
pub struct VmSnapshot {
    pub snapshot: DetachedResource,
    pub mem_file: DetachedResource,
    pub configuration_data: VmConfigurationData,
}

impl VmSnapshot {
    // pub async fn copy<R: Runtime>(
    //     &mut self,
    //     runtime: &R,
    //     new_snapshot_path: impl Into<PathBuf>,
    //     new_mem_file_path: impl Into<PathBuf>,
    // ) -> Result<(), std::io::Error> {
    //     futures_util::try_join!(
    //         self.snapshot.copy(runtime, new_snapshot_path),
    //         self.mem_file.copy(runtime, new_mem_file_path)
    //     )
    //     .map(|_| ())
    // }

    // pub async fn rename<R: Runtime>(
    //     &mut self,
    //     runtime: &R,
    //     new_snapshot_path: impl Into<PathBuf>,
    //     new_mem_file_path: impl Into<PathBuf>,
    // ) -> Result<(), std::io::Error> {
    //     futures_util::try_join!(
    //         self.snapshot.rename(runtime, new_snapshot_path),
    //         self.mem_file.rename(runtime, new_mem_file_path)
    //     )
    //     .map(|_| ())
    // }

    // pub async fn remove<R: Runtime>(self, runtime: &R) -> Result<(), std::io::Error> {
    //     futures_util::try_join!(self.snapshot.remove(runtime), self.mem_file.remove(runtime))
    //         .map(|_| ())
    //         .map_err(|(_, err)| err)
    // }

    pub fn into_configuration<S: ProcessSpawner, R: Runtime>(
        mut self,
        resource_system: &mut ResourceSystem<S, R>,
        moved_resource_type: MovedResourceType,
        enable_diff_snapshots: Option<bool>,
        resume_vm: Option<bool>,
    ) -> Result<VmConfiguration, ResourceSystemError> {
        if !self.mem_file.deinitialize(ResourceType::Moved(moved_resource_type))
            || !self.snapshot.deinitialize(ResourceType::Moved(moved_resource_type))
        {
            return Err(ResourceSystemError::IncorrectState(ResourceState::Uninitialized));
        }

        let mem_file = self.mem_file.attach(resource_system).map_err(|(_, err)| err)?;
        let snapshot = self.snapshot.attach(resource_system).map_err(|(_, err)| err)?;

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
