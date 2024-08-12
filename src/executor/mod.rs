use std::{
    collections::HashMap,
    io,
    path::{Path, PathBuf},
    process::ExitStatus,
};

use arguments::FirecrackerConfigOverride;
use async_trait::async_trait;
use installation::FirecrackerInstallation;
use jailed::JailRenamerError;
use tokio::{fs, process::Child, task::JoinError};

use crate::shell_spawner::ShellSpawner;

pub mod arguments;
pub mod command_modifier;
pub mod installation;
pub mod jailed;
pub mod unrestricted;

#[derive(Debug)]
pub enum FirecrackerExecutorError {
    IoError(io::Error),
    ShellWaitFailed(io::Error),
    ChownExitedWithWrongStatus(ExitStatus),
    MkdirExitedWithWrongStatus(ExitStatus),
    TaskJoinFailed(JoinError),
    ShellSpawnFailed(io::Error),
    ExpectedResourceMissing(PathBuf),
    ExpectedDirectoryParentMissing,
    ToInnerPathFailed(JailRenamerError),
    Other(Box<dyn std::error::Error + Send>),
}

/// A VMM executor is layer 2 of FCTools: manages a VMM process by setting up the environment, correctly invoking
/// the process and cleaning up the environment. This allows modularity between different modes of VMM execution.
#[async_trait]
pub trait VmmExecutor {
    /// Get the host location of the VMM socket, if one exists.
    fn get_outer_socket_path(&self) -> Option<PathBuf>;

    /// Resolves an inner path into an outer path.
    fn inner_to_outer_path(&self, inner_path: &Path) -> PathBuf;

    /// Prepare all transient resources for the VM invocation.
    async fn prepare(
        &self,
        shell_spawner: &impl ShellSpawner,
        outer_paths: Vec<PathBuf>,
    ) -> Result<HashMap<PathBuf, PathBuf>, FirecrackerExecutorError>;

    /// Invoke the VM on the given FirecrackerInstallation and return the spawned tokio Child.
    async fn invoke(
        &self,
        shell_spawner: &impl ShellSpawner,
        installation: &FirecrackerInstallation,
        config_override: FirecrackerConfigOverride,
    ) -> Result<Child, FirecrackerExecutorError>;

    /// Clean up all transient resources of the VM invocation.
    async fn cleanup(&self, shell_spawner: &impl ShellSpawner) -> Result<(), FirecrackerExecutorError>;
}

pub(crate) async fn force_chown(
    path: &Path,
    shell_spawner: &impl ShellSpawner,
) -> Result<(), FirecrackerExecutorError> {
    if shell_spawner.belongs_to_process() {
        return Ok(());
    }

    // SAFETY: calling FFI libc functions that return the process UID and GID can never result in UB
    let uid = unsafe { libc::geteuid() };
    let gid = unsafe { libc::getegid() };

    let mut child = shell_spawner
        .spawn(format!("chown -R {uid}:{gid} {}", path.to_string_lossy()))
        .await
        .map_err(FirecrackerExecutorError::ShellSpawnFailed)?;
    let exit_status = child.wait().await.map_err(FirecrackerExecutorError::ShellWaitFailed)?;

    if !exit_status.success() {
        return Err(FirecrackerExecutorError::ChownExitedWithWrongStatus(exit_status));
    }

    Ok(())
}

async fn force_mkdir(path: &Path, shell_spawner: &impl ShellSpawner) -> Result<(), FirecrackerExecutorError> {
    if shell_spawner.belongs_to_process() {
        fs::create_dir_all(path)
            .await
            .map_err(FirecrackerExecutorError::IoError)?;
        return Ok(());
    }

    let mut child = shell_spawner
        .spawn(format!("mkdir -p {}", path.to_string_lossy()))
        .await
        .map_err(FirecrackerExecutorError::ShellSpawnFailed)?;
    let exit_status = child.wait().await.map_err(FirecrackerExecutorError::ShellWaitFailed)?;

    if !exit_status.success() {
        return Err(FirecrackerExecutorError::MkdirExitedWithWrongStatus(exit_status));
    }

    Ok(())
}

async fn create_file_with_tree(path: impl AsRef<Path>) -> Result<(), FirecrackerExecutorError> {
    let path = path.as_ref();

    if let Some(parent_path) = path.parent() {
        fs::create_dir_all(parent_path)
            .await
            .map_err(FirecrackerExecutorError::IoError)?;
    }

    fs::File::create(path)
        .await
        .map_err(FirecrackerExecutorError::IoError)?;
    Ok(())
}
