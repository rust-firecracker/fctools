use std::path::PathBuf;

use fctools::executor::installation::{FirecrackerInstallation, FirecrackerInstallationError};
use test_framework::get_test_path;
use uuid::Uuid;

mod test_framework;

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

#[tokio::test]
async fn installation_does_not_verify_for_incorrect_binary_type() {
    let installation = FirecrackerInstallation {
        firecracker_path: get_test_path("jailer"),
        jailer_path: get_test_path("snapshot-editor"),
        snapshot_editor_path: get_test_path("firecracker"),
    };
    assert_matches::assert_matches!(
        installation.verify("v1.8.0").await,
        Err(FirecrackerInstallationError::BinaryIsOfIncorrectType)
    );
}

#[tokio::test]
async fn installation_does_not_verify_for_incorrect_binary_version() {
    let installation = FirecrackerInstallation {
        firecracker_path: get_test_path("firecracker-wrong-version"),
        jailer_path: get_test_path("jailer"),
        snapshot_editor_path: get_test_path("snapshot-editor"),
    };
    assert_matches::assert_matches!(
        installation.verify("v1.8.0").await,
        Err(FirecrackerInstallationError::BinaryDoesNotMatchExpectedVersion)
    );
}
