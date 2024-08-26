use std::sync::Arc;

use assert_matches::assert_matches;
use bytes::Bytes;
use fctools::{
    executor::{
        arguments::{FirecrackerApiSocket, FirecrackerArguments, JailerArguments},
        jailed::{FlatJailRenamer, JailedVmmExecutor},
        unrestricted::UnrestrictedVmmExecutor,
    },
    process::HyperResponseExt,
    vm::{
        api::{VmApi, VmSnapshot},
        models::{
            VmBalloon, VmCreateSnapshot, VmInfo, VmMetricsSystem, VmReturnedState, VmUpdateBalloon,
            VmUpdateBalloonStatistics,
        },
        VmError, VmShutdownMethod, VmState,
    },
};
use http::{Request, StatusCode};
use http_body_util::Full;
use rand::RngCore;
use test_framework::{
    env_get_boot_socket_wait, env_get_boot_wait, get_real_firecracker_installation, get_tmp_path, shutdown_test_vm,
    NewVmBuilder, SnapshottingContext, TestExecutor, TestVm,
};

mod test_framework;

#[test]
fn vm_api_can_catch_api_errors() {
    NewVmBuilder::new()
        .balloon(VmBalloon::new(64, false))
        .run(|mut vm| async move {
            // trying to set up balloon stats after being disabled pre-boot is a bad request
            let error = vm
                .api_update_balloon_statistics(VmUpdateBalloonStatistics::new(1))
                .await
                .unwrap_err();
            assert_matches!(
                error,
                VmError::ApiRespondedWithFault {
                    status_code: StatusCode::BAD_REQUEST,
                    fault_message: _
                }
            );
            shutdown_test_vm(&mut vm, VmShutdownMethod::CtrlAltDel).await;
        });
}

#[test]
fn vm_api_can_send_custom_request() {
    NewVmBuilder::new().run(|mut vm| async move {
        let mut response = vm
            .api_custom_request(
                "/",
                Request::builder().method("GET").body(Full::new(Bytes::new())).unwrap(),
                None,
            )
            .await
            .unwrap();
        assert!(response.status().is_success());
        serde_json::from_str::<VmInfo>(&response.recv_to_string().await.unwrap()).unwrap();
        shutdown_test_vm(&mut vm, VmShutdownMethod::CtrlAltDel).await;
    });
}

#[test]
fn vm_api_custom_requests_perform_pause_changes() {
    NewVmBuilder::new().run(|mut vm| async move {
        let request = Request::builder().method("GET").body(Full::new(Bytes::new())).unwrap();
        vm.api_custom_request("/", request.clone(), Some(true)).await.unwrap();
        assert_eq!(vm.state(), VmState::Paused);
        vm.api_custom_request("/", request.clone(), Some(false)).await.unwrap();
        assert_eq!(vm.state(), VmState::Running);
        shutdown_test_vm(&mut vm, VmShutdownMethod::CtrlAltDel).await;
    });
}

#[test]
fn vm_api_can_receive_info() {
    NewVmBuilder::new().run(|mut vm| async move {
        let info = vm.api_get_info().await.unwrap();
        assert_eq!(info.app_name, "Firecracker");
        assert_eq!(info.state, VmReturnedState::Running);
        shutdown_test_vm(&mut vm, VmShutdownMethod::CtrlAltDel).await;
    });
}

#[test]
fn vm_api_can_flush_metrics() {
    NewVmBuilder::new()
        .metrics_system(VmMetricsSystem::new(get_tmp_path()))
        .run(|mut vm| async move {
            vm.api_flush_metrics().await.unwrap();
            shutdown_test_vm(&mut vm, VmShutdownMethod::CtrlAltDel).await;
        });
}

#[test]
fn vm_api_can_get_balloon() {
    NewVmBuilder::new()
        .balloon(VmBalloon::new(64, false))
        .run(|mut vm| async move {
            let balloon = vm.api_get_balloon().await.unwrap();
            assert_eq!(balloon.get_stats_polling_interval_s(), 0);
            assert_eq!(balloon.get_amount_mib(), 64);
            assert!(!balloon.get_deflate_on_oom());
            shutdown_test_vm(&mut vm, VmShutdownMethod::CtrlAltDel).await;
        });
}

#[test]
fn vm_api_can_update_balloon() {
    NewVmBuilder::new()
        .balloon(VmBalloon::new(64, false))
        .run(|mut vm| async move {
            vm.api_update_balloon(VmUpdateBalloon::new(50)).await.unwrap();
            let balloon = vm.api_get_balloon().await.unwrap();
            assert_eq!(balloon.get_amount_mib(), 50);
            shutdown_test_vm(&mut vm, VmShutdownMethod::CtrlAltDel).await;
        });
}

#[test]
fn vm_api_can_get_balloon_statistics() {
    NewVmBuilder::new()
        .balloon(VmBalloon::new(64, false).stats_polling_interval_s(1))
        .run(|mut vm| async move {
            let statistics = vm.api_get_balloon_statistics().await.unwrap();
            assert_ne!(statistics.actual_mib, 0);
            shutdown_test_vm(&mut vm, VmShutdownMethod::CtrlAltDel).await;
        });
}

#[test]
fn vm_api_can_update_balloon_statistics() {
    NewVmBuilder::new()
        .balloon(VmBalloon::new(64, false).stats_polling_interval_s(1))
        .run(|mut vm| async move {
            vm.api_update_balloon_statistics(VmUpdateBalloonStatistics::new(3))
                .await
                .unwrap();
            let _ = vm.api_get_balloon_statistics().await.unwrap();
            shutdown_test_vm(&mut vm, VmShutdownMethod::CtrlAltDel).await;
        });
}

#[test]
fn vm_api_can_get_machine_configuration() {
    NewVmBuilder::new().run(|mut vm| async move {
        let machine_configuration = vm.api_get_machine_configuration().await.unwrap();
        assert_eq!(machine_configuration.get_vcpu_count(), 1);
        assert_eq!(machine_configuration.get_mem_size_mib(), 128);
        shutdown_test_vm(&mut vm, VmShutdownMethod::CtrlAltDel).await;
    });
}

#[test]
fn vm_api_can_get_firecracker_version() {
    NewVmBuilder::new().run(|mut vm| async move {
        let firecracker_version = vm.api_get_firecracker_version().await.unwrap();
        assert!(firecracker_version.contains("1"));
        shutdown_test_vm(&mut vm, VmShutdownMethod::CtrlAltDel).await;
    });
}

#[test]
fn vm_api_can_get_effective_configuration() {
    NewVmBuilder::new().run(|mut vm| async move {
        let effective_configuration = vm.api_get_effective_configuration().await.unwrap();
        effective_configuration.boot_source.unwrap();
        effective_configuration.machine_configuration.unwrap();
        assert_eq!(effective_configuration.drives.len(), 1);
        shutdown_test_vm(&mut vm, VmShutdownMethod::CtrlAltDel).await;
    });
}

#[test]
fn vm_api_can_pause_and_resume() {
    NewVmBuilder::new().run(|mut vm| async move {
        vm.api_pause().await.unwrap();
        assert_eq!(vm.state(), VmState::Paused);
        vm.api_resume().await.unwrap();
        assert_eq!(vm.state(), VmState::Running);
        shutdown_test_vm(&mut vm, VmShutdownMethod::CtrlAltDel).await;
    });
}

#[test]
fn vm_api_can_snapshot_while_original_is_running() {
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
fn vm_api_can_snapshot_after_original_has_exited() {
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
