use std::{os::unix::fs::FileTypeExt, time::Duration};

use fctools::{
    process_spawner::DirectProcessSpawner,
    runtime::tokio::TokioRuntime,
    vm::{
        api::VmApi,
        configuration::InitMethod,
        models::{CreateSnapshot, LoggerSystem, MetricsSystem},
        shutdown::{VmShutdownAction, VmShutdownMethod},
        snapshot::SnapshotData,
        VmState,
    },
    vmm::{
        arguments::{jailer::JailerArguments, VmmApiSocket, VmmArguments},
        executor::{
            either::EitherVmmExecutor,
            jailed::{renamer::FlatJailRenamer, JailedVmmExecutor},
            unrestricted::UnrestrictedVmmExecutor,
        },
        ownership::VmmOwnershipModel,
    },
};
use futures_util::{io::BufReader, AsyncBufReadExt, StreamExt};
use rand::RngCore;
use test_framework::{
    get_real_firecracker_installation, get_tmp_fifo_path, get_tmp_path, shutdown_test_vm, TestOptions, TestVm,
    VmBuilder,
};
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
                .shutdown(VmShutdownAction {
                    method: method.clone(),
                    timeout: None,
                    graceful: true,
                })
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
    VmBuilder::new()
        .logger_system(LoggerSystem::new().log_path(get_tmp_fifo_path()))
        .run(|mut vm| async move {
            let log_path = vm.get_accessible_paths().log_path.clone().unwrap();
            assert!(metadata(&log_path).await.unwrap().file_type().is_fifo());
            shutdown_test_vm(&mut vm).await;
            assert!(!try_exists(log_path).await.unwrap());
        });
}

#[test]
fn vm_processes_logger_path_as_plaintext() {
    VmBuilder::new()
        .logger_system(LoggerSystem::new().log_path(get_tmp_path()))
        .run(|mut vm| async move {
            let log_path = vm.get_accessible_paths().log_path.clone().unwrap();
            let file_type = metadata(&log_path).await.unwrap().file_type();
            assert!(file_type.is_file());
            assert!(!file_type.is_fifo());

            shutdown_test_vm(&mut vm).await;
            assert!(!try_exists(log_path).await.unwrap());
        });
}

#[test]
fn vm_processes_metrics_path() {
    VmBuilder::new()
        .metrics_system(MetricsSystem::new(get_tmp_path()))
        .run(|mut vm| async move {
            let metrics_path = vm.get_accessible_paths().metrics_path.clone().unwrap();
            assert!(metadata(&metrics_path).await.unwrap().is_file());
            shutdown_test_vm(&mut vm).await;
            assert!(!try_exists(metrics_path).await.unwrap());
        })
}

#[test]
fn vm_processes_vsock_multiplexer_path() {
    VmBuilder::new().vsock_device().run(|mut vm| async move {
        let vsock_multiplexer_path = vm.get_accessible_paths().vsock_multiplexer_path.clone().unwrap();
        assert!(metadata(&vsock_multiplexer_path).await.unwrap().file_type().is_socket());
        shutdown_test_vm(&mut vm).await;
        assert!(!try_exists(vsock_multiplexer_path).await.unwrap());
    });
}

#[test]
fn vm_processes_vsock_listener_paths() {
    VmBuilder::new().run(|mut vm| async move {
        assert!(vm.get_accessible_paths().vsock_listener_paths.is_empty());
        let expected_path = get_tmp_path();
        vm.register_vsock_listener_path(&expected_path);
        assert!(vm.get_accessible_paths().vsock_listener_paths.contains(&expected_path));
        shutdown_test_vm(&mut vm).await;
    });
}

#[test]
fn vm_translates_inner_to_outer_paths() {
    VmBuilder::new().run(|mut vm| async move {
        let inner_path = get_tmp_path();
        let outer_path = vm.get_accessible_path_from_inner(&inner_path);
        assert!(inner_path == outer_path || outer_path.to_str().unwrap().ends_with(inner_path.to_str().unwrap()));
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
                assert_eq!(vm.state(), VmState::NotStarted);
            })
        })
        .run(|mut vm| async move {
            assert_eq!(vm.state(), VmState::Running);
            shutdown_test_vm(&mut vm).await;
            assert_eq!(vm.state(), VmState::Exited);
        });
}

#[test]
fn vm_tracks_state_with_crash() {
    VmBuilder::new().run(|mut vm| async move {
        assert_eq!(vm.state(), VmState::Running);
        shutdown_test_vm(&mut vm).await;
    });
}

#[test]
fn vm_can_snapshot_while_original_is_running() {
    VmBuilder::new().run_with_is_jailed(|mut vm, is_jailed| async move {
        vm.api_pause().await.unwrap();
        let snapshot = vm
            .api_create_snapshot(CreateSnapshot::new(get_tmp_path(), get_tmp_path()))
            .await
            .unwrap();

        restore_vm_from_snapshot(snapshot.clone(), is_jailed).await;

        vm.api_resume().await.unwrap();
        shutdown_test_vm(&mut vm).await;

        assert!(!tokio::fs::try_exists(snapshot.snapshot_path).await.unwrap());
        assert!(!tokio::fs::try_exists(snapshot.mem_file_path).await.unwrap());
    });
}

#[test]
fn vm_can_snapshot_after_original_has_exited() {
    VmBuilder::new().run_with_is_jailed(|mut vm, is_jailed| async move {
        vm.api_pause().await.unwrap();
        let mut snapshot = vm
            .api_create_snapshot(CreateSnapshot::new(get_tmp_path(), get_tmp_path()))
            .await
            .unwrap();
        snapshot
            .copy::<TokioRuntime>(get_tmp_path(), get_tmp_path())
            .await
            .unwrap();
        vm.api_resume().await.unwrap();
        shutdown_test_vm(&mut vm).await;

        restore_vm_from_snapshot(snapshot.clone(), is_jailed).await;
        snapshot.remove::<TokioRuntime>().await.unwrap();
    });
}

#[test]
fn vm_can_boot_with_simple_networking() {
    VmBuilder::new().simple_networking().run(verify_networking);
}

#[test]
fn vm_can_boot_with_namespaced_networking() {
    VmBuilder::new().namespaced_networking().run(verify_networking);
}

#[test]
fn vm_can_boot_with_vsock_device() {
    VmBuilder::new().vsock_device().run(|mut vm| async move {
        shutdown_test_vm(&mut vm).await;
    });
}

async fn verify_networking(mut vm: TestVm) {
    let configuration = vm.api_get_effective_configuration().await.unwrap();
    assert_eq!(configuration.network_interfaces.len(), 1);
    shutdown_test_vm(&mut vm).await;
}

async fn restore_vm_from_snapshot(snapshot: SnapshotData, is_jailed: bool) {
    let executor = match is_jailed {
        true => EitherVmmExecutor::Jailed(JailedVmmExecutor::new(
            VmmArguments::new(VmmApiSocket::Enabled(get_tmp_path())),
            JailerArguments::new(rand::thread_rng().next_u32().to_string().try_into().unwrap()),
            FlatJailRenamer::default(),
        )),
        false => EitherVmmExecutor::Unrestricted(UnrestrictedVmmExecutor::new(VmmArguments::new(
            VmmApiSocket::Enabled(get_tmp_path()),
        ))),
    };

    let mut vm = TestVm::prepare(
        executor,
        VmmOwnershipModel::Downgraded {
            uid: unsafe { libc::geteuid() },
            gid: unsafe { libc::getegid() },
        },
        DirectProcessSpawner,
        get_real_firecracker_installation(),
        snapshot.into_configuration(Some(true), None),
    )
    .await
    .unwrap();
    vm.start(Duration::from_millis(
        TestOptions::get().await.waits.boot_socket_timeout_ms,
    ))
    .await
    .unwrap();
    tokio::time::sleep(Duration::from_millis(TestOptions::get().await.waits.boot_wait_ms)).await;

    vm.api_get_info().await.unwrap();
    shutdown_test_vm(&mut vm).await;
}
