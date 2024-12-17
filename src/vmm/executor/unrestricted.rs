use std::path::PathBuf;

use futures_util::TryFutureExt;

use crate::{
    process_spawner::ProcessSpawner,
    runtime::{Runtime, RuntimeFilesystem, RuntimeJoinSet},
    vmm::{
        arguments::{command_modifier::CommandModifier, VmmApiSocket, VmmArguments},
        id::VmmId,
        installation::VmmInstallation,
        ownership::upgrade_owner,
        resource::VmmResourceReferences,
    },
};

use super::{process_handle::ProcessHandle, VmmExecutor, VmmExecutorContext, VmmExecutorError};

/// A [VmmExecutor] that uses the "firecracker" binary directly, without jailing it or ensuring it doesn't run as root.
/// This [VmmExecutor] allows rootless execution, given that the user has been granted access to /dev/kvm, but using
/// this "direct" mode of execution is not recommended by Firecracker developers in production scenarios.
#[derive(Debug)]
pub struct UnrestrictedVmmExecutor {
    vmm_arguments: VmmArguments,
    command_modifier_chain: Vec<Box<dyn CommandModifier>>,
    pipes_to_null: bool,
    id: Option<VmmId>,
}

impl UnrestrictedVmmExecutor {
    pub fn new(vmm_arguments: VmmArguments) -> Self {
        Self {
            vmm_arguments,
            command_modifier_chain: Vec::new(),
            pipes_to_null: false,
            id: None,
        }
    }

    pub fn command_modifier(mut self, command_modifier: impl CommandModifier + 'static) -> Self {
        self.command_modifier_chain.push(Box::new(command_modifier));
        self
    }

    pub fn command_modifiers(mut self, command_modifiers: impl IntoIterator<Item = Box<dyn CommandModifier>>) -> Self {
        self.command_modifier_chain.extend(command_modifiers);
        self
    }

    pub fn pipes_to_null(mut self) -> Self {
        self.pipes_to_null = true;
        self
    }

    pub fn id(mut self, id: VmmId) -> Self {
        self.id = Some(id);
        self
    }
}

impl VmmExecutor for UnrestrictedVmmExecutor {
    fn get_socket_path(&self, _installation: &VmmInstallation) -> Option<PathBuf> {
        match &self.vmm_arguments.api_socket {
            VmmApiSocket::Disabled => None,
            VmmApiSocket::Enabled(path) => Some(path.clone()),
        }
    }

    fn local_to_effective_path(&self, _installation: &VmmInstallation, local_path: PathBuf) -> PathBuf {
        local_path
    }

    async fn prepare<S: ProcessSpawner, R: Runtime>(
        &mut self,
        context: VmmExecutorContext<S, R>,
        mut resource_references: VmmResourceReferences<'_>,
    ) -> Result<(), VmmExecutorError> {
        let mut join_set = RuntimeJoinSet::new(context.runtime.clone());

        // Apply moved resources
        for moved_resource in resource_references.moved_resources {
            join_set.spawn(
                moved_resource
                    .initialize_with_same_path(
                        context.ownership_model,
                        context.process_spawner.clone(),
                        context.runtime.clone(),
                    )
                    .map_err(VmmExecutorError::ResourceError),
            );
        }

        if let VmmApiSocket::Enabled(socket_path) = self.vmm_arguments.api_socket.clone() {
            let process_spawner = context.process_spawner.clone();
            let ownership_model = context.ownership_model;
            let runtime = context.runtime.clone();

            join_set.spawn(async move {
                upgrade_owner(&socket_path, ownership_model, &process_spawner, &runtime)
                    .await
                    .map_err(VmmExecutorError::ChangeOwnerError)?;

                if runtime
                    .filesystem()
                    .check_exists(&socket_path)
                    .await
                    .map_err(VmmExecutorError::FilesystemError)?
                {
                    runtime
                        .filesystem()
                        .remove_file(&socket_path)
                        .await
                        .map_err(VmmExecutorError::FilesystemError)?;
                }

                Ok(())
            });
        }

        // Apply created resources
        if let Some(ref mut logs) = self.vmm_arguments.logs {
            resource_references.created_resources.push(logs);
        }

        if let Some(ref mut metrics) = self.vmm_arguments.metrics {
            resource_references.created_resources.push(metrics);
        }

        for created_resource in resource_references.created_resources {
            join_set.spawn(
                created_resource
                    .initialize_with_same_path::<R>(context.ownership_model, context.runtime.clone())
                    .map_err(VmmExecutorError::ResourceError),
            );
        }

        for produced_resource in resource_references.produced_resources {
            join_set.spawn(
                produced_resource
                    .initialize_with_same_path::<R>(context.ownership_model, context.runtime.clone())
                    .map_err(VmmExecutorError::ResourceError),
            );
        }

        join_set.wait().await.unwrap_or(Err(VmmExecutorError::TaskJoinFailed))?;
        Ok(())
    }

    async fn invoke<S: ProcessSpawner, R: Runtime>(
        &mut self,
        context: VmmExecutorContext<S, R>,
        config_path: Option<PathBuf>,
    ) -> Result<ProcessHandle<R::Process>, VmmExecutorError> {
        let mut arguments = self.vmm_arguments.join(config_path);
        let mut binary_path = context.installation.firecracker_path.clone();

        for command_modifier in &self.command_modifier_chain {
            command_modifier.apply(&mut binary_path, &mut arguments);
        }

        if let Some(ref id) = self.id {
            arguments.push("--id".to_string());
            arguments.push(id.as_ref().to_owned());
        }

        let child = context
            .process_spawner
            .spawn(&binary_path, arguments, self.pipes_to_null, &context.runtime)
            .await
            .map_err(VmmExecutorError::ProcessSpawnFailed)?;
        Ok(ProcessHandle::with_child(child, self.pipes_to_null))
    }

    async fn cleanup<S: ProcessSpawner, R: Runtime>(
        &mut self,
        context: VmmExecutorContext<S, R>,
        mut resource_references: VmmResourceReferences<'_>,
    ) -> Result<(), VmmExecutorError> {
        let mut join_set = RuntimeJoinSet::new(context.runtime.clone());

        if let VmmApiSocket::Enabled(socket_path) = self.vmm_arguments.api_socket.clone() {
            let process_spawner = context.process_spawner.clone();
            let runtime = context.runtime.clone();
            let ownership_model = context.ownership_model;

            join_set.spawn(async move {
                upgrade_owner(&socket_path, ownership_model, &process_spawner, &runtime)
                    .await
                    .map_err(VmmExecutorError::ChangeOwnerError)?;

                if runtime
                    .filesystem()
                    .check_exists(&socket_path)
                    .await
                    .map_err(VmmExecutorError::FilesystemError)?
                {
                    runtime
                        .filesystem()
                        .remove_file(&socket_path)
                        .await
                        .map_err(VmmExecutorError::FilesystemError)?;
                }

                Ok(())
            });
        }

        if let Some(ref mut logs) = self.vmm_arguments.logs {
            resource_references.created_resources.push(logs);
        }

        if let Some(ref mut metrics) = self.vmm_arguments.metrics {
            resource_references.created_resources.push(metrics);
        }

        for created_resource in resource_references.created_resources {
            join_set.spawn(
                created_resource
                    .dispose(
                        context.ownership_model,
                        context.process_spawner.clone(),
                        context.runtime.clone(),
                    )
                    .map_err(VmmExecutorError::ResourceError),
            );
        }

        for produced_resource in resource_references.produced_resources {
            join_set.spawn(
                produced_resource
                    .dispose(
                        context.ownership_model,
                        context.process_spawner.clone(),
                        context.runtime.clone(),
                    )
                    .map_err(VmmExecutorError::ResourceError),
            );
        }

        join_set.wait().await.unwrap_or(Err(VmmExecutorError::TaskJoinFailed))
    }
}
