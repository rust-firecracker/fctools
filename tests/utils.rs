use std::path::PathBuf;

use assert_matches::assert_matches;
use fctools::{
    executor::{
        installation::{FirecrackerInstallation, FirecrackerInstallationError},
        jailed::{FlatJailRenamer, JailRenamer, JailRenamerError, MappingJailRenamer},
    },
    shell::{SameUserShellSpawner, ShellSpawner, SuShellSpawner, SudoShellSpawner},
};
use tokio::fs;
use uuid::Uuid;

#[tokio::test]
async fn same_user_shell_launches_simple_command() {
    let shell_spawner = SameUserShellSpawner {
        shell_path: PathBuf::from("/usr/bin/bash"),
    };
    let child = shell_spawner.spawn("cat --help".into()).await.unwrap();
    let output = child.wait_with_output().await.unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(stdout.contains("Usage: cat [OPTION]... [FILE]..."));
    assert!(stdout.contains("or available locally via: info '(coreutils) cat invocation'"))
}

#[tokio::test]
async fn same_user_shell_runs_under_correct_uid() {
    let uid = unsafe { libc::geteuid() };
    let shell_spawner = SameUserShellSpawner {
        shell_path: PathBuf::from("/usr/bin/sh"),
    };
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

async fn elevation_test<F, S>(closure: F)
where
    F: FnOnce(String) -> S,
    S: ShellSpawner,
{
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
    assert_matches!(
        installation.verify("v1.8.0").await,
        Err(FirecrackerInstallationError::BinaryMissing)
    );
}

#[tokio::test]
async fn installation_does_not_verify_for_non_executable_files() {
    async fn non_executable_path() -> PathBuf {
        let path = PathBuf::from(format!("/tmp/{}", Uuid::new_v4()));
        drop(fs::File::create(&path).await.unwrap());
        path
    }

    let installation = FirecrackerInstallation {
        firecracker_path: non_executable_path().await,
        jailer_path: non_executable_path().await,
        snapshot_editor_path: non_executable_path().await,
    };
    assert_matches!(
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
    assert_matches!(
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
    assert_matches!(
        installation.verify("v1.8.0").await,
        Err(FirecrackerInstallationError::BinaryDoesNotMatchExpectedVersion)
    );
}

#[test]
fn flat_jail_renamer_moves_correctly() {
    let renamer = FlatJailRenamer::default();
    assert_renamer(&renamer, "/opt/file", "/file");
    assert_renamer(&renamer, "/tmp/some_path.txt", "/some_path.txt");
    assert_renamer(&renamer, "/some/complex/outside/path/filename.ext4", "/filename.ext4");
}

#[test]
fn mapping_jail_renamer_moves_correctly() {
    let mut renamer = MappingJailRenamer::new();
    renamer
        .map("/etc/a", "/tmp/a")
        .map("/opt/b", "/etc/b")
        .map("/tmp/c", "/c");
    assert_renamer(&renamer, "/etc/a", "/tmp/a");
    assert_renamer(&renamer, "/opt/b", "/etc/b");
    assert_renamer(&renamer, "/tmp/c", "/c");
    assert_matches!(
        renamer.rename_for_jail(PathBuf::from("/tmp/unknown").as_ref()),
        Err(JailRenamerError::PathIsUnmapped(_))
    );
}

fn assert_renamer(renamer: &impl JailRenamer, path: &str, expectation: &str) {
    assert_eq!(
        renamer.rename_for_jail(&PathBuf::from(path)).unwrap().to_str().unwrap(),
        expectation
    );
}
