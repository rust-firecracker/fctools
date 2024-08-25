use std::time::Duration;

use fctools::vm::{configuration::NewVmConfigurationApplier, VmCleanupOptions, VmShutdownMethod};
use test_framework::{get_tmp_path, NewVmBuilder, TestVm};

mod test_framework;

#[test]
fn vm_can_boot_via_api_calls() {
    vm_boot_test(NewVmConfigurationApplier::ViaApiCalls);
}

#[test]
fn vm_can_boot_via_json() {
    vm_boot_test(NewVmConfigurationApplier::ViaJsonConfiguration(get_tmp_path()));
}

fn vm_boot_test(applier: NewVmConfigurationApplier) {
    NewVmBuilder::new().applier(applier).run(|mut vm| async move {
        shutdown(&mut vm).await;
    });
}

async fn shutdown(vm: &mut TestVm) {
    vm.shutdown(vec![VmShutdownMethod::CtrlAltDel], Duration::from_secs(1))
        .await
        .unwrap();
    vm.cleanup(VmCleanupOptions::new()).await.unwrap();
}
