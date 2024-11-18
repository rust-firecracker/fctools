use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use crate::runtime::{Runtime, RuntimeFilesystem, RuntimeProcess};

/// A [VmmInstallation] encapsulates release binaries of the most important automatable VMM components:
/// "firecracker", "jailer" and "snapshot-editor". Using a partial installation with only
/// some of these binaries is neither recommended nor supported.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct VmmInstallation {
    pub firecracker_path: PathBuf,
    pub jailer_path: PathBuf,
    pub snapshot_editor_path: PathBuf,
}

/// Error caused during [VmmInstallation] verification.
#[derive(Debug, thiserror::Error)]
pub enum VmmInstallationError {
    #[error("An I/O error was emitted by the filesystem: {0}")]
    FilesystemError(std::io::Error),
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
    /// Verify the [VmmInstallation] using the given [FsBackend] by ensuring all binaries exist,
    /// are executable and yield the correct type and version when spawned and awaited with "--version".
    pub async fn verify<R: Runtime>(&self, expected_version: impl AsRef<str>) -> Result<(), VmmInstallationError> {
        tokio::try_join!(
            verify_imp::<R>(&self.firecracker_path, expected_version.as_ref(), "Firecracker",),
            verify_imp::<R>(&self.jailer_path, expected_version.as_ref(), "Jailer"),
            verify_imp::<R>(&self.snapshot_editor_path, expected_version.as_ref(), "snapshot-editor",)
        )?;
        Ok(())
    }
}

async fn verify_imp<R: Runtime>(
    path: &Path,
    expected_version: &str,
    expected_name: &str,
) -> Result<(), VmmInstallationError> {
    if !R::Filesystem::check_exists(path)
        .await
        .map_err(VmmInstallationError::FilesystemError)?
    {
        return Err(VmmInstallationError::BinaryMissing);
    }

    let mut command = Command::new(path);
    command
        .arg("--version")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .stdin(Stdio::null());

    let output = R::Process::output(command)
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
