use std::{
    collections::HashMap,
    future::Future,
    os::unix::process::ExitStatusExt,
    path::{Path, PathBuf},
    process::ExitStatus,
    sync::Arc,
};

use arguments::ConfigurationFileOverride;
use installation::VmmInstallation;
use jailed::JailRenamerError;
use tokio::{
    process::Child,
    task::{JoinError, JoinSet},
};

use crate::{
    fs_backend::{FsBackend, FsBackendError},
    process_spawner::ProcessSpawner,
};

pub mod arguments;
pub mod command_modifier;
pub mod installation;
pub mod jailed;
pub mod unrestricted;

#[derive(Debug, thiserror::Error)]
pub enum VmmExecutorError {
    #[error("A non-FS I/O error occurred: `{0}")]
    IoError(std::io::Error),
    #[error("An I/O error emitted by an FS backend occurred: `{0}`")]
    FsBackendError(FsBackendError),
    #[error("Spawning an auxiliary process via the spawner to force chown/mkdir failed: `{0}`")]
    ShellForkFailed(std::io::Error),
    #[error("A forced chown command exited with a non-zero exit status: `{0}`")]
    ChownExitedWithWrongStatus(ExitStatus),
    #[error("A forced mkdir command exited with a non-zero exit status: `{0}`")]
    MkdirExitedWithWrongStatus(ExitStatus),
    #[error("Joining on a spawned async task failed: `{0}`")]
    TaskJoinFailed(JoinError),
    #[error("Spawning an auxiliary or primary process via the process spawner failed: `{0}`")]
    ProcessSpawnFailed(std::io::Error),
    #[error("A passed-in resource at the path `{0}` was expected but doesn't exist or isn't accessible")]
    ExpectedResourceMissing(PathBuf),
    #[error("A directory that is supposed to have a parent in the filesystem has none")]
    ExpectedDirectoryParentMissing,
    #[error("Invoking the jail renamer to produce an inner path failed: `{0}`")]
    JailRenamerFailed(JailRenamerError),
    #[error("The given installation's file items have no filenames")]
    InstallationHasNoFilename,
    #[error("Another error occurred: `{0}`")]
    Other(Box<dyn std::error::Error + Send>),
}

/// A VMM executor is layer 2 of fctools: manages the environment of a VMM process, correctly invoking the process,
/// setting up and subsequently cleaning the environment. This allows modularity between different modes of VMM execution.
pub trait VmmExecutor: Send + Sync {
    /// Get the host location of the VMM socket, if one exists.
    fn get_socket_path(&self, installation: &VmmInstallation) -> Option<PathBuf>;

    /// Resolves an inner path into an outer path.
    fn inner_to_outer_path(&self, installation: &VmmInstallation, inner_path: &Path) -> PathBuf;

    // Returns a boolean determining whether this executor leaves any traces on the host filesystem after cleanup.
    fn traceless(&self) -> bool;

    /// Prepare all transient resources for the VMM invocation.
    fn prepare(
        &self,
        installation: &VmmInstallation,
        process_spawner: Arc<impl ProcessSpawner>,
        fs_backend: Arc<impl FsBackend>,
        outer_paths: Vec<PathBuf>,
    ) -> impl Future<Output = Result<HashMap<PathBuf, PathBuf>, VmmExecutorError>> + Send;

    /// Invoke the VMM on the given FirecrackerInstallation and return the spawned tokio Child.
    fn invoke(
        &self,
        installation: &VmmInstallation,
        process_spawner: Arc<impl ProcessSpawner>,
        config_override: ConfigurationFileOverride,
    ) -> impl Future<Output = Result<Child, VmmExecutorError>> + Send;

    /// Clean up all transient resources of the VMM invocation.
    fn cleanup(
        &self,
        installation: &VmmInstallation,
        process_spawner: Arc<impl ProcessSpawner>,
        fs_backend: Arc<impl FsBackend>,
    ) -> impl Future<Output = Result<(), VmmExecutorError>> + Send;
}

pub(crate) async fn force_chown(path: &Path, process_spawner: &impl ProcessSpawner) -> Result<(), VmmExecutorError> {
    if !process_spawner.increases_privileges() {
        return Ok(());
    }

    // SAFETY: calling FFI libc functions that return the process UID and GID can never result in UB
    let uid = unsafe { libc::geteuid() };
    let gid = unsafe { libc::getegid() };

    let mut child = process_spawner
        .spawn(
            &PathBuf::from("chown"),
            vec![
                "chown".to_string(),
                "-f".to_string(),
                "-R".to_string(),
                format!("{uid}:{gid}"),
                path.to_string_lossy().into_owned(),
            ],
            true,
        )
        .await
        .map_err(VmmExecutorError::ProcessSpawnFailed)?;
    let exit_status = child.wait().await.map_err(VmmExecutorError::ShellForkFailed)?;

    // code 256 means that a concurrent chown is being called and the chown will still be applied, so this error can
    // "safely" be ignored, which is better than inducing the overhead of global locking on chown paths
    if !exit_status.success() && exit_status.into_raw() != 256 {
        return Err(VmmExecutorError::ChownExitedWithWrongStatus(exit_status));
    }

    Ok(())
}

async fn force_mkdir(
    fs_backend: &impl FsBackend,
    path: &Path,
    process_spawner: &impl ProcessSpawner,
) -> Result<(), VmmExecutorError> {
    if !process_spawner.increases_privileges() {
        fs_backend
            .create_dir_all(path)
            .await
            .map_err(VmmExecutorError::FsBackendError)?;
        return Ok(());
    }

    let mut child = process_spawner
        .spawn(
            &PathBuf::from("mkdir"),
            vec!["-p".to_string(), path.to_string_lossy().into_owned()],
            true,
        )
        .await
        .map_err(VmmExecutorError::ProcessSpawnFailed)?;
    let exit_status = child.wait().await.map_err(VmmExecutorError::ShellForkFailed)?;

    if !exit_status.success() {
        return Err(VmmExecutorError::MkdirExitedWithWrongStatus(exit_status));
    }

    Ok(())
}

async fn create_file_with_tree(fs_backend: Arc<impl FsBackend>, path: PathBuf) -> Result<(), VmmExecutorError> {
    if let Some(parent_path) = path.parent() {
        fs_backend
            .create_dir_all(parent_path)
            .await
            .map_err(VmmExecutorError::FsBackendError)?;
    }

    fs_backend
        .create_file(&path)
        .await
        .map_err(VmmExecutorError::FsBackendError)?;
    Ok(())
}

async fn join_on_set(mut join_set: JoinSet<Result<(), VmmExecutorError>>) -> Result<(), VmmExecutorError> {
    while let Some(result) = join_set.join_next().await {
        match result {
            Ok(result) => {
                if let Err(err) = result {
                    join_set.abort_all();
                    return Err(err);
                }
            }
            Err(err) => {
                join_set.abort_all();
                return Err(VmmExecutorError::TaskJoinFailed(err));
            }
        }
    }

    Ok(())
}
