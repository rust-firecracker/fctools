use std::path::PathBuf;

use fctools::{
    process_spawner::{DirectProcessSpawner, ProcessSpawner},
    runtime::{tokio::TokioRuntime, RuntimeProcess},
    vmm::installation::{VmmInstallation, VmmInstallationError},
};
use test_framework::{get_test_path, TestOptions};
use uuid::Uuid;

mod test_framework;

#[tokio::test]
async fn installation_does_not_verify_for_missing_files() {
    fn random_path() -> PathBuf {
        PathBuf::from(format!("/tmp/{}", Uuid::new_v4()))
    }

    let installation = VmmInstallation {
        firecracker_path: random_path(),
        jailer_path: random_path(),
        snapshot_editor_path: random_path(),
    };
    assert_matches::assert_matches!(
        installation
            .verify::<TokioRuntime>(&TestOptions::get().await.toolchain.version)
            .await,
        Err(VmmInstallationError::BinaryMissing)
    );
}

#[tokio::test]
async fn installation_does_not_verify_for_non_executable_files() {
    async fn non_executable_path() -> PathBuf {
        let path = PathBuf::from(format!("/tmp/{}", Uuid::new_v4()));
        drop(tokio::fs::File::create(&path).await.unwrap());
        path
    }

    let installation = VmmInstallation {
        firecracker_path: non_executable_path().await,
        jailer_path: non_executable_path().await,
        snapshot_editor_path: non_executable_path().await,
    };
    assert_matches::assert_matches!(
        installation
            .verify::<TokioRuntime>(&TestOptions::get().await.toolchain.version)
            .await,
        Err(VmmInstallationError::BinaryNotExecutable)
    );
}

#[tokio::test]
async fn installation_does_not_verify_for_incorrect_binary_type() {
    let installation = VmmInstallation {
        firecracker_path: get_test_path("toolchain/jailer"),
        jailer_path: get_test_path("toolchain/snapshot-editor"),
        snapshot_editor_path: get_test_path("toolchain/firecracker"),
    };
    assert_matches::assert_matches!(
        installation
            .verify::<TokioRuntime>(&TestOptions::get().await.toolchain.version)
            .await,
        Err(VmmInstallationError::BinaryIsOfIncorrectType)
    );
}

#[tokio::test]
async fn installation_does_not_verify_for_incorrect_binary_version() {
    let installation = VmmInstallation {
        firecracker_path: get_test_path("toolchain/firecracker-wrong-version"),
        jailer_path: get_test_path("toolchain/jailer"),
        snapshot_editor_path: get_test_path("toolchain/snapshot-editor"),
    };
    assert_matches::assert_matches!(
        installation
            .verify::<TokioRuntime>(&TestOptions::get().await.toolchain.version)
            .await,
        Err(VmmInstallationError::BinaryDoesNotMatchExpectedVersion)
    );
}

#[tokio::test]
async fn installation_verifies_for_correct_parameters() {
    let installation = VmmInstallation {
        firecracker_path: get_test_path("toolchain/firecracker"),
        jailer_path: get_test_path("toolchain/jailer"),
        snapshot_editor_path: get_test_path("toolchain/snapshot-editor"),
    };
    installation
        .verify::<TokioRuntime>(&TestOptions::get().await.toolchain.version)
        .await
        .unwrap();
}

#[tokio::test]
async fn direct_process_spawner_can_null_pipes() {
    let mut process = DirectProcessSpawner
        .spawn::<TokioRuntime>(&PathBuf::from("echo"), vec![], true)
        .await
        .unwrap();
    assert!(process.take_stdout().is_none());
    assert!(process.take_stderr().is_none());
    assert!(process.take_stdin().is_none());
}
