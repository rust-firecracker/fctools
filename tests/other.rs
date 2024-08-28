use std::path::PathBuf;

use fctools::{
    executor::installation::{FirecrackerInstallation, FirecrackerInstallationError},
    shell_spawner::{SameUserShellSpawner, ShellSpawner, SuShellSpawner, SudoShellSpawner},
};
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

#[tokio::test]
async fn same_user_shell_launches_simple_command() {
    let shell_spawner = SameUserShellSpawner::new(which::which("sh").unwrap());
    let child = shell_spawner.spawn("cat --help".into()).await.unwrap();
    let output = child.wait_with_output().await.unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(stdout.contains("Usage: cat [OPTION]... [FILE]..."));
    assert!(stdout.contains("or available locally via: info '(coreutils) cat invocation'"))
}

#[tokio::test]
async fn same_user_shell_runs_under_correct_uid() {
    let uid = unsafe { libc::geteuid() };
    let shell_spawner = SameUserShellSpawner::new(which::which("sh").unwrap());
    let stdout = String::from_utf8_lossy(
        &shell_spawner
            .spawn("echo $UID".into())
            .await
            .unwrap()
            .wait_with_output()
            .await
            .unwrap()
            .stdout,
    )
    .into_owned();
    assert_eq!(stdout.trim_end().parse::<u32>().unwrap(), uid);
}

#[tokio::test]
async fn su_shell_should_elevate() {
    elevation_test(SuShellSpawner::new).await;
}

#[tokio::test]
async fn sudo_shell_should_elevate() {
    elevation_test(SudoShellSpawner::with_password).await;
}

async fn elevation_test<S: ShellSpawner, F: FnOnce(String) -> S>(closure: F) {
    let password = std::env::var("ROOT_PWD");
    if password.is_err() {
        println!("Test was skipped due to ROOT_PWD not being set");
        return;
    }
    let shell_spawner = closure(password.unwrap());
    let child = shell_spawner.spawn("echo $UID".into()).await.unwrap();
    let stdout = String::from_utf8_lossy(&child.wait_with_output().await.unwrap().stdout).into_owned();
    assert_eq!(stdout, "0\n");
}
