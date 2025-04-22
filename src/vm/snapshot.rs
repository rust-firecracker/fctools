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

/// The data necessary to prepare a [Vm] from a [VmSnapshot].
#[derive(Debug)]
pub struct PrepareVmFromSnapshotOptions<E: VmmExecutor, S: ProcessSpawner, R: Runtime> {
    /// The [VmmExecutor] for the new [Vm].
    pub executor: E,
    /// The [ProcessSpawner] for the new [Vm].
    pub process_spawner: S,
    /// The [Runtime] for the new [Vm].
    pub runtime: R,
    /// The [MovedResourceType] to assign to all resources moved from the old to the new VM.
    pub moved_resource_type: MovedResourceType,
    /// The [VmmOwnershipModel] of the new [Vm].
    pub ownership_model: VmmOwnershipModel,
    /// Optionally, whether to enable diff snapshots when restoring the new VM.
    pub enable_diff_snapshots: Option<bool>,
    /// Optionally, whether to resume the new VM immediately.
    pub resume_vm: Option<bool>,
}

impl VmSnapshot {
    /// Copy the snapshot and memory files of this [VmSnapshot] to new locations via the provided [Runtime].
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

    /// A helper that automates the most common cases of preparing a new [Vm] from a [VmSnapshot] using
    /// the options supported in [PrepareVmFromSnapshotOptions]. Everything done internally by this function
    /// is public, so custom alternatives that take care of more advanced cases are possible and encouraged.
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

        for mut resource in old_vm.get_resource_system().get_resources() {
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
