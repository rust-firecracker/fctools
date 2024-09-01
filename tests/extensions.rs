use fctools::{
    ext::snapshot_editor::SnapshotEditorExt,
    vm::{
        api::VmApi,
        models::{VmCreateSnapshot, VmSnapshotType},
    },
};
use test_framework::{get_real_firecracker_installation, get_tmp_path, shutdown_test_vm, VmBuilder};

mod test_framework;

#[test]
fn snapshot_editor_can_rebase_memory() {
    VmBuilder::new().run(|mut vm| async move {
        vm.api_pause().await.unwrap();
        let base_snapshot = vm
            .api_create_snapshot(VmCreateSnapshot::new(get_tmp_path(), get_tmp_path()))
            .await
            .unwrap();
        vm.api_resume().await.unwrap();
        vm.api_pause().await.unwrap();
        let diff_snapshot = vm
            .api_create_snapshot(
                VmCreateSnapshot::new(get_tmp_path(), get_tmp_path()).snapshot_type(VmSnapshotType::Diff),
            )
            .await
            .unwrap();
        vm.api_resume().await.unwrap();

        get_real_firecracker_installation()
            .snapshot_editor()
            .rebase_memory(base_snapshot.mem_file_path(), diff_snapshot.mem_file_path())
            .await
            .unwrap();
        shutdown_test_vm(&mut vm, fctools::vm::VmShutdownMethod::CtrlAltDel).await;
    })
}

#[test]
fn snapshot_editor_can_get_snapshot_version() {
    VmBuilder::new().run(|mut vm| async move {
        vm.api_pause().await.unwrap();
        let snapshot = vm
            .api_create_snapshot(VmCreateSnapshot::new(get_tmp_path(), get_tmp_path()))
            .await
            .unwrap();
        vm.api_resume().await.unwrap();

        let version = get_real_firecracker_installation()
            .snapshot_editor()
            .get_snapshot_version(snapshot.snapshot_path())
            .await
            .unwrap();
        assert_eq!(version.trim(), "v2.0.0");
        shutdown_test_vm(&mut vm, fctools::vm::VmShutdownMethod::CtrlAltDel).await;
    });
}

#[test]
fn snapshot_editor_can_get_snapshot_vcpu_states() {
    VmBuilder::new().run(|mut vm| async move {
        vm.api_pause().await.unwrap();
        let snapshot = vm
            .api_create_snapshot(VmCreateSnapshot::new(get_tmp_path(), get_tmp_path()))
            .await
            .unwrap();
        vm.api_resume().await.unwrap();

        let data = get_real_firecracker_installation()
            .snapshot_editor()
            .get_snapshot_vcpu_states(snapshot.snapshot_path())
            .await
            .unwrap();
        let first_line = data.lines().next().unwrap();
        assert!(first_line.contains("vcpu 0:"));

        shutdown_test_vm(&mut vm, fctools::vm::VmShutdownMethod::CtrlAltDel).await;
    });
}

#[test]
fn snapshot_editor_can_get_snapshot_vm_state() {
    VmBuilder::new().run(|mut vm| async move {
        vm.api_pause().await.unwrap();
        let snapshot = vm
            .api_create_snapshot(VmCreateSnapshot::new(get_tmp_path(), get_tmp_path()))
            .await
            .unwrap();
        vm.api_resume().await.unwrap();

        let data = get_real_firecracker_installation()
            .snapshot_editor()
            .get_snapshot_vm_state(snapshot.snapshot_path())
            .await
            .unwrap();
        assert!(data.contains("kvm"));
        shutdown_test_vm(&mut vm, fctools::vm::VmShutdownMethod::CtrlAltDel).await;
    });
}
