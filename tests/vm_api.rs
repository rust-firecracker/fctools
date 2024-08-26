use assert_matches::assert_matches;
use fctools::vm::{
    api::VmApi,
    models::{VmBalloon, VmUpdateBalloon, VmUpdateBalloonStatistics},
    VmError, VmShutdownMethod,
};
use http::StatusCode;
use test_framework::{shutdown_test_vm, NewVmBuilder};

mod test_framework;

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
