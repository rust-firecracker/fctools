use std::{path::PathBuf, sync::Arc};

use crate::{
    process_spawner::ProcessSpawner,
    runtime::Runtime,
    vm::models::{LoadSnapshot, MemoryBackend, MemoryBackendType},
    vmm::{
        executor::VmmExecutor,
        ownership::VmmOwnershipModel,
        resource::{
            system::{ResourceSystem, ResourceSystemError},
            MovedResourceType, ResourceState, ResourceType,
        },
    },
};

use super::{
    configuration::{VmConfiguration, VmConfigurationData},
    Vm, VmError,
};

/// The data associated with a snapshot created for a [Vm].
#[derive(Debug, Clone)]
pub struct VmSnapshot {
    /// The owned [PathBuf] pointing to the effective location of the snapshot file.
    pub snapshot_path: PathBuf,
    /// The owned [PathBuf] pointing to the effective location of the memory file.
    pub mem_file_path: PathBuf,
    /// A clone of the original [Vm]'s [VmConfigurationData], necessary to subsequently create
    /// a new [Vm].
    pub configuration_data: VmConfigurationData,
}

#[derive(Debug)]
pub struct PrepareVmFromSnapshotOptions<E: VmmExecutor, S: ProcessSpawner, R: Runtime> {
    pub executor: E,
    pub process_spawner: S,
    pub runtime: R,
    pub moved_resource_type: MovedResourceType,
    pub ownership_model: VmmOwnershipModel,
    pub enable_diff_snapshots: Option<bool>,
    pub resume_vm: Option<bool>,
}

impl VmSnapshot {
    pub async fn copy<P: Into<PathBuf>, Q: Into<PathBuf>, R: Runtime>(
        &mut self,
        runtime: &R,
        new_snapshot_path: P,
        new_mem_file_path: Q,
    ) -> Result<(), ResourceSystemError> {
        let new_snapshot_path = new_snapshot_path.into();
        let new_mem_file_path = new_mem_file_path.into();

        futures_util::try_join!(
            runtime.fs_copy(&self.snapshot_path, &new_snapshot_path),
            runtime.fs_copy(&self.mem_file_path, &new_mem_file_path)
        )
        .map_err(|err| ResourceSystemError::FilesystemError(Arc::new(err)))?;

        self.snapshot_path = new_snapshot_path;
        self.mem_file_path = new_mem_file_path;
        Ok(())
    }

    pub async fn prepare_vm<E: VmmExecutor, S: ProcessSpawner, R: Runtime>(
        self,
        old_vm: &mut Vm<E, S, R>,
        options: PrepareVmFromSnapshotOptions<E, S, R>,
    ) -> Result<Vm<E, S, R>, VmError> {
        let mut resource_system =
            ResourceSystem::new(options.process_spawner, options.runtime, options.ownership_model);

        let mem_file = resource_system
            .create_resource(self.mem_file_path, ResourceType::Moved(options.moved_resource_type))
            .map_err(VmError::ResourceSystemError)?;
        let snapshot = resource_system
            .create_resource(self.snapshot_path, ResourceType::Moved(options.moved_resource_type))
            .map_err(VmError::ResourceSystemError)?;

        for resource in old_vm.get_resource_system().get_resources() {
            if let ResourceType::Moved(_) = resource.get_type() {
                let resource_path = resource.get_effective_path().ok_or_else(|| {
                    VmError::ResourceSystemError(ResourceSystemError::IncorrectState(ResourceState::Uninitialized))
                })?;

                resource_system
                    .create_resource(resource_path, ResourceType::Moved(options.moved_resource_type))
                    .map_err(VmError::ResourceSystemError)?;
            }
        }

        let load_snapshot = LoadSnapshot {
            enable_diff_snapshots: options.enable_diff_snapshots,
            mem_backend: MemoryBackend {
                backend_type: MemoryBackendType::File,
                backend: mem_file,
            },
            snapshot,
            resume_vm: options.resume_vm,
        };

        let configuration = VmConfiguration::RestoredFromSnapshot {
            load_snapshot,
            data: self.configuration_data,
        };

        Vm::prepare(
            options.executor,
            resource_system,
            old_vm.vmm_process.installation.clone(),
            configuration,
        )
        .await
    }
}
