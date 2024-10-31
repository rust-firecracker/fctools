use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
};

use nix::unistd::{Gid, Uid};
use tokio::task::JoinSet;

use crate::{
    fs_backend::FsBackend,
    process_spawner::ProcessSpawner,
    vmm::{
        arguments::{command_modifier::CommandModifier, VmmApiSocket, VmmArguments},
        id::VmmId,
        installation::VmmInstallation,
    },
};

use super::{
    create_file_with_tree, join_on_set, process_handle::ProcessHandle, upgrade_owner, VmmExecutor, VmmExecutorError,
    VmmOwnershipModel,
};

/// A [VmmExecutor] that uses the "firecracker" binary directly, without jailing it or ensuring it doesn't run as root.
/// This [VmmExecutor] allows rootless execution, given that the user has been granted access to /dev/kvm, but using
/// this "direct" mode of execution is not recommended by Firecracker developers in production scenarios.
#[derive(Debug)]
pub struct UnrestrictedVmmExecutor {
    vmm_arguments: VmmArguments,
    command_modifier_chain: Vec<Box<dyn CommandModifier>>,
    remove_metrics_on_cleanup: bool,
    remove_logs_on_cleanup: bool,
    pipes_to_null: bool,
    id: Option<VmmId>,
    ownership_downgrade: Option<(Uid, Gid)>,
}

impl UnrestrictedVmmExecutor {
    pub fn new(vmm_arguments: VmmArguments) -> Self {
        Self {
            vmm_arguments,
            command_modifier_chain: Vec::new(),
            remove_metrics_on_cleanup: false,
            remove_logs_on_cleanup: false,
            pipes_to_null: false,
            id: None,
            ownership_downgrade: None,
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

    pub fn remove_metrics_on_cleanup(mut self) -> Self {
        self.remove_metrics_on_cleanup = true;
        self
    }

    pub fn remove_logs_on_cleanup(mut self) -> Self {
        self.remove_logs_on_cleanup = true;
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

    pub fn downgrade_ownership(mut self, uid: Uid, gid: Gid) -> Self {
        self.ownership_downgrade = Some((uid, gid));
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

    async fn prepare(
        &self,
        _installation: &VmmInstallation,
        process_spawner: Arc<impl ProcessSpawner>,
        fs_backend: Arc<impl FsBackend>,
        outer_paths: Vec<PathBuf>,
        ownership_model: VmmOwnershipModel,
    ) -> Result<HashMap<PathBuf, PathBuf>, VmmExecutorError> {
        let mut join_set = JoinSet::new();

        for path in outer_paths.clone() {
            let fs_backend = fs_backend.clone();
            let process_spawner = process_spawner.clone();

            join_set.spawn(async move {
                upgrade_owner(&path, ownership_model, process_spawner.as_ref(), fs_backend.as_ref())
                    .await
                    .map_err(VmmExecutorError::ChangeOwnerError)?;

                if !fs_backend
                    .check_exists(&path)
                    .await
                    .map_err(VmmExecutorError::FsBackendError)?
                {
                    return Err(VmmExecutorError::ExpectedResourceMissing(path));
                }

                Ok(())
            });
        }

        if let VmmApiSocket::Enabled(socket_path) = self.vmm_arguments.api_socket.clone() {
            let fs_backend = fs_backend.clone();
            let process_spawner = process_spawner.clone();

            join_set.spawn(async move {
                upgrade_owner(
                    &socket_path,
                    ownership_model,
                    process_spawner.as_ref(),
                    fs_backend.as_ref(),
                )
                .await
                .map_err(VmmExecutorError::ChangeOwnerError)?;

                if fs_backend
                    .check_exists(&socket_path)
                    .await
                    .map_err(VmmExecutorError::FsBackendError)?
                {
                    fs_backend
                        .remove_file(&socket_path)
                        .await
                        .map_err(VmmExecutorError::FsBackendError)?;
                }

                Ok(())
            });
        }

        // Ensure argument paths exist
        if let Some(log_path) = self.vmm_arguments.log_path.clone() {
            join_set.spawn(create_file_with_tree(
                fs_backend.clone(),
                process_spawner.clone(),
                ownership_model,
                log_path,
            ));
        }

        if let Some(metrics_path) = self.vmm_arguments.metrics_path.clone() {
            join_set.spawn(create_file_with_tree(
                fs_backend.clone(),
                process_spawner.clone(),
                ownership_model,
                metrics_path,
            ));
        }

        join_on_set(join_set).await?;
        Ok(outer_paths.into_iter().map(|path| (path.clone(), path)).collect())
    }

    async fn invoke(
        &self,
        installation: &VmmInstallation,
        process_spawner: Arc<impl ProcessSpawner>,
        _fs_backend: Arc<impl FsBackend>,
        config_path: Option<PathBuf>,
        _ownership_model: VmmOwnershipModel,
    ) -> Result<ProcessHandle, VmmExecutorError> {
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
            .spawn(&binary_path, arguments, self.pipes_to_null)
            .await
            .map_err(VmmExecutorError::ProcessSpawnFailed)?;
        Ok(ProcessHandle::attached(child, self.pipes_to_null))
    }

    async fn cleanup(
        &self,
        _installation: &VmmInstallation,
        process_spawner: Arc<impl ProcessSpawner>,
        fs_backend: Arc<impl FsBackend>,
        ownership_model: VmmOwnershipModel,
    ) -> Result<(), VmmExecutorError> {
        let mut join_set: JoinSet<Result<(), VmmExecutorError>> = JoinSet::new();

        if let VmmApiSocket::Enabled(socket_path) = self.vmm_arguments.api_socket.clone() {
            let process_spawner = process_spawner.clone();
            let fs_backend = fs_backend.clone();

            join_set.spawn(async move {
                upgrade_owner(
                    &socket_path,
                    ownership_model,
                    process_spawner.as_ref(),
                    fs_backend.as_ref(),
                )
                .await
                .map_err(VmmExecutorError::ChangeOwnerError)?;

                if fs_backend
                    .check_exists(&socket_path)
                    .await
                    .map_err(VmmExecutorError::FsBackendError)?
                {
                    fs_backend
                        .remove_file(&socket_path)
                        .await
                        .map_err(VmmExecutorError::FsBackendError)?;
                }

                Ok(())
            });
        }

        if self.remove_logs_on_cleanup {
            if let Some(log_path) = self.vmm_arguments.log_path.clone() {
                let fs_backend = fs_backend.clone();
                let process_spawner = process_spawner.clone();

                join_set.spawn(async move {
                    upgrade_owner(
                        &log_path,
                        ownership_model,
                        process_spawner.as_ref(),
                        fs_backend.as_ref(),
                    )
                    .await
                    .map_err(VmmExecutorError::ChangeOwnerError)?;

                    fs_backend
                        .remove_file(&log_path)
                        .await
                        .map_err(VmmExecutorError::FsBackendError)
                });
            }
        }

        if self.remove_metrics_on_cleanup {
            if let Some(metrics_path) = self.vmm_arguments.metrics_path.clone() {
                let fs_backend = fs_backend.clone();

                join_set.spawn(async move {
                    upgrade_owner(
                        &metrics_path,
                        ownership_model,
                        process_spawner.as_ref(),
                        fs_backend.as_ref(),
                    )
                    .await
                    .map_err(VmmExecutorError::ChangeOwnerError)?;

                    fs_backend
                        .remove_file(&metrics_path)
                        .await
                        .map_err(VmmExecutorError::FsBackendError)
                });
            }
        }

        join_on_set(join_set).await
    }
}
