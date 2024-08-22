use std::{path::PathBuf, vec};

use assert_matches::assert_matches;
use fctools::{
    executor::{
        arguments::{FirecrackerApiSocket, FirecrackerArguments, JailerArguments},
        jailed::{FlatJailRenamer, JailedVmmExecutor},
        unrestricted::UnrestrictedVmmExecutor,
        FirecrackerExecutorError, VmmExecutor,
    },
    shell_spawner::SameUserShellSpawner,
};
use rand::RngCore;
use tokio::{fs, io::AsyncWriteExt};
use uuid::Uuid;

fn get_unrestricted_executor() -> UnrestrictedVmmExecutor {
    UnrestrictedVmmExecutor::new(FirecrackerArguments::new(FirecrackerApiSocket::Disabled))
}

fn get_jailed_executor<A, B>(fc_arg_closure: A, j_arg_closure: B) -> (JailedVmmExecutor<FlatJailRenamer>, String)
where
    A: FnOnce(FirecrackerArguments) -> FirecrackerArguments,
    B: FnOnce(JailerArguments) -> JailerArguments,
{
    let firecracker_arguments = fc_arg_closure(FirecrackerArguments::new(FirecrackerApiSocket::Disabled));
    let jail_id = rand::thread_rng().next_u32().to_string();
    let jailer_arguments =
        j_arg_closure(JailerArguments::new(1000, 1000, jail_id.clone()).chroot_base_dir("/tmp/jail_root"));
    (
        JailedVmmExecutor::new(firecracker_arguments, jailer_arguments, FlatJailRenamer::default()),
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
    let enabled_socket = UnrestrictedVmmExecutor::new(FirecrackerArguments::new(FirecrackerApiSocket::Enabled(
        PathBuf::from("/tmp/socket.sock"),
    )));
    assert_eq!(
        enabled_socket.get_socket_path(),
        Some(PathBuf::from("/tmp/socket.sock"))
    );

    let disabled_socket = get_unrestricted_executor();
    assert_eq!(disabled_socket.get_socket_path(), None);
}

#[tokio::test]
async fn unrestricted_executor_prepare_deletes_socket() {
    fs::File::create("/tmp/some_socket.sock").await.unwrap();
    let executor = UnrestrictedVmmExecutor::new(FirecrackerArguments::new(FirecrackerApiSocket::Enabled(
        PathBuf::from("/tmp/some_socket.sock"),
    )));
    executor.prepare(&get_shell_spawner(), vec![]).await.unwrap();
    assert!(!fs::try_exists("/tmp/some_socket.sock").await.unwrap());
}

#[tokio::test]
async fn unrestricted_executor_prepare_ensures_resources_exist() {
    let executor = get_unrestricted_executor();
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
    let executor = UnrestrictedVmmExecutor::new(FirecrackerArguments::new(FirecrackerApiSocket::Enabled(path.clone())));
    executor.cleanup(&get_shell_spawner()).await.unwrap();
    assert!(!fs::try_exists(&path).await.unwrap());
}

#[tokio::test]
async fn jailed_executor_prepare_creates_chroot_base_dir() {
    let chroot_base_dir: PathBuf = format!("/tmp/chroot_base_dir/{}", rand::thread_rng().next_u32()).into();
    let (executor, _id) = get_jailed_executor(|x| x, |x| x.chroot_base_dir(&chroot_base_dir));
    executor.prepare(&get_shell_spawner(), vec![]).await.unwrap();
    assert!(fs::try_exists(chroot_base_dir).await.unwrap());
}

#[tokio::test]
async fn jailed_executor_prepare_creates_jail() {
    let (executor, id) = get_jailed_executor(|x| x, |x| x);
    executor.prepare(&get_shell_spawner(), vec![]).await.unwrap();
    assert!(fs::try_exists(format!("/tmp/jail_root/firecracker/{id}/root"))
        .await
        .unwrap());
}

#[tokio::test]
async fn jailed_executor_prepare_creates_socket_parent() {
    let (executor, id) = get_jailed_executor(
        |_| FirecrackerArguments::new(FirecrackerApiSocket::Enabled(PathBuf::from("/opt/fc.sock"))),
        |x| x,
    );
    executor.prepare(&get_shell_spawner(), vec![]).await.unwrap();
    assert!(fs::try_exists(format!("/tmp/jail_root/firecracker/{id}/root/opt"))
        .await
        .unwrap());
}

#[tokio::test]
async fn jailed_executor_handles_socket_in_root() {
    let (executor, _id) = get_jailed_executor(
        |_| FirecrackerArguments::new(FirecrackerApiSocket::Enabled(PathBuf::from("/fc.sock"))),
        |x| x,
    );
    executor.prepare(&get_shell_spawner(), vec![]).await.unwrap();
}

#[tokio::test]
async fn jailed_executor_prepare_copies_outer_paths_according_to_renamer() {
    let (executor, id) = get_jailed_executor(|x| x, |x| x);
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
    let (executor, id) = get_jailed_executor(|x| x, |x| x);
    fs::create_dir_all(format!("/tmp/jail_root/firecracker/{id}/root/inner_dir"))
        .await
        .unwrap();
    executor.cleanup(&get_shell_spawner()).await.unwrap();
    assert!(!fs::try_exists(format!("/tmp/jail_root/firecracker/{id}"))
        .await
        .unwrap());
}
