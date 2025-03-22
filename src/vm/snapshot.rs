use std::path::PathBuf;

use crate::{
    process_spawner::ProcessSpawner,
    runtime::Runtime,
    vm::models::{LoadSnapshot, MemoryBackend, MemoryBackendType},
    vmm::{
        executor::VmmExecutor,
        ownership::VmmOwnershipModel,
        resource::{
            system::{ResourceSystem, ResourceSystemError},
            template::ResourceTemplate,
            MovedResourceType, ResourceState, ResourceType,
        },
    },
};

use super::{
    configuration::{VmConfiguration, VmConfigurationData},
    Vm, VmError,
};

/// The data associated with a snapshot created for a [Vm](crate::vm::Vm).
#[derive(Debug, Clone)]
pub struct VmSnapshot {
    pub snapshot: ResourceTemplate,
    pub mem_file: ResourceTemplate,
    pub configuration_data: VmConfigurationData,
}

#[derive(Debug)]
pub struct PrepareVmFromSnapshotOptions<E: VmmExecutor, S: ProcessSpawner, R: Runtime> {
    pub executor: E,
    pub process_spawner: S,
    pub runtime: R,
    pub moved_resource_type: MovedResourceType,
    pub ownership_model: VmmOwnershipModel,
    pub detach_resources_from_old_vm: bool,
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

    pub async fn prepare_vm<E: VmmExecutor, S: ProcessSpawner, R: Runtime>(
        mut self,
        old_vm: &mut Vm<E, S, R>,
        options: PrepareVmFromSnapshotOptions<E, S, R>,
    ) -> Result<Vm<E, S, R>, VmError> {
        if !self
            .mem_file
            .deinitialize(ResourceType::Moved(options.moved_resource_type))
            || !self
                .snapshot
                .deinitialize(ResourceType::Moved(options.moved_resource_type))
        {
            return Err(VmError::ResourceSystemError(ResourceSystemError::IncorrectState(
                ResourceState::Uninitialized,
            )));
        }

        let mut resource_system =
            ResourceSystem::new(options.process_spawner, options.runtime, options.ownership_model);

        let mem_file = resource_system
            .create_resource_from_template(self.mem_file)
            .map_err(VmError::ResourceSystemError)?;
        let snapshot = resource_system
            .create_resource_from_template(self.snapshot)
            .map_err(VmError::ResourceSystemError)?;

        for resource in old_vm.resource_system().get_resources() {
            if let ResourceType::Moved(_) = resource.get_type() {
                let mut template = match options.detach_resources_from_old_vm {
                    true => resource.as_template(),
                    false => resource
                        .clone()
                        .into_template()
                        .map_err(|(_, err)| VmError::ResourceSystemError(err))?,
                };

                if !template.deinitialize(ResourceType::Moved(options.moved_resource_type)) {
                    continue;
                }

                resource_system
                    .create_resource_from_template(template)
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
