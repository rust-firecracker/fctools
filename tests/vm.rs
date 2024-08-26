use fctools::vm::{
    configuration::NewVmConfigurationApplier,
    models::{VmLogger, VmMetricsSystem},
    VmShutdownMethod,
};
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
fn vm_can_be_shut_down_via_sigkill() {
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
fn vm_can_be_shut_down_via_reboot_into_tty() {
    NewVmBuilder::new().run(|mut vm| async move {
        let pipes = vm.take_pipes().unwrap();
        shutdown(&mut vm, VmShutdownMethod::WriteRebootToStdin(pipes.stdin)).await;
    });
}

#[test]
fn vm_can_handle_logger() {
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
fn vm_can_handle_metrics() {
    NewVmBuilder::new()
        .metrics_system(VmMetricsSystem::new(get_tmp_path()))
        .run(|mut vm| async move {
            let metrics_path = vm.standard_paths().get_metrics_path().unwrap().clone();
            assert!(metadata(&metrics_path).await.unwrap().is_file());
            shutdown(&mut vm, VmShutdownMethod::CtrlAltDel).await;
            assert!(!try_exists(metrics_path).await.unwrap());
        })
}

async fn shutdown(vm: &mut TestVm, shutdown_method: VmShutdownMethod) {
    vm.shutdown(vec![shutdown_method], env_get_shutdown_timeout())
        .await
        .unwrap();
    vm.cleanup().await.unwrap();
}
