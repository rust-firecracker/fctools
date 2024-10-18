use std::{
    path::{Path, PathBuf},
    process::Stdio,
};

use tokio::process::Command;

use crate::fs_backend::{FsBackend, FsBackendError};

/// A VMM installation encapsulates release binaries of the most important automatable
/// Firecracker components: firecracker, jailer, snapshot-editor. Using a partial installation with only
/// some of these binaries is neither recommended nor supported.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct VmmInstallation {
    pub firecracker_path: PathBuf,
    pub jailer_path: PathBuf,
    pub snapshot_editor_path: PathBuf,
}

/// Error caused during installation verification.
#[derive(Debug, thiserror::Error)]
pub enum VmmInstallationError {
    #[error("An error was emitted by the FS backend: `{0}`")]
    FsBackendError(FsBackendError),
    #[error("A binary inside the installation doesn't exist")]
    BinaryMissing,
    #[error("A binary inside the installation does not have execution permissions")]
    BinaryNotExecutable,
    #[error("A binary inside the installation is incorrectly labeled as something else")]
    BinaryIsOfIncorrectType,
    #[error("A binary's version inside the installation does not match the version that was given")]
    BinaryDoesNotMatchExpectedVersion,
}

impl VmmInstallation {
    /// Verify the installation using the given filesystem backend by ensuring all binaries exist,
    /// are executable and yield the correct type and version when forked.
    pub async fn verify(
        &self,
        fs_backend: &impl FsBackend,
        expected_version: impl AsRef<str>,
    ) -> Result<(), VmmInstallationError> {
        verify_imp(
            fs_backend,
            &self.firecracker_path,
            expected_version.as_ref(),
            "Firecracker",
        )
        .await?;
        verify_imp(fs_backend, &self.jailer_path, expected_version.as_ref(), "Jailer").await?;
        verify_imp(
            fs_backend,
            &self.snapshot_editor_path,
            expected_version.as_ref(),
            "snapshot-editor",
        )
        .await?;
        Ok(())
    }
}

async fn verify_imp(
    fs_backend: &impl FsBackend,
    path: &Path,
    expected_version: &str,
    expected_name: &str,
) -> Result<(), VmmInstallationError> {
    if !fs_backend
        .check_exists(path)
        .await
        .map_err(VmmInstallationError::FsBackendError)?
    {
        return Err(VmmInstallationError::BinaryMissing);
    }

    let mut command = Command::new(path);
    command.arg("--version").stdout(Stdio::piped());
    let output = command
        .output()
        .await
        .map_err(|_| VmmInstallationError::BinaryNotExecutable)?;
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();

    if !stdout.starts_with(expected_name) {
        return Err(VmmInstallationError::BinaryIsOfIncorrectType);
    }

    if !stdout.contains(expected_version) {
        return Err(VmmInstallationError::BinaryDoesNotMatchExpectedVersion);
    }

    Ok(())
}
