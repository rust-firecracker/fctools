use std::{
    path::{Path, PathBuf},
    process::Stdio,
};

use tokio::{fs, process::Command};

/// A Firecracker installation encapsulates release binaries of the most important automatable
/// Firecracker components: firecracker, jailer, snapshot-editor. Using a partial installation with only
/// some of these binaries is neither recommended nor supported.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FirecrackerInstallation {
    pub firecracker_path: PathBuf,
    pub jailer_path: PathBuf,
    pub snapshot_editor_path: PathBuf,
}

/// Error caused during installation verification.
#[derive(Debug)]
pub enum FirecrackerVerificationError {
    FilesystemError(tokio::io::Error),
    BinaryMissing,
    BinaryNotExecutable,
    BinaryIsOfIncorrectType,
    BinaryDoesNotMatchExpectedVersion,
}

impl FirecrackerInstallation {
    pub async fn verify(&self, expected_version: &str) -> Result<(), FirecrackerVerificationError> {
        verify_imp(&self.firecracker_path, expected_version, "Firecracker").await?;
        verify_imp(&self.jailer_path, expected_version, "Jailer").await?;
        verify_imp(&self.snapshot_editor_path, expected_version, "snapshot-editor").await?;
        Ok(())
    }
}

async fn verify_imp(path: &Path, expected_version: &str, expected_name: &str) -> Result<(), FirecrackerVerificationError> {
    if !fs::try_exists(path)
        .await
        .map_err(FirecrackerVerificationError::FilesystemError)?
    {
        return Err(FirecrackerVerificationError::BinaryMissing);
    }

    let mut command = Command::new(path);
    command.arg("--version").stdout(Stdio::piped());
    let output = command
        .output()
        .await
        .map_err(|_| FirecrackerVerificationError::BinaryNotExecutable)?;
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();

    if !stdout.starts_with(expected_name) {
        return Err(FirecrackerVerificationError::BinaryIsOfIncorrectType);
    }

    if !stdout.contains(expected_version) {
        return Err(FirecrackerVerificationError::BinaryDoesNotMatchExpectedVersion);
    }

    Ok(())
}
