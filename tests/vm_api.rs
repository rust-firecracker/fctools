use assert_matches::assert_matches;
use bytes::Bytes;
use fctools::{
    vm::{
        api::{VmApi, VmApiError},
        models::{UpdateBalloonDevice, UpdateBalloonStatistics},
        VmState,
    },
    vmm::{process::HyperResponseExt, resource::CreatedResourceType},
};
use http::{Request, StatusCode};
use http_body_util::Full;
use serde::{Deserialize, Serialize};
use test_framework::{shutdown_test_vm, VmBuilder};

mod test_framework;

#[test]
fn vm_api_can_catch_api_errors() {
    VmBuilder::new().balloon_device(None).run(|mut vm| async move {
        // trying to set up balloon stats after being disabled pre-boot is a bad request
        let error = vm
            .api_update_balloon_statistics(UpdateBalloonStatistics {
                stats_polling_interval_s: 1,
            })
            .await
            .unwrap_err();
        assert_matches!(
            error,
            VmApiError::ReceivedErrorResponse {
                status_code: StatusCode::BAD_REQUEST,
                fault_message: _
            }
        );
        shutdown_test_vm(&mut vm).await;
    });
}

#[test]
fn vm_api_can_send_custom_request() {
    VmBuilder::new().run(|mut vm| async move {
        let mut response = vm
            .api_custom_request(
                "/",
                Request::builder().method("GET").body(Full::new(Bytes::new())).unwrap(),
                None,
            )
            .await
            .unwrap();
        assert!(response.status().is_success());
        let content = response.read_body_to_string().await.unwrap();
        assert!(content.starts_with('{'));
        shutdown_test_vm(&mut vm).await;
    });
}

#[test]
fn vm_api_custom_requests_perform_pause_changes() {
    VmBuilder::new().run(|mut vm| async move {
        let request = Request::builder().method("GET").body(Full::new(Bytes::new())).unwrap();
        vm.api_custom_request("/", request.clone(), Some(true)).await.unwrap();
        assert_eq!(vm.state(), VmState::Paused);
        vm.api_custom_request("/", request.clone(), Some(false)).await.unwrap();
        assert_eq!(vm.state(), VmState::Running);
        shutdown_test_vm(&mut vm).await;
    });
}

#[test]
fn vm_api_can_receive_info() {
    VmBuilder::new().run(|mut vm| async move {
        let info = vm.api_get_info().await.unwrap();
        assert_eq!(info.app_name, "Firecracker");
        assert!(!info.is_paused);
        shutdown_test_vm(&mut vm).await;
    });
}

#[test]
fn vm_api_can_flush_metrics() {
    VmBuilder::new()
        .metrics_system(CreatedResourceType::Fifo)
        .run(|mut vm| async move {
            vm.api_flush_metrics().await.unwrap();
            shutdown_test_vm(&mut vm).await;
        });
}

#[test]
fn vm_api_can_get_balloon() {
    VmBuilder::new().balloon_device(None).run(|mut vm| async move {
        let balloon = vm.api_get_balloon_device().await.unwrap();
        assert_eq!(balloon.stats_polling_interval_s, Some(0));
        assert_eq!(balloon.amount_mib, 64);
        assert!(!balloon.deflate_on_oom);
        shutdown_test_vm(&mut vm).await;
    });
}

#[test]
fn vm_api_can_update_balloon() {
    VmBuilder::new().balloon_device(None).run(|mut vm| async move {
        vm.api_update_balloon_device(UpdateBalloonDevice { amount_mib: 50 })
            .await
            .unwrap();
        let balloon = vm.api_get_balloon_device().await.unwrap();
        assert_eq!(balloon.amount_mib, 50);
        shutdown_test_vm(&mut vm).await;
    });
}

#[test]
fn vm_api_can_get_balloon_statistics() {
    VmBuilder::new().balloon_device(Some(1)).run(|mut vm| async move {
        let statistics = vm.api_get_balloon_statistics().await.unwrap();
        assert_ne!(statistics.actual_mib, 0);
        shutdown_test_vm(&mut vm).await;
    });
}

#[test]
fn vm_api_can_update_balloon_statistics() {
    VmBuilder::new().balloon_device(Some(1)).run(|mut vm| async move {
        vm.api_update_balloon_statistics(UpdateBalloonStatistics {
            stats_polling_interval_s: 3,
        })
        .await
        .unwrap();
        let _ = vm.api_get_balloon_statistics().await.unwrap();
        shutdown_test_vm(&mut vm).await;
    });
}

#[test]
fn vm_api_can_get_machine_configuration() {
    VmBuilder::new().run(|mut vm| async move {
        let machine_configuration = vm.api_get_machine_configuration().await.unwrap();
        assert_eq!(machine_configuration.vcpu_count, 1);
        assert_eq!(machine_configuration.mem_size_mib, 128);
        shutdown_test_vm(&mut vm).await;
    });
}

#[test]
fn vm_api_can_get_firecracker_version() {
    VmBuilder::new().run(|mut vm| async move {
        let firecracker_version = vm.api_get_firecracker_version().await.unwrap();
        assert!(firecracker_version.contains("1"));
        shutdown_test_vm(&mut vm).await;
    });
}

#[test]
fn vm_api_can_pause_and_resume() {
    VmBuilder::new().run(|mut vm| async move {
        vm.api_pause().await.unwrap();
        assert_eq!(vm.state(), VmState::Paused);
        vm.api_resume().await.unwrap();
        assert_eq!(vm.state(), VmState::Running);
        shutdown_test_vm(&mut vm).await;
    });
}

#[test]
fn vm_api_can_put_and_get_mmds_untyped() {
    VmBuilder::new().simple_networking().mmds().run(|mut vm| async move {
        vm.api_create_mmds_untyped(&serde_json::to_value(MmdsData { number: 4 }).unwrap())
            .await
            .unwrap();
        let data = serde_json::from_value::<MmdsData>(vm.api_get_mmds_untyped().await.unwrap()).unwrap();
        assert_eq!(data.number, 4);
        shutdown_test_vm(&mut vm).await;
    });
}

#[test]
fn vm_api_can_patch_mmds_untyped() {
    VmBuilder::new().simple_networking().mmds().run(|mut vm| async move {
        vm.api_create_mmds_untyped(&serde_json::to_value(MmdsData { number: 4 }).unwrap())
            .await
            .unwrap();
        vm.api_update_mmds_untyped(&serde_json::to_value(MmdsData { number: 5 }).unwrap())
            .await
            .unwrap();
        let data = serde_json::from_value::<MmdsData>(vm.api_get_mmds_untyped().await.unwrap()).unwrap();
        assert_eq!(data.number, 5);
        shutdown_test_vm(&mut vm).await;
    });
}

#[test]
fn vm_api_can_put_and_get_mmds_typed() {
    VmBuilder::new().simple_networking().mmds().run(|mut vm| async move {
        vm.api_create_mmds(MmdsData { number: 4 }).await.unwrap();
        let data = vm.api_get_mmds::<MmdsData>().await.unwrap();
        assert_eq!(data.number, 4);
        shutdown_test_vm(&mut vm).await;
    });
}

#[test]
fn vm_api_can_patch_mmds_typed() {
    VmBuilder::new().simple_networking().mmds().run(|mut vm| async move {
        vm.api_create_mmds(MmdsData { number: 4 }).await.unwrap();
        vm.api_update_mmds(MmdsData { number: 5 }).await.unwrap();
        let data = vm.api_get_mmds::<MmdsData>().await.unwrap();
        assert_eq!(data.number, 5);
        shutdown_test_vm(&mut vm).await;
    });
}

#[derive(Serialize, Deserialize)]
struct MmdsData {
    number: i32,
}
