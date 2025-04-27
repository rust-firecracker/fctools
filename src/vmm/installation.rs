use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    sync::Arc,
};

use crate::runtime::Runtime;

/// A [VmmInstallation] encapsulates release binaries of the most important automatable VMM components:
/// "firecracker", "jailer" and "snapshot-editor". The [VmmInstallation] holds an [Arc] of an inner struct
/// that owns the paths in order to avoid excessive path copying in memory. As such, cloning a [VmmInstallation]
/// is cheap and creating an [Arc] of [VmmInstallation] is entirely redundant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VmmInstallation(Arc<VmmInstallationInner>);

#[derive(Debug, PartialEq, Eq)]
struct VmmInstallationInner {
    firecracker_path: PathBuf,
    jailer_path: PathBuf,
    snapshot_editor_path: PathBuf,
}

/// Error caused during [VmmInstallation] verification.
#[derive(Debug)]
pub enum VmmInstallationVerificationError {
    /// An I/O error occurred while interacting with the filesystem.
    FilesystemError(std::io::Error),
    /// An installation binary was missing on the filesystem.
    BinaryMissing,
    /// An installation binary did not have the executable permission.
    BinaryNotExecutable,
    /// An installation binary didn't report the expected type, meaning it either doesn't
    /// belong to a Firecracker toolchain, or the paths for the installation were passed
    /// in an incorrect order (meaning they are mismatched with the actual binaries).
    BinaryIsOfIncorrectType,
    /// An installation binary didn't match the expected version.
    BinaryDoesNotMatchExpectedVersion,
}

impl std::error::Error for VmmInstallationVerificationError {}

impl std::fmt::Display for VmmInstallationVerificationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VmmInstallationVerificationError::FilesystemError(err) => {
                write!(f, "A filesystem operation backed by the runtime failed: {err}")
            }
            VmmInstallationVerificationError::BinaryMissing => {
                write!(f, "A binary inside the installation doesn't exist")
            }
            VmmInstallationVerificationError::BinaryNotExecutable => {
                write!(f, "A binary inside the installation doesn't have execution permissions")
            }
            VmmInstallationVerificationError::BinaryIsOfIncorrectType => {
                write!(f, "A binary inside the installation is incorrectly labeled")
            }
            VmmInstallationVerificationError::BinaryDoesNotMatchExpectedVersion => {
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
    /// are executable and yield the correct type and version when spawned and waited on with "--version".
    pub async fn verify<R: Runtime, V: AsRef<str>>(
        &self,
        expected_version: V,
        runtime: &R,
    ) -> Result<(), VmmInstallationVerificationError> {
        futures_util::try_join!(
            verify_imp(
                runtime,
                &self.0.firecracker_path,
                expected_version.as_ref(),
                "Firecracker"
            ),
            verify_imp(runtime, &self.0.jailer_path, expected_version.as_ref(), "Jailer"),
            verify_imp(
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
) -> Result<(), VmmInstallationVerificationError> {
    if !runtime
        .fs_exists(path)
        .await
        .map_err(VmmInstallationVerificationError::FilesystemError)?
    {
        return Err(VmmInstallationVerificationError::BinaryMissing);
    }

    let output = runtime
        .run_process(path.as_os_str(), &[OsString::from("--version")], true, false)
        .await
        .map_err(|_| VmmInstallationVerificationError::BinaryNotExecutable)?;
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();

    if !stdout.starts_with(expected_name) {
        return Err(VmmInstallationVerificationError::BinaryIsOfIncorrectType);
    }

    if !stdout.contains(expected_version) {
        return Err(VmmInstallationVerificationError::BinaryDoesNotMatchExpectedVersion);
    }

    Ok(())
}
