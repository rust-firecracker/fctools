use std::{collections::HashMap, path::PathBuf};

use assert_matches::assert_matches;
use common::{get_mock_firecracker_installation, get_shell_spawner, get_tmp_path, FailingShellSpawner};
use fctools::executor::{
    arguments::{FirecrackerApiSocket, FirecrackerArguments, FirecrackerConfigOverride},
    command_modifier::{AppendCommandModifier, RewriteCommandModifier},
    unrestricted::UnrestrictedVmmExecutor,
    VmmExecutor, VmmExecutorError,
};
use tokio::fs::{remove_file, try_exists, File};

mod common;

#[test]
fn unrestricted_executor_returns_socket_path_according_to_configuration() {
    assert_eq!(
        UnrestrictedVmmExecutor::new(FirecrackerArguments::new(FirecrackerApiSocket::Disabled)).get_socket_path(),
        None
    );

    let path = PathBuf::from("/a/certain/path");
    assert_eq!(
        UnrestrictedVmmExecutor::new(FirecrackerArguments::new(FirecrackerApiSocket::Enabled(path.clone())))
            .get_socket_path(),
        Some(path)
    );
}

#[tokio::test]
async fn unrestricted_executor_prepare_runs_with_existing_resources() {
    let existing_path = get_tmp_path();
    File::create_new(&existing_path).await.unwrap();

    let executor = UnrestrictedVmmExecutor::new(FirecrackerArguments::new(FirecrackerApiSocket::Disabled));
    let mut expected_hash_map = HashMap::new();
    expected_hash_map.insert(existing_path.clone(), existing_path.clone());
    assert_eq!(
        executor
            .prepare(&get_shell_spawner(), vec![existing_path.clone()])
            .await
            .unwrap(),
        expected_hash_map
    );

    remove_file(existing_path).await.unwrap();
}

#[tokio::test]
async fn unrestricted_executor_prepare_fails_with_missing_resources() {
    let path = get_tmp_path();
    let executor = UnrestrictedVmmExecutor::new(FirecrackerArguments::new(FirecrackerApiSocket::Disabled));
    assert_matches!(
        executor.prepare(&get_shell_spawner(), vec![path.clone()]).await,
        Err(VmmExecutorError::ExpectedResourceMissing(_))
    );
}

#[tokio::test]
async fn unrestricted_executor_prepare_removes_pre_existing_api_socket() {
    let socket_path = get_tmp_path();
    File::create_new(&socket_path).await.unwrap();
    let executor = UnrestrictedVmmExecutor::new(FirecrackerArguments::new(FirecrackerApiSocket::Enabled(
        socket_path.clone(),
    )));
    executor.prepare(&get_shell_spawner(), vec![]).await.unwrap();
    assert!(!try_exists(socket_path).await.unwrap());
}

#[tokio::test]
async fn unrestricted_executor_prepare_creates_log_file() {
    let log_path = get_tmp_path();
    let executor =
        UnrestrictedVmmExecutor::new(FirecrackerArguments::new(FirecrackerApiSocket::Disabled).log_path(&log_path));
    executor.prepare(&get_shell_spawner(), vec![]).await.unwrap();
    assert!(try_exists(&log_path).await.unwrap());
    remove_file(log_path).await.unwrap();
}

#[tokio::test]
async fn unrestricted_executor_prepare_creates_metrics_file() {
    let metrics_path = get_tmp_path();
    let executor = UnrestrictedVmmExecutor::new(
        FirecrackerArguments::new(FirecrackerApiSocket::Disabled).metrics_path(&metrics_path),
    );
    executor.prepare(&get_shell_spawner(), vec![]).await.unwrap();
    assert!(try_exists(&metrics_path).await.unwrap());
    remove_file(metrics_path).await.unwrap();
}

#[tokio::test]
async fn unrestricted_executor_invoke_reports_shell_spawner_error() {
    let executor = UnrestrictedVmmExecutor::new(FirecrackerArguments::new(FirecrackerApiSocket::Disabled));
    assert_matches!(
        executor
            .invoke(
                &FailingShellSpawner::default(),
                &get_mock_firecracker_installation(),
                FirecrackerConfigOverride::NoOverride,
            )
            .await,
        Err(VmmExecutorError::ShellSpawnFailed(_))
    );
}

#[tokio::test]
async fn unrestricted_executor_invoke_applies_command_modifier_chain() {
    let executor = UnrestrictedVmmExecutor::new(FirecrackerArguments::new(FirecrackerApiSocket::Disabled))
        .command_modifier(RewriteCommandModifier::new("echo"))
        .command_modifier(AppendCommandModifier::new(" test"));
    let child = executor
        .invoke(
            &get_shell_spawner(),
            &get_mock_firecracker_installation(),
            FirecrackerConfigOverride::NoOverride,
        )
        .await
        .unwrap();
    let output = child.wait_with_output().await.unwrap();
    assert!(output.status.success());
    let stdout_str = String::from_utf8(output.stdout).unwrap();
    assert_eq!(stdout_str, "test\n");
}

#[tokio::test]
async fn unrestricted_executor_cleanup_removes_api_socket() {
    let socket_path = get_tmp_path();
    File::create_new(&socket_path).await.unwrap();
    let executor = UnrestrictedVmmExecutor::new(FirecrackerArguments::new(FirecrackerApiSocket::Enabled(
        socket_path.clone(),
    )));
    executor.cleanup(&get_shell_spawner()).await.unwrap();
    assert!(!try_exists(socket_path).await.unwrap());
}

#[tokio::test]
async fn unrestricted_executor_cleanup_removes_log_and_metrics_file() {
    let log_path = get_tmp_path();
    let metrics_path = get_tmp_path();
    File::create_new(&log_path).await.unwrap();
    File::create_new(&metrics_path).await.unwrap();

    let executor = UnrestrictedVmmExecutor::new(
        FirecrackerArguments::new(FirecrackerApiSocket::Disabled)
            .log_path(&log_path)
            .metrics_path(&metrics_path),
    )
    .remove_logs_on_cleanup()
    .remove_metrics_on_cleanup();
    executor.cleanup(&get_shell_spawner()).await.unwrap();

    assert!(!try_exists(log_path).await.unwrap());
    assert!(!try_exists(metrics_path).await.unwrap());
}

#[tokio::test]
async fn unrestricted_executor_cleanup_does_not_remove_log_and_metrics_files() {
    let (log_path, metrics_path) = (get_tmp_path(), get_tmp_path());
    File::create_new(&log_path).await.unwrap();
    File::create_new(&metrics_path).await.unwrap();

    let executor = UnrestrictedVmmExecutor::new(
        FirecrackerArguments::new(FirecrackerApiSocket::Disabled)
            .log_path(&log_path)
            .metrics_path(&metrics_path),
    );
    executor.cleanup(&get_shell_spawner()).await.unwrap();

    assert!(try_exists(&log_path).await.unwrap());
    assert!(try_exists(&metrics_path).await.unwrap());
    remove_file(log_path).await.unwrap();
    remove_file(metrics_path).await.unwrap();
}
