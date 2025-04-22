use std::path::PathBuf;

use crate::{
    process_spawner::ProcessSpawner,
    runtime::Runtime,
    vmm::{
        arguments::{command_modifier::CommandModifier, VmmApiSocket, VmmArguments},
        id::VmmId,
        installation::VmmInstallation,
        ownership::upgrade_owner,
        resource::ResourceType,
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
    /// Create a new [UnrestrictedVmmExecutor] from a [VmmArguments] instance.
    pub fn new(vmm_arguments: VmmArguments) -> Self {
        Self {
            vmm_arguments,
            command_modifier_chain: Vec::new(),
            pipes_to_null: false,
            id: None,
        }
    }

    /// Add a [CommandModifier] implementation to the end of the [CommandModifier] chain.
    pub fn command_modifier<C: CommandModifier>(mut self, command_modifier: C) -> Self {
        self.command_modifier_chain.push(Box::new(command_modifier));
        self
    }

    /// Sequentially insert an iterator of boxed [CommandModifier]s to the end of the [CommandModifier] chain.
    pub fn command_modifiers<I: IntoIterator<Item = Box<dyn CommandModifier>>>(mut self, command_modifiers: I) -> Self {
        self.command_modifier_chain.extend(command_modifiers);
        self
    }

    /// Configure the [UnrestrictedVmmExecutor] to set the pipes of the [ProcessHandle]'s process to null, meaning
    /// that they won't be accessible via a [ProcessHandle::get_pipes] call.
    pub fn pipes_to_null(mut self) -> Self {
        self.pipes_to_null = true;
        self
    }

    /// Set an optional [VmmId] for Firecracker to use. If not specified, a default value decided on by Firecracker itself
    /// will be used instead.
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

    fn resolve_effective_path(&self, _installation: &VmmInstallation, local_path: PathBuf) -> PathBuf {
        local_path
    }

    async fn prepare<S: ProcessSpawner, R: Runtime>(
        &self,
        context: VmmExecutorContext<S, R>,
    ) -> Result<(), VmmExecutorError> {
        if let VmmApiSocket::Enabled(socket_path) = self.vmm_arguments.api_socket.clone() {
            let process_spawner = context.process_spawner.clone();
            let ownership_model = context.ownership_model;
            let runtime = context.runtime.clone();

            upgrade_owner(&socket_path, ownership_model, &process_spawner, &runtime)
                .await
                .map_err(VmmExecutorError::ChangeOwnerError)?;

            if runtime
                .fs_exists(&socket_path)
                .await
                .map_err(VmmExecutorError::FilesystemError)?
            {
                runtime
                    .fs_remove_file(&socket_path)
                    .await
                    .map_err(VmmExecutorError::FilesystemError)?;
            }
        }

        for mut resource in context.resources.chain(self.vmm_arguments.get_resources()) {
            resource
                .start_initialization_with_same_path()
                .map_err(VmmExecutorError::ResourceSystemError)?;
        }

        Ok(())
    }

    async fn invoke<S: ProcessSpawner, R: Runtime>(
        &self,
        context: VmmExecutorContext<S, R>,
        config_path: Option<PathBuf>,
    ) -> Result<ProcessHandle<R>, VmmExecutorError> {
        let mut arguments = self.vmm_arguments.join(config_path);
        let mut binary_path = context.installation.get_firecracker_path().to_owned();

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
        Ok(ProcessHandle::from_child(child, self.pipes_to_null))
    }

    async fn cleanup<S: ProcessSpawner, R: Runtime>(
        &self,
        context: VmmExecutorContext<S, R>,
    ) -> Result<(), VmmExecutorError> {
        if let VmmApiSocket::Enabled(socket_path) = self.vmm_arguments.api_socket.clone() {
            let process_spawner = context.process_spawner.clone();
            let runtime = context.runtime.clone();
            let ownership_model = context.ownership_model;

            upgrade_owner(&socket_path, ownership_model, &process_spawner, &runtime)
                .await
                .map_err(VmmExecutorError::ChangeOwnerError)?;

            if runtime
                .fs_exists(&socket_path)
                .await
                .map_err(VmmExecutorError::FilesystemError)?
            {
                runtime
                    .fs_remove_file(&socket_path)
                    .await
                    .map_err(VmmExecutorError::FilesystemError)?;
            }
        }

        for mut resource in context.resources.chain(self.vmm_arguments.get_resources()) {
            if !matches!(resource.get_type(), ResourceType::Moved(_)) {
                resource
                    .start_disposal()
                    .map_err(VmmExecutorError::ResourceSystemError)?;
            }
        }

        Ok(())
    }
}
