use std::path::PathBuf;

use crate::{process_spawner::ProcessSpawner, runtime::Runtime, vmm::installation::VmmInstallation};

use super::{
    jailed::{JailedVmmExecutor, VirtualPathResolver},
    process_handle::ProcessHandle,
    unrestricted::UnrestrictedVmmExecutor,
    VmmExecutor, VmmExecutorContext, VmmExecutorError,
};

/// [EitherVmmExecutor] encapsulates either an [UnrestrictedVmmExecutor] or a [JailedVmmExecutor]
/// with the given [VirtualPathResolver] behind an enum with [VmmExecutor] implemented on it. fctools was
/// specifically designed with the minimization of heap allocation and dynamic dispatch, so this is a
/// statically dispatched workaround provided out-of-the-box.
pub enum EitherVmmExecutor<V: VirtualPathResolver> {
    Unrestricted(UnrestrictedVmmExecutor),
    Jailed(JailedVmmExecutor<V>),
}

impl<V: VirtualPathResolver> From<UnrestrictedVmmExecutor> for EitherVmmExecutor<V> {
    fn from(value: UnrestrictedVmmExecutor) -> Self {
        EitherVmmExecutor::Unrestricted(value)
    }
}

impl<V: VirtualPathResolver> From<JailedVmmExecutor<V>> for EitherVmmExecutor<V> {
    fn from(value: JailedVmmExecutor<V>) -> Self {
        EitherVmmExecutor::Jailed(value)
    }
}

impl<V: VirtualPathResolver> VmmExecutor for EitherVmmExecutor<V> {
    fn get_socket_path(&self, installation: &VmmInstallation) -> Option<PathBuf> {
        match self {
            EitherVmmExecutor::Unrestricted(executor) => executor.get_socket_path(installation),
            EitherVmmExecutor::Jailed(executor) => executor.get_socket_path(installation),
        }
    }

    fn resolve_effective_path(&self, installation: &VmmInstallation, local_path: PathBuf) -> PathBuf {
        match self {
            EitherVmmExecutor::Unrestricted(executor) => executor.resolve_effective_path(installation, local_path),
            EitherVmmExecutor::Jailed(executor) => executor.resolve_effective_path(installation, local_path),
        }
    }

    async fn prepare<S: ProcessSpawner, R: Runtime>(
        &self,
        context: VmmExecutorContext<'_, S, R>,
    ) -> Result<(), VmmExecutorError> {
        match self {
            EitherVmmExecutor::Unrestricted(executor) => executor.prepare(context).await,
            EitherVmmExecutor::Jailed(executor) => executor.prepare(context).await,
        }
    }

    async fn invoke<S: ProcessSpawner, R: Runtime>(
        &self,
        context: VmmExecutorContext<'_, S, R>,
        config_path: Option<PathBuf>,
    ) -> Result<ProcessHandle<R>, VmmExecutorError> {
        match self {
            EitherVmmExecutor::Unrestricted(executor) => executor.invoke(context, config_path).await,
            EitherVmmExecutor::Jailed(executor) => executor.invoke(context, config_path).await,
        }
    }

    async fn cleanup<S: ProcessSpawner, R: Runtime>(
        &self,
        context: VmmExecutorContext<'_, S, R>,
    ) -> Result<(), VmmExecutorError> {
        match self {
            EitherVmmExecutor::Unrestricted(executor) => executor.cleanup(context).await,
            EitherVmmExecutor::Jailed(executor) => executor.cleanup(context).await,
        }
    }
}
