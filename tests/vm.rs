use std::os::unix::fs::FileTypeExt;

use fctools::vm::{
    configuration::NewVmConfigurationApplier,
    models::{VmLogger, VmMetricsSystem, VmVsock},
    VmShutdownMethod,
};
use rand::RngCore;
use test_framework::{env_get_shutdown_timeout, get_tmp_path, NewVmBuilder, TestVm};
use tokio::fs::{metadata, try_exists};

mod test_framework;

#[test]
fn vm_can_boot_via_api_calls() {
    NewVmBuilder::new()
        .applier(NewVmConfigurationApplier::ViaApiCalls)
        .run(|mut vm| async move {
            shutdown(&mut vm, VmShutdownMethod::CtrlAltDel).await;
        });
}

#[test]
fn vm_can_boot_via_json() {
    NewVmBuilder::new()
        .applier(NewVmConfigurationApplier::ViaJsonConfiguration(get_tmp_path()))
        .run(|mut vm| async move {
            shutdown(&mut vm, VmShutdownMethod::CtrlAltDel).await;
        });
}

#[test]
fn vm_can_be_shut_down_via_kill() {
    NewVmBuilder::new().run(|mut vm| async move {
        shutdown(&mut vm, VmShutdownMethod::Kill).await;
    });
}

#[test]
fn vm_can_be_shut_down_via_pause_then_kill() {
    NewVmBuilder::new().run(|mut vm| async move {
        shutdown(&mut vm, VmShutdownMethod::PauseThenKill).await;
    });
}

#[test]
fn vm_processes_logger_path() {
    NewVmBuilder::new()
        .logger(VmLogger::new().log_path(get_tmp_path()))
        .run(|mut vm| async move {
            let log_path = vm.standard_paths().get_log_path().unwrap().clone();
            assert!(metadata(&log_path).await.unwrap().is_file());
            shutdown(&mut vm, VmShutdownMethod::CtrlAltDel).await;
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
            shutdown(&mut vm, VmShutdownMethod::CtrlAltDel).await;
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
            shutdown(&mut vm, VmShutdownMethod::CtrlAltDel).await;
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
        shutdown(&mut vm, VmShutdownMethod::CtrlAltDel).await;
    });
}

#[test]
fn vm_translates_inner_to_outer_paths() {
    NewVmBuilder::new().run(|mut vm| async move {
        let inner_path = get_tmp_path();
        let outer_path = vm.inner_to_outer_path(&inner_path);
        assert!(inner_path == outer_path || outer_path.to_str().unwrap().ends_with(inner_path.to_str().unwrap()));
        shutdown(&mut vm, VmShutdownMethod::CtrlAltDel).await;
    });
}

async fn shutdown(vm: &mut TestVm, shutdown_method: VmShutdownMethod) {
    vm.shutdown(vec![shutdown_method], env_get_shutdown_timeout())
        .await
        .unwrap();
    vm.cleanup().await.unwrap();
}
