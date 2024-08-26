use std::{os::unix::fs::FileTypeExt, sync::Arc};

use fctools::{
    executor::{
        arguments::{FirecrackerApiSocket, FirecrackerArguments, JailerArguments},
        jailed::{FlatJailRenamer, JailedVmmExecutor},
        unrestricted::UnrestrictedVmmExecutor,
    },
    vm::{
        api::{VmApi, VmSnapshot},
        configuration::NewVmBootMethod,
        models::{VmCreateSnapshot, VmLogger, VmMetricsSystem, VmVsock},
        VmShutdownMethod, VmState,
    },
};
use futures_util::FutureExt;
use rand::RngCore;
use test_framework::{
    env_get_boot_socket_wait, env_get_boot_wait, get_real_firecracker_installation, get_tmp_path, shutdown_test_vm,
    NewVmBuilder, SnapshottingContext, TestExecutor, TestVm,
};
use tokio::{
    fs::{metadata, try_exists},
    io::{AsyncBufReadExt, BufReader},
};

mod test_framework;

#[test]
fn vm_can_boot_via_api_calls() {
    NewVmBuilder::new()
        .boot_method(NewVmBootMethod::ViaApiCalls)
        .run(|mut vm| async move {
            shutdown_test_vm(&mut vm, VmShutdownMethod::CtrlAltDel).await;
        });
}

#[test]
fn vm_can_boot_via_json() {
    NewVmBuilder::new()
        .boot_method(NewVmBootMethod::ViaJsonConfiguration(get_tmp_path()))
        .run(|mut vm| async move {
            shutdown_test_vm(&mut vm, VmShutdownMethod::CtrlAltDel).await;
        });
}

#[test]
fn vm_can_be_shut_down_via_kill() {
    NewVmBuilder::new().run(|mut vm| async move {
        shutdown_test_vm(&mut vm, VmShutdownMethod::Kill).await;
    });
}

#[test]
fn vm_can_be_shut_down_via_pause_then_kill() {
    NewVmBuilder::new().run(|mut vm| async move {
        shutdown_test_vm(&mut vm, VmShutdownMethod::PauseThenKill).await;
    });
}

#[test]
fn vm_processes_logger_path() {
    NewVmBuilder::new()
        .logger(VmLogger::new().log_path(get_tmp_path()))
        .run(|mut vm| async move {
            let log_path = vm.standard_paths().get_log_path().unwrap().clone();
            assert!(metadata(&log_path).await.unwrap().is_file());
            shutdown_test_vm(&mut vm, VmShutdownMethod::CtrlAltDel).await;
            assert!(!try_exists(log_path).await.unwrap());
        });
}

#[test]
fn vm_processes_metrics_path() {
    NewVmBuilder::new()
        .metrics_system(VmMetricsSystem::new(get_tmp_path()))
        .run(|mut vm| async move {
            let metrics_path = vm.standard_paths().get_metrics_path().unwrap().clone();
            assert!(metadata(&metrics_path).await.unwrap().is_file());
            shutdown_test_vm(&mut vm, VmShutdownMethod::CtrlAltDel).await;
            assert!(!try_exists(metrics_path).await.unwrap());
        })
}

#[test]
fn vm_processes_vsock_multiplexer_path() {
    NewVmBuilder::new()
        .vsock(VmVsock::new(rand::thread_rng().next_u32(), get_tmp_path()))
        .run(|mut vm| async move {
            let vsock_multiplexer_path = vm.standard_paths().get_vsock_multiplexer_path().unwrap().clone();
            assert!(metadata(&vsock_multiplexer_path).await.unwrap().file_type().is_socket());
            shutdown_test_vm(&mut vm, VmShutdownMethod::CtrlAltDel).await;
            assert!(!try_exists(vsock_multiplexer_path).await.unwrap());
        });
}

#[test]
fn vm_processes_vsock_listener_paths() {
    NewVmBuilder::new().run(|mut vm| async move {
        assert!(vm.standard_paths().get_vsock_listener_paths().is_empty());
        let expected_path = get_tmp_path();
        vm.standard_paths_mut().add_vsock_listener_path(&expected_path);
        assert!(vm.standard_paths().get_vsock_listener_paths().contains(&expected_path));
        shutdown_test_vm(&mut vm, VmShutdownMethod::CtrlAltDel).await;
    });
}

#[test]
fn vm_translates_inner_to_outer_paths() {
    NewVmBuilder::new().run(|mut vm| async move {
        let inner_path = get_tmp_path();
        let outer_path = vm.inner_to_outer_path(&inner_path);
        assert!(inner_path == outer_path || outer_path.to_str().unwrap().ends_with(inner_path.to_str().unwrap()));
        shutdown_test_vm(&mut vm, VmShutdownMethod::CtrlAltDel).await;
    });
}

#[test]
fn vm_can_take_pipes() {
    NewVmBuilder::new()
        .pre_start_hook(|vm| {
            async {
                vm.take_pipes().unwrap_err(); // cannot take out pipes before start
            }
            .boxed()
        })
        .run(|mut vm| async move {
            let pipes = vm.take_pipes().unwrap();
            vm.take_pipes().unwrap_err(); // cannot take out pipes twice
            let mut buf = String::new();
            let mut buf_reader = BufReader::new(pipes.stdout).lines();
            shutdown_test_vm(&mut vm, VmShutdownMethod::CtrlAltDel).await;

            while let Ok(Some(line)) = buf_reader.next_line().await {
                buf.push_str(&line);
            }

            assert!(buf.contains("Artificially kick devices."));
            assert!(buf.contains("Firecracker exiting successfully. exit_code=0"));
        });
}

#[test]
fn vm_tracks_state_with_graceful_exit() {
    NewVmBuilder::new()
        .pre_start_hook(|vm| {
            async {
                assert_eq!(vm.state(), VmState::NotStarted);
            }
            .boxed()
        })
        .run(|mut vm| async move {
            assert_eq!(vm.state(), VmState::Running);
            shutdown_test_vm(&mut vm, VmShutdownMethod::CtrlAltDel).await;
            assert_eq!(vm.state(), VmState::Exited);
        });
}

#[test]
fn vm_tracks_state_with_crash() {
    NewVmBuilder::new().run(|mut vm| async move {
        assert_eq!(vm.state(), VmState::Running);
        shutdown_test_vm(&mut vm, VmShutdownMethod::CtrlAltDel).await;
    });
}

#[test]
fn vm_can_snapshot_while_original_is_running() {
    NewVmBuilder::new().run_with_snapshotting_context(|mut vm, snapshotting_context| async move {
        vm.api_pause().await.unwrap();
        let snapshot = vm
            .api_create_snapshot(VmCreateSnapshot::new(get_tmp_path(), get_tmp_path()))
            .await
            .unwrap();

        restore_vm_from_snapshot(snapshot.clone(), snapshotting_context).await;

        vm.api_resume().await.unwrap();
        shutdown_test_vm(&mut vm, VmShutdownMethod::CtrlAltDel).await;

        assert!(!tokio::fs::try_exists(snapshot.get_snapshot_path()).await.unwrap());
        assert!(!tokio::fs::try_exists(snapshot.get_mem_file_path()).await.unwrap());
    });
}

#[test]
fn vm_can_snapshot_after_original_has_exited() {
    NewVmBuilder::new().run_with_snapshotting_context(|mut vm, snapshotting_context| async move {
        vm.api_pause().await.unwrap();
        let mut snapshot = vm
            .api_create_snapshot(VmCreateSnapshot::new(get_tmp_path(), get_tmp_path()))
            .await
            .unwrap();
        snapshot.copy(get_tmp_path(), get_tmp_path()).await.unwrap();
        vm.api_resume().await.unwrap();
        shutdown_test_vm(&mut vm, VmShutdownMethod::CtrlAltDel).await;

        restore_vm_from_snapshot(snapshot.clone(), snapshotting_context).await;
        snapshot.remove().await.unwrap();
    });
}

async fn restore_vm_from_snapshot(snapshot: VmSnapshot, snapshotting_context: SnapshottingContext) {
    let executor = match snapshotting_context.is_jailed {
        true => TestExecutor::Jailed(JailedVmmExecutor::new(
            FirecrackerArguments::new(FirecrackerApiSocket::Enabled(get_tmp_path())),
            JailerArguments::new(
                unsafe { libc::geteuid() },
                unsafe { libc::getegid() },
                rand::thread_rng().next_u32().to_string(),
            ),
            FlatJailRenamer::default(),
        )),
        false => TestExecutor::Unrestricted(UnrestrictedVmmExecutor::new(FirecrackerArguments::new(
            FirecrackerApiSocket::Enabled(get_tmp_path()),
        ))),
    };

    let mut vm = TestVm::prepare_arced(
        Arc::new(executor),
        snapshotting_context.shell_spawner,
        Arc::new(get_real_firecracker_installation()),
        snapshot.into_configuration(true),
    )
    .await
    .unwrap();
    vm.start(env_get_boot_socket_wait()).await.unwrap();
    tokio::time::sleep(env_get_boot_wait()).await;

    vm.api_get_info().await.unwrap();
    shutdown_test_vm(&mut vm, VmShutdownMethod::CtrlAltDel).await;
}
