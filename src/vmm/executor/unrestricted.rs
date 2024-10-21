use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
};

use tokio::{process::Child, task::JoinSet};

use crate::{
    fs_backend::FsBackend,
    process_spawner::ProcessSpawner,
    vmm::{
        arguments::{command_modifier::CommandModifier, VmmApiSocket, VmmArguments, VmmConfigurationOverride},
        id::VmmId,
        installation::VmmInstallation,
    },
};

use super::{
    change_owner, create_file_with_tree, join_on_set, VmmExecutor, VmmExecutorError, PROCESS_GID, PROCESS_UID,
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
    downgrade: Option<(u32, u32)>,
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
            downgrade: None,
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

    pub fn downgrade_ownership(mut self, uid: u32, gid: u32) -> Self {
        self.downgrade = Some((uid, gid));
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

    fn ownership_downgrade(&self) -> Option<(u32, u32)> {
        self.downgrade
    }

    async fn prepare(
        &self,
        _installation: &VmmInstallation,
        process_spawner: Arc<impl ProcessSpawner>,
        fs_backend: Arc<impl FsBackend>,
        outer_paths: Vec<PathBuf>,
    ) -> Result<HashMap<PathBuf, PathBuf>, VmmExecutorError> {
        let mut join_set = JoinSet::new();

        for path in outer_paths.clone() {
            let fs_backend = fs_backend.clone();
            let process_spawner = process_spawner.clone();

            join_set.spawn(async move {
                if process_spawner.upgrades_ownership() {
                    change_owner(&path, *PROCESS_UID, *PROCESS_GID, process_spawner.as_ref()).await?;
                }

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
                if process_spawner.upgrades_ownership() {
                    change_owner(&socket_path, *PROCESS_UID, *PROCESS_GID, process_spawner.as_ref()).await?;
                }

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
                self.downgrade,
                log_path,
            ));
        }

        if let Some(metrics_path) = self.vmm_arguments.metrics_path.clone() {
            join_set.spawn(create_file_with_tree(
                fs_backend.clone(),
                process_spawner.clone(),
                self.downgrade,
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
        configuration_override: VmmConfigurationOverride,
    ) -> Result<Child, VmmExecutorError> {
        let mut arguments = self.vmm_arguments.join(configuration_override);
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
        Ok(child)
    }

    async fn cleanup(
        &self,
        _installation: &VmmInstallation,
        process_spawner: Arc<impl ProcessSpawner>,
        fs_backend: Arc<impl FsBackend>,
    ) -> Result<(), VmmExecutorError> {
        let mut join_set: JoinSet<Result<(), VmmExecutorError>> = JoinSet::new();

        if let VmmApiSocket::Enabled(socket_path) = self.vmm_arguments.api_socket.clone() {
            let process_spawner = process_spawner.clone();
            let fs_backend = fs_backend.clone();

            join_set.spawn(async move {
                if process_spawner.upgrades_ownership() {
                    change_owner(&socket_path, *PROCESS_UID, *PROCESS_GID, process_spawner.as_ref()).await?;
                }

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
            if let Some(ref log_path) = self.vmm_arguments.log_path {
                let fs_backend = fs_backend.clone();
                let log_path = log_path.clone();
                join_set.spawn(async move {
                    fs_backend
                        .remove_file(&log_path)
                        .await
                        .map_err(VmmExecutorError::FsBackendError)
                });
            }
        }

        if self.remove_metrics_on_cleanup {
            if let Some(ref metrics_path) = self.vmm_arguments.metrics_path {
                let fs_backend = fs_backend.clone();
                let metrics_path = metrics_path.clone();
                join_set.spawn(async move {
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
