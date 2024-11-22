use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use crate::{
    process_spawner::ProcessSpawner,
    runtime::Runtime,
    vmm::{installation::VmmInstallation, ownership::VmmOwnershipModel, resource::MovedVmmResource},
};

use super::{
    jailed::{JailRenamer, JailedVmmExecutor},
    process_handle::ProcessHandle,
    unrestricted::UnrestrictedVmmExecutor,
    VmmExecutor, VmmExecutorError,
};

/// [EitherVmmExecutor] encapsulates either an unrestricted or a jailed executor with the
/// given [JailRenamer] behind an enum with [VmmExecutor] implemented. fctools was specifically
/// designed against heap allocation and dynamic dispatch, so this is a statically dispatched
/// enum instead.
pub enum EitherVmmExecutor<J: JailRenamer + 'static> {
    Unrestricted(UnrestrictedVmmExecutor),
    Jailed(JailedVmmExecutor<J>),
}

impl<J: JailRenamer + 'static> From<UnrestrictedVmmExecutor> for EitherVmmExecutor<J> {
    fn from(value: UnrestrictedVmmExecutor) -> Self {
        EitherVmmExecutor::Unrestricted(value)
    }
}

impl<J: JailRenamer + 'static> From<JailedVmmExecutor<J>> for EitherVmmExecutor<J> {
    fn from(value: JailedVmmExecutor<J>) -> Self {
        EitherVmmExecutor::Jailed(value)
    }
}

impl<J: JailRenamer + 'static> VmmExecutor for EitherVmmExecutor<J> {
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

    async fn prepare<R: Runtime>(
        &self,
        installation: &VmmInstallation,
        process_spawner: Arc<impl ProcessSpawner>,
        moved_resources: Vec<&mut MovedVmmResource>,
        ownership_model: VmmOwnershipModel,
    ) -> Result<(), VmmExecutorError> {
        match self {
            EitherVmmExecutor::Unrestricted(executor) => {
                executor
                    .prepare(installation, process_spawner, moved_resources, ownership_model)
                    .await
            }
            EitherVmmExecutor::Jailed(executor) => {
                executor
                    .prepare(installation, process_spawner, moved_resources, ownership_model)
                    .await
            }
        }
    }

    async fn invoke<R: Runtime>(
        &self,
        installation: &VmmInstallation,
        process_spawner: Arc<impl ProcessSpawner>,
        config_path: Option<PathBuf>,
        ownership_model: VmmOwnershipModel,
    ) -> Result<ProcessHandle<R::Process>, VmmExecutorError> {
        match self {
            EitherVmmExecutor::Unrestricted(executor) => {
                executor
                    .invoke::<R>(installation, process_spawner, config_path, ownership_model)
                    .await
            }
            EitherVmmExecutor::Jailed(executor) => {
                executor
                    .invoke::<R>(installation, process_spawner, config_path, ownership_model)
                    .await
            }
        }
    }

    async fn cleanup<R: Runtime>(
        &self,
        installation: &VmmInstallation,
        process_spawner: Arc<impl ProcessSpawner>,
        ownership_model: VmmOwnershipModel,
    ) -> Result<(), VmmExecutorError> {
        match self {
            EitherVmmExecutor::Unrestricted(executor) => {
                executor
                    .cleanup::<R>(installation, process_spawner, ownership_model)
                    .await
            }
            EitherVmmExecutor::Jailed(executor) => {
                executor
                    .cleanup::<R>(installation, process_spawner, ownership_model)
                    .await
            }
        }
    }
}
