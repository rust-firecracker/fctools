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
#[derive(Debug)]
pub enum VmmInstallationError {
    FilesystemError(std::io::Error),
    BinaryMissing,
    BinaryNotExecutable,
    BinaryIsOfIncorrectType,
    BinaryDoesNotMatchExpectedVersion,
}

impl std::error::Error for VmmInstallationError {}

impl std::fmt::Display for VmmInstallationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VmmInstallationError::FilesystemError(err) => {
                write!(f, "A filesystem operation backed by the runtime failed: {err}")
            }
            VmmInstallationError::BinaryMissing => write!(f, "A binary inside the installation doesn't exist"),
            VmmInstallationError::BinaryNotExecutable => {
                write!(f, "A binary inside the installation doesn't have execution permissions")
            }
            VmmInstallationError::BinaryIsOfIncorrectType => {
                write!(f, "A binary inside the installation is incorrectly labeled")
            }
            VmmInstallationError::BinaryDoesNotMatchExpectedVersion => {
                write!(f, "A binary inside the installation does not match the given version")
            }
        }
    }
}

impl VmmInstallation {
    /// Verify the [VmmInstallation] using the given [Runtime] by ensuring all binaries exist,
    /// are executable and yield the correct type and version when spawned and awaited with "--version".
    pub async fn verify<R: Runtime>(&self, expected_version: impl AsRef<str>) -> Result<(), VmmInstallationError> {
        futures_util::try_join!(
            verify_imp::<R>(&self.firecracker_path, expected_version.as_ref(), "Firecracker"),
            verify_imp::<R>(&self.jailer_path, expected_version.as_ref(), "Jailer"),
            verify_imp::<R>(&self.snapshot_editor_path, expected_version.as_ref(), "snapshot-editor")
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
