use std::{path::PathBuf, vec};

use assert_matches::assert_matches;
use fctools::{
    executor::{
        arguments::{FirecrackerApiSocket, FirecrackerArguments, JailerArguments},
        FirecrackerExecutorError, FlatJailRenamer, JailMoveMethod, JailedVmmExecutor, UnrestrictedVmmExecutor,
        VmmExecutor,
    },
    shell::SameUserShellSpawner,
};
use rand::RngCore;
use tokio::{fs, io::AsyncWriteExt};
use uuid::Uuid;

fn get_unrestricted_executor() -> UnrestrictedVmmExecutor {
    let firecracker_arguments = FirecrackerArguments::new(FirecrackerApiSocket::Disabled);
    UnrestrictedVmmExecutor { firecracker_arguments }
}

fn get_jailed_executor() -> (JailedVmmExecutor<FlatJailRenamer>, String) {
    let firecracker_arguments = FirecrackerArguments::new(FirecrackerApiSocket::Disabled);
    let jail_id = rand::thread_rng().next_u32().to_string();
    let jailer_arguments = JailerArguments::new(1000, 1000, jail_id.clone()).chroot_base_dir("/tmp/jail_root");
    (
        JailedVmmExecutor {
            firecracker_arguments,
            jailer_arguments,
            jail_move_method: JailMoveMethod::Copy,
            jail_renamer: FlatJailRenamer::default(),
        },
        jail_id,
    )
}

fn get_shell_spawner() -> SameUserShellSpawner {
    SameUserShellSpawner {
        shell_path: PathBuf::from("/usr/bin/sh"),
    }
}

fn make_outer_path() -> PathBuf {
    format!("/tmp/{}", Uuid::new_v4()).into()
}

#[test]
fn unrestricted_executor_gets_socket_path() {
    let mut enabled_socket = get_unrestricted_executor();
    enabled_socket.firecracker_arguments =
        FirecrackerArguments::new(FirecrackerApiSocket::Enabled(PathBuf::from("/tmp/socket.sock")));
    assert_eq!(
        enabled_socket.get_outer_socket_path(),
        Some(PathBuf::from("/tmp/socket.sock"))
    );

    let disabled_socket = get_unrestricted_executor();
    assert_eq!(disabled_socket.get_outer_socket_path(), None);
}

#[tokio::test]
async fn unrestricted_executor_prepare_deletes_socket() {
    fs::File::create("/tmp/some_socket.sock").await.unwrap();
    let mut executor = get_unrestricted_executor();
    executor.firecracker_arguments =
        FirecrackerArguments::new(FirecrackerApiSocket::Enabled(PathBuf::from("/tmp/some_socket.sock")));
    executor.prepare(&get_shell_spawner(), vec![]).await.unwrap();
    assert!(!fs::try_exists("/tmp/some_socket.sock").await.unwrap());
}

#[tokio::test]
async fn unrestricted_executor_prepare_ensures_resources_exist() {
    let mut executor = get_unrestricted_executor();
    executor.firecracker_arguments = FirecrackerArguments::new(FirecrackerApiSocket::Disabled);
    let path = make_outer_path();
    assert_matches!(
        executor.prepare(&get_shell_spawner(), vec![path.clone()]).await,
        Err(FirecrackerExecutorError::ExpectedResourceMissing(_))
    );

    fs::File::create(&path).await.unwrap();
    executor.prepare(&get_shell_spawner(), vec![path]).await.unwrap();
}

#[tokio::test]
async fn unrestricted_executor_cleanup_deletes_socket() {
    let path = PathBuf::from("/tmp/other_socket.sock");
    fs::File::create(&path).await.unwrap();
    let mut executor = get_unrestricted_executor();
    executor.firecracker_arguments = FirecrackerArguments::new(FirecrackerApiSocket::Enabled(path.clone()));
    executor.cleanup(&get_shell_spawner()).await.unwrap();
    assert!(!fs::try_exists(&path).await.unwrap());
}

#[tokio::test]
async fn jailed_executor_prepare_creates_chroot_base_dir() {
    let (mut executor, _id) = get_jailed_executor();
    let chroot_base_dir: PathBuf = format!("/tmp/chroot_base_dir/{}", rand::thread_rng().next_u32()).into();
    executor.jailer_arguments = executor.jailer_arguments.chroot_base_dir(&chroot_base_dir);
    executor.prepare(&get_shell_spawner(), vec![]).await.unwrap();
    assert!(fs::try_exists(chroot_base_dir).await.unwrap());
}

#[tokio::test]
async fn jailed_executor_prepare_creates_jail() {
    let (executor, id) = get_jailed_executor();
    executor.prepare(&get_shell_spawner(), vec![]).await.unwrap();
    assert!(fs::try_exists(format!("/tmp/jail_root/firecracker/{id}/root"))
        .await
        .unwrap());
}

#[tokio::test]
async fn jailed_executor_prepare_creates_socket_parent() {
    let (mut executor, id) = get_jailed_executor();
    executor.firecracker_arguments =
        FirecrackerArguments::new(FirecrackerApiSocket::Enabled(PathBuf::from("/opt/fc.sock")));
    executor.prepare(&get_shell_spawner(), vec![]).await.unwrap();
    assert!(fs::try_exists(format!("/tmp/jail_root/firecracker/{id}/root/opt"))
        .await
        .unwrap());
}

#[tokio::test]
async fn jailed_executor_handles_socket_in_root() {
    let (mut executor, _id) = get_jailed_executor();
    executor.firecracker_arguments =
        FirecrackerArguments::new(FirecrackerApiSocket::Enabled(PathBuf::from("/fc.sock")));
    executor.prepare(&get_shell_spawner(), vec![]).await.unwrap();
}

#[tokio::test]
async fn jailed_executor_prepare_copies_outer_paths_according_to_renamer() {
    let (executor, id) = get_jailed_executor();
    let mut expect_handle = fs::File::create("/tmp/jail_resource.txt").await.unwrap();
    expect_handle.write_all(b"text").await.unwrap();
    executor
        .prepare(&get_shell_spawner(), vec![PathBuf::from("/tmp/jail_resource.txt")])
        .await
        .unwrap();
    assert_eq!(
        fs::read_to_string(format!("/tmp/jail_root/firecracker/{id}/root/jail_resource.txt"))
            .await
            .unwrap(),
        "text"
    );
}

#[tokio::test]
async fn jailed_executor_cleanup_deletes_jail() {
    let (executor, id) = get_jailed_executor();
    fs::create_dir_all(format!("/tmp/jail_root/firecracker/{id}/root/inner_dir"))
        .await
        .unwrap();
    executor.cleanup(&get_shell_spawner()).await.unwrap();
    assert!(!fs::try_exists(format!("/tmp/jail_root/firecracker/{id}"))
        .await
        .unwrap());
}
