use std::path::PathBuf;

use common::{get_shell_spawner, get_tmp_path, jail_join};
use fctools::executor::{
    arguments::{FirecrackerApiSocket, FirecrackerArguments, JailerArguments},
    jailed::{FlatJailRenamer, JailedVmmExecutor},
    VmmExecutor,
};
use rand::RngCore;
use tokio::fs::{create_dir_all, metadata, remove_dir_all, try_exists, File};

mod common;

#[test]
fn jailed_executor_get_socket_path_returns_valid_path() {
    assert_eq!(setup_executor(None, None, None, None).0.get_socket_path(), None);

    let path = get_tmp_path();
    let (executor, jail_path) = setup_executor(None, Some(path.clone()), None, None);
    assert_eq!(executor.get_socket_path(), Some(jail_join(jail_path, path)));
}

#[test]
fn jailed_executor_inner_to_outer_path_should_perform_transformation() {
    let (executor, jail_path) = setup_executor(None, None, None, None);
    for path in ["/a", "/b/c", "/d/e/f", "/g/h/i/j"].iter().map(PathBuf::from) {
        assert_eq!(executor.inner_to_outer_path(&path), jail_join(&jail_path, path));
    }
}

#[tokio::test]
async fn jailed_executor_prepare_creates_chroot_base_dir() {
    let chroot_base_dir = get_tmp_path();
    let (executor, _) = setup_executor(Some(chroot_base_dir.clone()), None, None, None);
    executor.prepare(&get_shell_spawner(), vec![]).await.unwrap();
    assert!(metadata(&chroot_base_dir).await.unwrap().is_dir());
    remove_dir_all(chroot_base_dir).await.unwrap();
}

#[tokio::test]
async fn jailed_executor_prepare_defaults_to_srv_jailer() {
    let (executor, jail_path) = setup_executor(None, None, None, None);
    executor.prepare(&get_shell_spawner(), vec![]).await.unwrap();
    assert!(metadata(&jail_path).await.unwrap().is_dir());
    remove_dir_all(&jail_path).await.unwrap();
}

#[tokio::test]
async fn jailed_executor_prepare_deletes_existing_jail() {
    let (executor, jail_path) = setup_executor(None, None, None, None);
    create_dir_all(&jail_path).await.unwrap();
    let jail_inner_path = jail_path.join("some_file.txt");
    File::create_new(&jail_inner_path).await.unwrap();

    executor.prepare(&get_shell_spawner(), vec![]).await.unwrap();
    assert!(!try_exists(jail_inner_path).await.unwrap());
    assert!(try_exists(&jail_path).await.unwrap());
    remove_dir_all(jail_path).await.unwrap();
}

#[tokio::test]
async fn jailed_executor_prepare_creates_socket_parent_directory() {
    let (executor, jail_path) = setup_executor(None, Some(PathBuf::from("/parent_dir/socket")), None, None);
    executor.prepare(&get_shell_spawner(), vec![]).await.unwrap();
    assert!(try_exists(jail_path.join("parent_dir")).await.unwrap());
    remove_dir_all(jail_path).await.unwrap();
}

#[tokio::test]
async fn jailed_executor_prepare_creates_log_file() {
    let (executor, jail_path) = setup_executor(None, None, Some(PathBuf::from("/log.txt")), None);
    executor.prepare(&get_shell_spawner(), vec![]).await.unwrap();
    assert!(try_exists(jail_path.join("log.txt")).await.unwrap());
    remove_dir_all(jail_path).await.unwrap();
}

#[tokio::test]
async fn jailed_executor_prepare_creates_metrics_file() {
    let (executor, jail_path) = setup_executor(None, None, None, Some(PathBuf::from("/metrics.txt")));
    executor.prepare(&get_shell_spawner(), vec![]).await.unwrap();
    assert!(try_exists(jail_path.join("metrics.txt")).await.unwrap());
    remove_dir_all(jail_path).await.unwrap();
}

fn setup_executor(
    chroot_base_dir: Option<PathBuf>,
    socket_path: Option<PathBuf>,
    log_path: Option<PathBuf>,
    metrics_path: Option<PathBuf>,
) -> (JailedVmmExecutor<FlatJailRenamer>, PathBuf) {
    let jail_id = rand::thread_rng().next_u32().to_string();
    let actual_chroot_base_dir = chroot_base_dir.clone().or(Some(PathBuf::from("/srv/jailer"))).unwrap();
    let mut jailer_arguments = JailerArguments::new(unsafe { libc::geteuid() }, unsafe { libc::getegid() }, &jail_id);
    if let Some(chroot_base_dir) = chroot_base_dir {
        jailer_arguments = jailer_arguments.chroot_base_dir(chroot_base_dir);
    }
    let mut firecracker_arguments =
        FirecrackerArguments::new(socket_path.map_or(FirecrackerApiSocket::Disabled, FirecrackerApiSocket::Enabled));
    if let Some(log_path) = log_path {
        firecracker_arguments = firecracker_arguments.log_path(log_path);
    }
    if let Some(metrics_path) = metrics_path {
        firecracker_arguments = firecracker_arguments.metrics_path(metrics_path);
    }

    let executor = JailedVmmExecutor::new(firecracker_arguments, jailer_arguments, FlatJailRenamer::default());

    let jail_path = actual_chroot_base_dir.join("firecracker").join(jail_id).join("root");
    (executor, jail_path)
}
