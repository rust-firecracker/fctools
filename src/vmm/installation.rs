use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::Arc,
};

use crate::runtime::Runtime;

/// A [VmmInstallation] encapsulates release binaries of the most important automatable VMM components:
/// "firecracker", "jailer" and "snapshot-editor". The [VmmInstallation] holds an [Arc] of an inner struct
/// that owns the paths in order to avoid excessive path copying in memory. As such, cloning a [VmmInstallation]
/// is cheap and creating an [Arc<VmmInstallation>] is entirely redundant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VmmInstallation(Arc<VmmInstallationInner>);

#[derive(Debug, Clone, PartialEq, Eq)]
struct VmmInstallationInner {
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
    /// Create a new [VmmInstallation] from three paths to the "firecracker", "jailer" and "snapshot-editor"
    /// binaries respectively.
    pub fn new<P: Into<PathBuf>>(firecracker_path: P, jailer_path: P, snapshot_editor_path: P) -> Self {
        Self(Arc::new(VmmInstallationInner {
            firecracker_path: firecracker_path.into(),
            jailer_path: jailer_path.into(),
            snapshot_editor_path: snapshot_editor_path.into(),
        }))
    }

    /// Get a shared reference to this [VmmInstallation]'s path to the "firecracker" binary.
    pub fn get_firecracker_path(&self) -> &Path {
        &self.0.firecracker_path
    }

    /// Get a shared reference to this [VmmInstallation]'s path to the "jailer" binary.
    pub fn get_jailer_path(&self) -> &Path {
        &self.0.jailer_path
    }

    /// Get a shared reference to this [VmmInstallation]'s path to the "snapshot-editor" binary.
    pub fn get_snapshot_editor_path(&self) -> &Path {
        &self.0.snapshot_editor_path
    }

    /// Verify the [VmmInstallation] using the given [Runtime] by ensuring all binaries exist,
    /// are executable and yield the correct type and version when spawned and awaited with "--version".
    pub async fn verify<R: Runtime>(
        &self,
        expected_version: impl AsRef<str>,
        runtime: &R,
    ) -> Result<(), VmmInstallationError> {
        futures_util::try_join!(
            verify_imp::<R>(
                runtime,
                &self.0.firecracker_path,
                expected_version.as_ref(),
                "Firecracker"
            ),
            verify_imp::<R>(runtime, &self.0.jailer_path, expected_version.as_ref(), "Jailer"),
            verify_imp::<R>(
                runtime,
                &self.0.snapshot_editor_path,
                expected_version.as_ref(),
                "snapshot-editor"
            )
        )?;
        Ok(())
    }
}

async fn verify_imp<R: Runtime>(
    runtime: &R,
    path: &Path,
    expected_version: &str,
    expected_name: &str,
) -> Result<(), VmmInstallationError> {
    if !runtime
        .fs_exists(path)
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

    let output = runtime
        .run_child(command)
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
