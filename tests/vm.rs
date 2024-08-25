use std::time::Duration;

use fctools::vm::{VmCleanupOptions, VmShutdownMethod};
use test_framework::NewVmBuilder;

mod test_framework;

#[test]
fn vm_can_boot() {
    NewVmBuilder::new().run(|mut vm| async move {
        vm.shutdown(vec![VmShutdownMethod::CtrlAltDel], Duration::from_secs(1))
            .await
            .unwrap();
        vm.cleanup(VmCleanupOptions::new()).await.unwrap();
    });
}
