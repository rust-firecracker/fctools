use std::path::PathBuf;

use crate::{process_spawner::ProcessSpawner, runtime::Runtime, vmm::installation::VmmInstallation};

use super::{
    jailed::{JailRenamer, JailedVmmExecutor},
    process_handle::ProcessHandle,
    unrestricted::UnrestrictedVmmExecutor,
    VmmExecutor, VmmExecutorContext, VmmExecutorError,
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

    fn local_to_effective_path(&self, installation: &VmmInstallation, local_path: PathBuf) -> PathBuf {
        match self {
            EitherVmmExecutor::Unrestricted(executor) => executor.local_to_effective_path(installation, local_path),
            EitherVmmExecutor::Jailed(executor) => executor.local_to_effective_path(installation, local_path),
        }
    }

    async fn prepare<S: ProcessSpawner, R: Runtime>(
        &mut self,
        context: VmmExecutorContext<'_, S, R>,
    ) -> Result<(), VmmExecutorError> {
        match self {
            EitherVmmExecutor::Unrestricted(executor) => executor.prepare(context).await,
            EitherVmmExecutor::Jailed(executor) => executor.prepare(context).await,
        }
    }

    async fn invoke<S: ProcessSpawner, R: Runtime>(
        &mut self,
        context: VmmExecutorContext<'_, S, R>,
        config_path: Option<PathBuf>,
    ) -> Result<ProcessHandle<R::Process>, VmmExecutorError> {
        match self {
            EitherVmmExecutor::Unrestricted(executor) => executor.invoke(context, config_path).await,
            EitherVmmExecutor::Jailed(executor) => executor.invoke(context, config_path).await,
        }
    }

    async fn cleanup<S: ProcessSpawner, R: Runtime>(
        &mut self,
        context: VmmExecutorContext<'_, S, R>,
    ) -> Result<(), VmmExecutorError> {
        match self {
            EitherVmmExecutor::Unrestricted(executor) => executor.cleanup(context).await,
            EitherVmmExecutor::Jailed(executor) => executor.cleanup(context).await,
        }
    }
}
