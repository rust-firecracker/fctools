use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
};

use tokio::process::Child;

use crate::{
    fs_backend::FsBackend,
    process_spawner::ProcessSpawner,
    vmm::{installation::VmmInstallation, ownership::VmmOwnershipModel},
};

use super::{
    jailed::{JailRenamer, JailedVmmExecutor},
    unrestricted::UnrestrictedVmmExecutor,
    VmmExecutor, VmmExecutorError,
};

/// [EitherVmmExecutor] encapsulates either an unrestricted or a jailed executor with the
/// given [JailRenamer] behind an enum with [VmmExecutor] implemented. fctools was specifically
/// designed against heap allocation and dynamic dispatch, so this is a statically dispatched
/// enum instead.
pub enum EitherVmmExecutor<R: JailRenamer + 'static> {
    Unrestricted(UnrestrictedVmmExecutor),
    Jailed(JailedVmmExecutor<R>),
}

impl<R: JailRenamer + 'static> From<UnrestrictedVmmExecutor> for EitherVmmExecutor<R> {
    fn from(value: UnrestrictedVmmExecutor) -> Self {
        EitherVmmExecutor::Unrestricted(value)
    }
}

impl<R: JailRenamer + 'static> From<JailedVmmExecutor<R>> for EitherVmmExecutor<R> {
    fn from(value: JailedVmmExecutor<R>) -> Self {
        EitherVmmExecutor::Jailed(value)
    }
}

impl<R: JailRenamer + 'static> VmmExecutor for EitherVmmExecutor<R> {
    fn get_socket_path(&self, installation: &VmmInstallation) -> Option<PathBuf> {
        match self {
            EitherVmmExecutor::Unrestricted(executor) => executor.get_socket_path(installation),
            EitherVmmExecutor::Jailed(executor) => executor.get_socket_path(installation),
        }
    }

    fn inner_to_outer_path(&self, installation: &VmmInstallation, inner_path: &Path) -> PathBuf {
        match self {
            EitherVmmExecutor::Unrestricted(executor) => executor.inner_to_outer_path(installation, inner_path),
            EitherVmmExecutor::Jailed(executor) => executor.inner_to_outer_path(installation, inner_path),
        }
    }

    fn is_traceless(&self) -> bool {
        match self {
            EitherVmmExecutor::Unrestricted(executor) => executor.is_traceless(),
            EitherVmmExecutor::Jailed(executor) => executor.is_traceless(),
        }
    }

    async fn prepare(
        &self,
        installation: &VmmInstallation,
        process_spawner: Arc<impl ProcessSpawner>,
        fs_backend: Arc<impl FsBackend>,
        outer_paths: Vec<PathBuf>,
        ownership_model: VmmOwnershipModel,
    ) -> Result<HashMap<PathBuf, PathBuf>, VmmExecutorError> {
        match self {
            EitherVmmExecutor::Unrestricted(executor) => {
                executor
                    .prepare(installation, process_spawner, fs_backend, outer_paths, ownership_model)
                    .await
            }
            EitherVmmExecutor::Jailed(executor) => {
                executor
                    .prepare(installation, process_spawner, fs_backend, outer_paths, ownership_model)
                    .await
            }
        }
    }

    async fn invoke(
        &self,
        installation: &VmmInstallation,
        process_spawner: Arc<impl ProcessSpawner>,
        config_path: Option<PathBuf>,
        ownership_model: VmmOwnershipModel,
    ) -> Result<Child, VmmExecutorError> {
        match self {
            EitherVmmExecutor::Unrestricted(executor) => {
                executor
                    .invoke(installation, process_spawner, config_path, ownership_model)
                    .await
            }
            EitherVmmExecutor::Jailed(executor) => {
                executor
                    .invoke(installation, process_spawner, config_path, ownership_model)
                    .await
            }
        }
    }

    async fn cleanup(
        &self,
        installation: &VmmInstallation,
        process_spawner: Arc<impl ProcessSpawner>,
        fs_backend: Arc<impl FsBackend>,
        ownership_model: VmmOwnershipModel,
    ) -> Result<(), VmmExecutorError> {
        match self {
            EitherVmmExecutor::Unrestricted(executor) => {
                executor
                    .cleanup(installation, process_spawner, fs_backend, ownership_model)
                    .await
            }
            EitherVmmExecutor::Jailed(executor) => {
                executor
                    .cleanup(installation, process_spawner, fs_backend, ownership_model)
                    .await
            }
        }
    }
}
