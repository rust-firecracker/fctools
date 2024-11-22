use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use futures_util::TryFutureExt;

use crate::{
    process_spawner::ProcessSpawner,
    runtime::{Runtime, RuntimeFilesystem, RuntimeJoinSet},
    vmm::{
        arguments::{command_modifier::CommandModifier, VmmApiSocket, VmmArguments},
        id::VmmId,
        installation::VmmInstallation,
        resource::MovedVmmResource,
    },
};

use super::{process_handle::ProcessHandle, upgrade_owner, VmmExecutor, VmmExecutorError, VmmOwnershipModel};

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

    fn inner_to_outer_path(&self, _installation: &VmmInstallation, inner_path: &Path) -> PathBuf {
        inner_path.to_owned()
    }

    fn is_traceless(&self) -> bool {
        false
    }

    async fn prepare<R: Runtime>(
        &self,
        _installation: &VmmInstallation,
        process_spawner: Arc<impl ProcessSpawner>,
        moved_resources: Vec<&mut MovedVmmResource>,
        ownership_model: VmmOwnershipModel,
    ) -> Result<(), VmmExecutorError> {
        let mut join_set: RuntimeJoinSet<_, R::Executor> = RuntimeJoinSet::new();

        for moved_resource in moved_resources {
            join_set.spawn(
                moved_resource
                    .apply_with_same_path::<R>(ownership_model, process_spawner.clone())
                    .map_err(VmmExecutorError::ResourceError),
            );
        }

        if let VmmApiSocket::Enabled(socket_path) = self.vmm_arguments.api_socket.clone() {
            let process_spawner = process_spawner.clone();

            join_set.spawn(async move {
                upgrade_owner::<R>(&socket_path, ownership_model, process_spawner.as_ref())
                    .await
                    .map_err(VmmExecutorError::ChangeOwnerError)?;

                if R::Filesystem::check_exists(&socket_path)
                    .await
                    .map_err(VmmExecutorError::FilesystemError)?
                {
                    R::Filesystem::remove_file(&socket_path)
                        .await
                        .map_err(VmmExecutorError::FilesystemError)?;
                }

                Ok(())
            });
        }

        // Ensure argument paths exist
        if let Some(ref mut logs) = self.vmm_arguments.logs {
            join_set.spawn(
                logs.apply_with_same_path::<R>(ownership_model)
                    .map_err(VmmExecutorError::ResourceError),
            );
        }

        if let Some(ref mut metrics) = self.vmm_arguments.metrics {
            join_set.spawn(
                metrics
                    .apply_with_same_path::<R>(ownership_model)
                    .map_err(VmmExecutorError::ResourceError),
            );
        }

        join_set.wait().await.unwrap_or(Err(VmmExecutorError::TaskJoinFailed))?;
        Ok(())
    }

    async fn invoke<R: Runtime>(
        &self,
        installation: &VmmInstallation,
        process_spawner: Arc<impl ProcessSpawner>,
        config_path: Option<PathBuf>,
        _ownership_model: VmmOwnershipModel,
    ) -> Result<ProcessHandle<R::Process>, VmmExecutorError> {
        let mut arguments = self.vmm_arguments.join(config_path);
        let mut binary_path = installation.firecracker_path.clone();

        for command_modifier in &self.command_modifier_chain {
            command_modifier.apply(&mut binary_path, &mut arguments);
        }

        if let Some(ref id) = self.id {
            arguments.push("--id".to_string());
            arguments.push(id.as_ref().to_owned());
        }

        let child = process_spawner
            .spawn::<R>(&binary_path, arguments, self.pipes_to_null)
            .await
            .map_err(VmmExecutorError::ProcessSpawnFailed)?;
        Ok(ProcessHandle::with_child(child, self.pipes_to_null))
    }

    async fn cleanup<R: Runtime>(
        &self,
        _installation: &VmmInstallation,
        process_spawner: Arc<impl ProcessSpawner>,
        ownership_model: VmmOwnershipModel,
    ) -> Result<(), VmmExecutorError> {
        let mut join_set: RuntimeJoinSet<_, R::Executor> = RuntimeJoinSet::new();

        if let VmmApiSocket::Enabled(socket_path) = self.vmm_arguments.api_socket.clone() {
            let process_spawner = process_spawner.clone();

            join_set.spawn(async move {
                upgrade_owner::<R>(&socket_path, ownership_model, process_spawner.as_ref())
                    .await
                    .map_err(VmmExecutorError::ChangeOwnerError)?;

                if R::Filesystem::check_exists(&socket_path)
                    .await
                    .map_err(VmmExecutorError::FilesystemError)?
                {
                    R::Filesystem::remove_file(&socket_path)
                        .await
                        .map_err(VmmExecutorError::FilesystemError)?;
                }

                Ok(())
            });
        }

        if let Some(ref logs) = self.vmm_arguments.logs {
            let process_spawner = process_spawner.clone();

            join_set.spawn(async move {
                upgrade_owner::<R>(&log_path, ownership_model, process_spawner.as_ref())
                    .await
                    .map_err(VmmExecutorError::ChangeOwnerError)?;

                R::Filesystem::remove_file(&log_path)
                    .await
                    .map_err(VmmExecutorError::FilesystemError)
            });
        }

        if self.remove_metrics_on_cleanup {
            if let Some(metrics_path) = self.vmm_arguments.metrics.clone() {
                join_set.spawn(async move {
                    upgrade_owner::<R>(&metrics_path, ownership_model, process_spawner.as_ref())
                        .await
                        .map_err(VmmExecutorError::ChangeOwnerError)?;

                    R::Filesystem::remove_file(&metrics_path)
                        .await
                        .map_err(VmmExecutorError::FilesystemError)
                });
            }
        }

        join_set.wait().await.unwrap_or(Err(VmmExecutorError::TaskJoinFailed))
    }
}
