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
pub enum FirecrackerInstallationError {
    FilesystemError(tokio::io::Error),
    BinaryMissing,
    BinaryNotExecutable,
    BinaryIsOfIncorrectType,
    BinaryDoesNotMatchExpectedVersion,
}

impl FirecrackerInstallation {
    /// Verify the installation by ensuring all binaries exist, are executable and yield the correct type and version when forked.
    pub async fn verify(&self, expected_version: impl AsRef<str>) -> Result<(), FirecrackerInstallationError> {
        verify_imp(&self.firecracker_path, expected_version.as_ref(), "Firecracker").await?;
        verify_imp(&self.jailer_path, expected_version.as_ref(), "Jailer").await?;
        verify_imp(&self.snapshot_editor_path, expected_version.as_ref(), "snapshot-editor").await?;
        Ok(())
    }
}

async fn verify_imp(
    path: &Path,
    expected_version: &str,
    expected_name: &str,
) -> Result<(), FirecrackerInstallationError> {
    if !fs::try_exists(path)
        .await
        .map_err(FirecrackerInstallationError::FilesystemError)?
    {
        return Err(FirecrackerInstallationError::BinaryMissing);
    }

    let mut command = Command::new(path);
    command.arg("--version").stdout(Stdio::piped());
    let output = command
        .output()
        .await
        .map_err(|_| FirecrackerInstallationError::BinaryNotExecutable)?;
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();

    if !stdout.starts_with(expected_name) {
        return Err(FirecrackerInstallationError::BinaryIsOfIncorrectType);
    }

    if !stdout.contains(expected_version) {
        return Err(FirecrackerInstallationError::BinaryDoesNotMatchExpectedVersion);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use uuid::Uuid;

    use crate::executor::installation::{FirecrackerInstallation, FirecrackerInstallationError};

    #[tokio::test]
    async fn installation_does_not_verify_for_missing_files() {
        fn random_path() -> PathBuf {
            PathBuf::from(format!("/tmp/{}", Uuid::new_v4()))
        }

        let installation = FirecrackerInstallation {
            firecracker_path: random_path(),
            jailer_path: random_path(),
            snapshot_editor_path: random_path(),
        };
        assert_matches::assert_matches!(
            installation.verify("v1.8.0").await,
            Err(FirecrackerInstallationError::BinaryMissing)
        );
    }

    #[tokio::test]
    async fn installation_does_not_verify_for_non_executable_files() {
        async fn non_executable_path() -> PathBuf {
            let path = PathBuf::from(format!("/tmp/{}", Uuid::new_v4()));
            drop(tokio::fs::File::create(&path).await.unwrap());
            path
        }

        let installation = FirecrackerInstallation {
            firecracker_path: non_executable_path().await,
            jailer_path: non_executable_path().await,
            snapshot_editor_path: non_executable_path().await,
        };
        assert_matches::assert_matches!(
            installation.verify("v1.8.0").await,
            Err(FirecrackerInstallationError::BinaryNotExecutable)
        );
    }

    fn testdata(name: &str) -> PathBuf {
        format!("/opt/testdata/{name}").into()
    }

    #[tokio::test]
    async fn installation_does_not_verify_for_incorrect_binary_type() {
        let installation = FirecrackerInstallation {
            firecracker_path: testdata("jailer"),
            jailer_path: testdata("snapshot-editor"),
            snapshot_editor_path: testdata("firecracker"),
        };
        assert_matches::assert_matches!(
            installation.verify("v1.8.0").await,
            Err(FirecrackerInstallationError::BinaryIsOfIncorrectType)
        );
    }

    #[tokio::test]
    async fn installation_does_not_verify_for_incorrect_binary_version() {
        let installation = FirecrackerInstallation {
            firecracker_path: testdata("firecracker-wrong-version"),
            jailer_path: testdata("jailer"),
            snapshot_editor_path: testdata("snapshot-editor"),
        };
        assert_matches::assert_matches!(
            installation.verify("v1.8.0").await,
            Err(FirecrackerInstallationError::BinaryDoesNotMatchExpectedVersion)
        );
    }
}
