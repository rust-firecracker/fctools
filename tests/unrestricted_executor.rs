use std::{os::unix::fs::FileTypeExt, sync::Arc};

use fctools::{
    process_spawner::DirectProcessSpawner,
    runtime::tokio::TokioRuntime,
    vmm::{
        arguments::{VmmApiSocket, VmmArguments},
        executor::{unrestricted::UnrestrictedVmmExecutor, VmmExecutor, VmmExecutorContext},
        ownership::VmmOwnershipModel,
    },
};
use test_framework::{get_fake_firecracker_installation, get_tmp_path};
use tokio::fs::{metadata, remove_file, try_exists, File};

mod test_framework;

#[test]
fn unrestricted_executor_gets_socket_path() {
    let mut executor = UnrestrictedVmmExecutor::new(VmmArguments::new(VmmApiSocket::Disabled));
    assert!(executor.get_socket_path(&get_fake_firecracker_installation()).is_none());

    let path = get_tmp_path();
    executor = UnrestrictedVmmExecutor::new(VmmArguments::new(VmmApiSocket::Enabled(path.clone())));
    assert_eq!(
        executor.get_socket_path(&get_fake_firecracker_installation()),
        Some(path)
    );
}

#[test]
fn unrestricted_executor_transforms_local_to_effective_path() {
    let executor = UnrestrictedVmmExecutor::new(VmmArguments::new(VmmApiSocket::Disabled));
    let path = get_tmp_path();
    assert_eq!(
        executor.local_to_effective_path(&get_fake_firecracker_installation(), path.clone()),
        path
    );
}

#[tokio::test]
async fn unrestricted_executor_prepare_removes_existing_api_socket() {
    let socket_path = get_tmp_path();
    File::create(&socket_path).await.unwrap();
    let (mut executor, context, resource_references) =
        configure_with_args(VmmArguments::new(VmmApiSocket::Enabled(socket_path.clone())));
    executor.prepare(context, resource_references).await.unwrap();
    assert!(!try_exists(socket_path).await.unwrap());
}

#[tokio::test]
async fn unrestricted_executor_prepare_applies_moved_resource() {
    let path = get_tmp_path();
    File::create(&path).await.unwrap();
    let mut resource = MovedVmmResource::new(path.clone(), VmmResourceMoveMethod::Copy);
    let (mut executor, context, mut resource_references) = configure();
    resource_references.moved_resources.push(&mut resource);
    executor.prepare(context, resource_references).await.unwrap();
    assert!(try_exists(&path).await.unwrap());
    assert_eq!(resource.effective_path(), path);
    assert_eq!(resource.local_path(), path);
    remove_file(path).await.unwrap();
}

#[tokio::test]
async fn unrestricted_executor_prepare_applies_log_created_resource() {
    for resource_type in [CreatedVmmResourceType::File, CreatedVmmResourceType::Fifo] {
        let log_path = get_tmp_path();
        let (mut executor, context, resource_references) = configure_with_args(
            VmmArguments::new(VmmApiSocket::Disabled).logs(CreatedVmmResource::new(log_path.clone(), resource_type)),
        );
        executor.prepare(context, resource_references).await.unwrap();
        assert!(
            metadata(&log_path).await.unwrap().file_type().is_fifo() == (resource_type == CreatedVmmResourceType::Fifo)
        );
        remove_file(log_path).await.unwrap();
    }
}

#[tokio::test]
async fn unrestricted_executor_prepare_applies_metrics_created_resource() {
    for resource_type in [CreatedVmmResourceType::File, CreatedVmmResourceType::Fifo] {
        let metrics_path = get_tmp_path();
        let (mut executor, context, resource_references) = configure_with_args(
            VmmArguments::new(VmmApiSocket::Disabled)
                .metrics(CreatedVmmResource::new(metrics_path.clone(), resource_type)),
        );
        executor.prepare(context, resource_references).await.unwrap();
        assert!(
            metadata(&metrics_path).await.unwrap().file_type().is_fifo()
                == (resource_type == CreatedVmmResourceType::Fifo)
        );
        remove_file(metrics_path).await.unwrap();
    }
}

#[tokio::test]
async fn unrestricted_executor_prepare_applies_created_resource() {
    let path = get_tmp_path();
    let (mut executor, context, mut resource_references) = configure();
    let mut resource = CreatedVmmResource::new(&path, CreatedVmmResourceType::File);
    resource_references.created_resources.push(&mut resource);
    executor.prepare(context, resource_references).await.unwrap();
    assert!(try_exists(&path).await.unwrap());
    assert_eq!(resource.effective_path(), path);
    remove_file(path).await.unwrap();
}

#[tokio::test]
async fn unrestricted_executor_prepare_applies_produced_resource() {
    let path = get_tmp_path();
    let mut resource = ProducedVmmResource::new(path.clone());
    let (mut executor, context, mut resource_references) = configure();
    resource_references.produced_resources.push(&mut resource);
    executor.prepare(context, resource_references).await.unwrap();
    assert_eq!(resource.effective_path(), path);
}

fn configure<'r>() -> (
    UnrestrictedVmmExecutor,
    VmmExecutorContext<DirectProcessSpawner, TokioRuntime>,
    VmmResourceReferences<'r>,
) {
    configure_with_args(VmmArguments::new(VmmApiSocket::Disabled))
}

fn configure_with_args<'r>(
    arguments: VmmArguments,
) -> (
    UnrestrictedVmmExecutor,
    VmmExecutorContext<DirectProcessSpawner, TokioRuntime>,
    VmmResourceReferences<'r>,
) {
    let executor = UnrestrictedVmmExecutor::new(arguments);
    let context = VmmExecutorContext {
        installation: Arc::new(get_fake_firecracker_installation()),
        process_spawner: DirectProcessSpawner,
        runtime: TokioRuntime,
        ownership_model: VmmOwnershipModel::Shared,
    };
    let resource_references = VmmResourceReferences::new();

    (executor, context, resource_references)
}
