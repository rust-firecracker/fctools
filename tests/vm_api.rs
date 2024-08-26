use assert_matches::assert_matches;
use bytes::Bytes;
use fctools::{
    process::HyperResponseExt,
    vm::{
        api::VmApi,
        models::{VmBalloon, VmInfo, VmMetricsSystem, VmReturnedState, VmUpdateBalloon, VmUpdateBalloonStatistics},
        VmError, VmShutdownMethod, VmState,
    },
};
use http::{Request, StatusCode};
use http_body_util::Full;
use test_framework::{get_tmp_path, shutdown_test_vm, NewVmBuilder};

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

// #[test]
// fn vm_api_can_create_restoreable_snapshot() {
//     NewVmBuilder::new().run(|mut model_vm| async move {
//         model_vm.api_pause().await.unwrap();
//         let snapshot_paths = model_vm
//             .api_create_snapshot(VmCreateSnapshot::new(get_tmp_path(), get_tmp_path()))
//             .await
//             .unwrap();

//         with_snapshot_restored_vm(snapshot_paths, |mut restored_vm| async move {
//             restored_vm.api_get_info().await.unwrap();
//             shutdown_test_vm(&mut restored_vm, VmShutdownMethod::CtrlAltDel).await;
//         })
//         .await;

//         shutdown_test_vm(&mut model_vm, VmShutdownMethod::CtrlAltDel).await;
//     });
// }
