use std::path::PathBuf;

use fctools::{
    process_spawner::{DirectProcessSpawner, ProcessSpawner, SuProcessSpawner, SudoProcessSpawner},
    runtime::{tokio::TokioRuntime, RuntimeChild},
    vmm::installation::{VmmInstallation, VmmInstallationVerificationError},
};
use futures_util::AsyncReadExt;
use test_framework::{get_test_path, TestOptions};
use uuid::Uuid;

mod test_framework;

#[tokio::test]
async fn installation_does_not_verify_for_missing_files() {
    fn random_path() -> PathBuf {
        PathBuf::from(format!("/tmp/{}", Uuid::new_v4()))
    }

    let installation = VmmInstallation::new(random_path(), random_path(), random_path());

    assert_matches::assert_matches!(
        installation
            .verify(&TestOptions::get().await.toolchain.version, &TokioRuntime)
            .await,
        Err(VmmInstallationVerificationError::BinaryMissing)
    );
}

#[tokio::test]
async fn installation_does_not_verify_for_non_executable_files() {
    async fn non_executable_path() -> PathBuf {
        let path = PathBuf::from(format!("/tmp/{}", Uuid::new_v4()));
        drop(tokio::fs::File::create(&path).await.unwrap());
        path
    }

    let installation = VmmInstallation::new(
        non_executable_path().await,
        non_executable_path().await,
        non_executable_path().await,
    );

    assert_matches::assert_matches!(
        installation
            .verify(&TestOptions::get().await.toolchain.version, &TokioRuntime)
            .await,
        Err(VmmInstallationVerificationError::BinaryNotExecutable)
    );
}

#[tokio::test]
async fn installation_does_not_verify_for_incorrect_binary_type() {
    let installation = VmmInstallation::new(
        get_test_path("toolchain/jailer"),
        get_test_path("toolchain/snapshot-editor"),
        get_test_path("toolchain/firecracker"),
    );

    assert_matches::assert_matches!(
        installation
            .verify(&TestOptions::get().await.toolchain.version, &TokioRuntime)
            .await,
        Err(VmmInstallationVerificationError::BinaryIsOfIncorrectType)
    );
}

#[tokio::test]
async fn installation_does_not_verify_for_incorrect_binary_version() {
    let installation = VmmInstallation::new(
        get_test_path("toolchain/firecracker-wrong-version"),
        get_test_path("toolchain/jailer"),
        get_test_path("toolchain/snapshot-editor"),
    );

    assert_matches::assert_matches!(
        installation
            .verify(&TestOptions::get().await.toolchain.version, &TokioRuntime)
            .await,
        Err(VmmInstallationVerificationError::BinaryDoesNotMatchExpectedVersion)
    );
}

#[tokio::test]
async fn installation_verifies_for_correct_parameters() {
    let installation = VmmInstallation::new(
        get_test_path("toolchain/firecracker"),
        get_test_path("toolchain/jailer"),
        get_test_path("toolchain/snapshot-editor"),
    );

    installation
        .verify(&TestOptions::get().await.toolchain.version, &TokioRuntime)
        .await
        .unwrap();
}

#[tokio::test]
async fn direct_process_spawner_can_null_pipes() {
    let mut process = DirectProcessSpawner
        .spawn(&PathBuf::from("echo"), vec![], true, &TokioRuntime)
        .await
        .unwrap();
    assert!(process.take_stdout().is_none());
    assert!(process.take_stderr().is_none());
    assert!(process.take_stdin().is_none());
}

#[tokio::test]
async fn direct_process_spawner_can_invoke_process() {
    let mut process = DirectProcessSpawner
        .spawn(&PathBuf::from("bash"), vec!["--help".to_string()], false, &TokioRuntime)
        .await
        .unwrap();
    let mut buf = Vec::new();
    process.take_stdout().unwrap().read_to_end(&mut buf).await.unwrap();
    let buf_string = String::from_utf8(buf).unwrap();
    assert!(buf_string.contains("GNU bash"));
}

#[tokio::test]
async fn su_process_spawner_can_elevate() {
    test_elevation(|password| SuProcessSpawner::new(password, None), false).await;
}

#[tokio::test]
async fn su_process_spawner_can_null_pipes() {
    test_elevation(|password| SuProcessSpawner::new(password, None), true).await;
}

#[tokio::test]
async fn sudo_process_spawner_can_elevate() {
    test_elevation(|password| SudoProcessSpawner::new(Some(password), None), false).await;
}

#[tokio::test]
async fn sudo_process_spawner_can_null_pipes() {
    test_elevation(|password| SudoProcessSpawner::new(Some(password), None), true).await;
}

async fn test_elevation<F: FnOnce(String) -> S, S: ProcessSpawner>(process_spawner_function: F, pipes_nulled: bool) {
    let Ok(password) = std::env::var("ROOT_PWD") else {
        println!("ROOT_PWD env var wasn't set for the elevation test, skipping it");
        return;
    };

    let process_spawner = process_spawner_function(password);
    let mut process = process_spawner
        .spawn(
            &PathBuf::from("bash"),
            vec!["-c".to_string(), "'echo $UID'".to_string()],
            pipes_nulled,
            &TokioRuntime,
        )
        .await
        .unwrap();

    if pipes_nulled {
        assert!(process.take_stdout().is_none());
        assert!(process.take_stderr().is_none());
    } else {
        let mut stdout = Vec::new();
        process.take_stdout().unwrap().read_to_end(&mut stdout).await.unwrap();
        assert_eq!(String::from_utf8(stdout).unwrap(), "0\n");
    }
}
