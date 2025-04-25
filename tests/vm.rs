use std::{os::unix::fs::FileTypeExt, time::Duration};

use fctools::{
    process_spawner::DirectProcessSpawner,
    runtime::tokio::TokioRuntime,
    vm::{
        api::VmApi,
        configuration::InitMethod,
        shutdown::{VmShutdownAction, VmShutdownMethod},
        snapshot::{PrepareVmFromSnapshotOptions, VmSnapshot},
        VmState,
    },
    vmm::{
        arguments::{jailer::JailerArguments, VmmApiSocket, VmmArguments},
        executor::{
            either::EitherVmmExecutor,
            jailed::{FlatVirtualPathResolver, JailedVmmExecutor},
            unrestricted::UnrestrictedVmmExecutor,
        },
        ownership::VmmOwnershipModel,
        resource::{CreatedResourceType, MovedResourceType},
    },
};
use futures_util::{io::BufReader, AsyncBufReadExt, StreamExt};
use rand::RngCore;
use test_framework::{get_create_snapshot, get_tmp_path, shutdown_test_vm, TestOptions, TestVm, VmBuilder};
use tokio::fs::{metadata, try_exists};

mod test_framework;

#[test]
fn vm_can_boot_via_api_calls() {
    VmBuilder::new()
        .init_method(InitMethod::ViaApiCalls)
        .run(|mut vm| async move {
            shutdown_test_vm(&mut vm).await;
        });
}

#[test]
fn vm_can_boot_via_json() {
    VmBuilder::new()
        .init_method(InitMethod::ViaJsonConfiguration(get_tmp_path()))
        .run(|mut vm| async move {
            shutdown_test_vm(&mut vm).await;
        });
}

#[test]
fn vm_can_shut_down_via_ctrl_alt_del() {
    vm_shutdown_test(VmShutdownMethod::CtrlAltDel);
}

#[test]
fn vm_can_be_shut_down_via_pause_then_kill() {
    vm_shutdown_test(VmShutdownMethod::PauseThenKill);
}

#[test]
fn vm_can_be_shut_down_via_kill() {
    vm_shutdown_test(VmShutdownMethod::Kill);
}

fn vm_shutdown_test(method: VmShutdownMethod) {
    VmBuilder::new().run(move |mut vm| {
        let method = method.clone();
        async move {
            let outcome = vm
                .shutdown([VmShutdownAction {
                    method: method.clone(),
                    timeout: None,
                    graceful: true,
                }])
                .await
                .unwrap();
            assert!(method != VmShutdownMethod::CtrlAltDel || outcome.graceful);
            assert!(outcome.errors.is_empty());
            assert_eq!(outcome.index, 0);
            vm.cleanup().await.unwrap();
        }
    });
}

#[test]
fn vm_processes_logger_path_as_fifo() {
    vm_logger_test(CreatedResourceType::Fifo);
}

#[test]
fn vm_processes_logger_path_as_plaintext() {
    vm_logger_test(CreatedResourceType::File);
}

fn vm_logger_test(resource_type: CreatedResourceType) {
    VmBuilder::new()
        .logger_system(resource_type)
        .run(move |mut vm| async move {
            let log_path = vm
                .get_configuration()
                .get_data()
                .logger_system
                .as_ref()
                .unwrap()
                .logs
                .as_ref()
                .unwrap()
                .get_effective_path()
                .unwrap()
                .to_owned();

            let metadata = metadata(&log_path).await.unwrap();
            if resource_type == CreatedResourceType::Fifo {
                assert!(metadata.file_type().is_fifo());
            } else {
                assert!(metadata.is_file() && !metadata.file_type().is_fifo());
            }

            shutdown_test_vm(&mut vm).await;
            assert!(!try_exists(log_path).await.unwrap());
        });
}

#[test]
fn vm_processes_metrics_path_as_plaintext() {
    vm_metrics_test(CreatedResourceType::File);
}

#[test]
fn vm_processes_metrics_path_as_fifo() {
    vm_metrics_test(CreatedResourceType::Fifo);
}

fn vm_metrics_test(resource_type: CreatedResourceType) {
    VmBuilder::new()
        .metrics_system(resource_type)
        .run(move |mut vm| async move {
            let metrics_path = vm
                .get_configuration()
                .get_data()
                .metrics_system
                .as_ref()
                .unwrap()
                .metrics
                .get_effective_path()
                .unwrap()
                .to_owned();

            assert_eq!(
                metadata(&metrics_path).await.unwrap().file_type().is_fifo(),
                resource_type == CreatedResourceType::Fifo
            );
            shutdown_test_vm(&mut vm).await;
            assert!(!try_exists(metrics_path).await.unwrap());
        });
}

#[test]
fn vm_processes_vsock() {
    VmBuilder::new().vsock_device().run(|mut vm| async move {
        let uds_path = vm
            .get_configuration()
            .get_data()
            .vsock_device
            .as_ref()
            .unwrap()
            .uds
            .get_effective_path()
            .unwrap()
            .to_owned();

        assert!(metadata(&uds_path).await.unwrap().file_type().is_socket());
        shutdown_test_vm(&mut vm).await;
        assert!(!try_exists(uds_path).await.unwrap());
    });
}

#[test]
fn vm_translates_local_to_effective_paths() {
    VmBuilder::new().run(|mut vm| async move {
        let local_path = get_tmp_path();
        let effective_path = vm.resolve_effective_path(&local_path);
        assert!(
            local_path == effective_path || effective_path.to_str().unwrap().ends_with(local_path.to_str().unwrap())
        );
        shutdown_test_vm(&mut vm).await;
    });
}

#[test]
fn vm_can_take_pipes() {
    VmBuilder::new()
        .pre_start_hook(|vm| {
            Box::pin(async {
                vm.take_pipes().unwrap_err(); // cannot take out pipes before start
            })
        })
        .no_new_pid_ns()
        .run(|mut vm| async move {
            let pipes = vm.take_pipes().unwrap();
            vm.take_pipes().unwrap_err(); // cannot take out pipes twice
            let mut buf = String::new();
            let mut buf_reader = BufReader::new(pipes.stdout).lines();
            shutdown_test_vm(&mut vm).await;

            while let Some(Ok(line)) = buf_reader.next().await {
                buf.push_str(&line);
            }

            assert!(buf.contains("Artificially kick devices."));
        });
}

#[test]
fn vm_tracks_state_with_graceful_exit() {
    VmBuilder::new()
        .pre_start_hook(|vm| {
            Box::pin(async {
                assert_eq!(vm.get_state(), VmState::NotStarted);
            })
        })
        .run(|mut vm| async move {
            assert_eq!(vm.get_state(), VmState::Running);
            shutdown_test_vm(&mut vm).await;
            assert_eq!(vm.get_state(), VmState::Exited);
        });
}

#[test]
fn vm_tracks_state_with_crash() {
    VmBuilder::new().run(|mut vm| async move {
        assert_eq!(vm.get_state(), VmState::Running);
        shutdown_test_vm(&mut vm).await;
    });
}

#[test]
fn vm_can_snapshot_while_original_is_running() {
    VmBuilder::new().run_with_is_jailed(|mut old_vm, is_jailed| async move {
        old_vm.pause().await.unwrap();
        let create_snapshot = get_create_snapshot(old_vm.get_resource_system_mut());
        let snapshot = old_vm.create_snapshot(create_snapshot).await.unwrap();
        let new_vm = prepare_snapshot_vm(&mut old_vm, snapshot.clone(), is_jailed).await;
        restore_snapshot_vm(new_vm).await;
        old_vm.resume().await.unwrap();
        shutdown_test_vm(&mut old_vm).await;
    });
}

#[test]
fn vm_can_snapshot_after_original_has_exited() {
    VmBuilder::new().run_with_is_jailed(|mut old_vm, is_jailed| async move {
        old_vm.pause().await.unwrap();
        let create_snapshot = get_create_snapshot(old_vm.get_resource_system_mut());
        let mut snapshot = old_vm.create_snapshot(create_snapshot).await.unwrap();
        snapshot
            .copy(&TokioRuntime, get_tmp_path(), get_tmp_path())
            .await
            .unwrap();
        old_vm.resume().await.unwrap();
        let new_vm = prepare_snapshot_vm(&mut old_vm, snapshot.clone(), is_jailed).await;
        shutdown_test_vm(&mut old_vm).await;
        restore_snapshot_vm(new_vm).await;
    });
}

#[test]
fn vm_can_boot_with_simple_networking() {
    VmBuilder::new().simple_networking().run(|mut vm| async move {
        shutdown_test_vm(&mut vm).await;
    });
}

#[test]
fn vm_can_boot_with_namespaced_networking() {
    VmBuilder::new().namespaced_networking().run(|mut vm| async move {
        shutdown_test_vm(&mut vm).await;
    });
}

#[test]
fn vm_can_boot_with_vsock_device() {
    VmBuilder::new().vsock_device().run(|mut vm| async move {
        shutdown_test_vm(&mut vm).await;
    });
}

async fn prepare_snapshot_vm(old_vm: &mut TestVm, snapshot: VmSnapshot, is_jailed: bool) -> TestVm {
    let executor = match is_jailed {
        true => EitherVmmExecutor::Jailed(JailedVmmExecutor::new(
            VmmArguments::new(VmmApiSocket::Enabled(get_tmp_path())),
            JailerArguments::new(rand::rng().next_u32().to_string().try_into().unwrap()),
            FlatVirtualPathResolver::default(),
        )),
        false => EitherVmmExecutor::Unrestricted(UnrestrictedVmmExecutor::new(VmmArguments::new(
            VmmApiSocket::Enabled(get_tmp_path()),
        ))),
    };

    snapshot
        .prepare_vm(
            old_vm,
            PrepareVmFromSnapshotOptions {
                executor,
                process_spawner: DirectProcessSpawner,
                runtime: TokioRuntime,
                moved_resource_type: MovedResourceType::Copied,
                ownership_model: VmmOwnershipModel::Downgraded {
                    uid: TestOptions::get().await.jailer_uid,
                    gid: TestOptions::get().await.jailer_gid,
                },
                enable_diff_snapshots: Some(false),
                resume_vm: Some(true),
            },
        )
        .await
        .unwrap()
}

async fn restore_snapshot_vm(mut new_vm: TestVm) {
    new_vm
        .start(Duration::from_millis(
            TestOptions::get().await.waits.boot_socket_timeout_ms,
        ))
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(TestOptions::get().await.waits.boot_wait_ms)).await;

    new_vm.get_info().await.unwrap();
    shutdown_test_vm(&mut new_vm).await;
}
