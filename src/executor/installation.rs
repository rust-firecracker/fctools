use std::{
    path::{Path, PathBuf},
    process::Stdio,
};

use tokio::{fs, process::Command};

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
    #[error("Accessing a file via tokio I/O failed: `{0}`")]
    FilesystemError(tokio::io::Error),
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
    /// Verify the installation by ensuring all binaries exist, are executable and yield the correct type and version when forked.
    pub async fn verify(&self, expected_version: impl AsRef<str>) -> Result<(), VmmInstallationError> {
        verify_imp(&self.firecracker_path, expected_version.as_ref(), "Firecracker").await?;
        verify_imp(&self.jailer_path, expected_version.as_ref(), "Jailer").await?;
        verify_imp(&self.snapshot_editor_path, expected_version.as_ref(), "snapshot-editor").await?;
        Ok(())
    }
}

async fn verify_imp(path: &Path, expected_version: &str, expected_name: &str) -> Result<(), VmmInstallationError> {
    if !fs::try_exists(path)
        .await
        .map_err(VmmInstallationError::FilesystemError)?
    {
        return Err(VmmInstallationError::BinaryMissing);
    }

    let mut command = Command::new(path);
    command.arg("--version").stdout(Stdio::piped());
    let output = command
        .output()
        .await
        .map_err(|_| VmmInstallationError::BinaryNotExecutable)?;
    let stdout = dbg!(String::from_utf8_lossy(&output.stdout).into_owned());

    if !stdout.starts_with(expected_name) {
        return Err(VmmInstallationError::BinaryIsOfIncorrectType);
    }

    if !stdout.contains(expected_version) {
        return Err(VmmInstallationError::BinaryDoesNotMatchExpectedVersion);
    }

    Ok(())
}
