use std::time::Duration;

use fctools::{
    ext::{metrics::spawn_metrics_task, snapshot_editor::SnapshotEditorExt},
    vm::{
        api::VmApi,
        models::{CreateSnapshot, MetricsSystem, SnapshotType},
        ShutdownMethod,
    },
};
use test_framework::{get_real_firecracker_installation, get_tmp_path, shutdown_test_vm, VmBuilder};

mod test_framework;

#[test]
fn snapshot_editor_can_rebase_memory() {
    VmBuilder::new().run(|mut vm| async move {
        vm.api_pause().await.unwrap();
        let base_snapshot = vm
            .api_create_snapshot(CreateSnapshot::new(get_tmp_path(), get_tmp_path()))
            .await
            .unwrap();
        vm.api_resume().await.unwrap();
        vm.api_pause().await.unwrap();
        let diff_snapshot = vm
            .api_create_snapshot(CreateSnapshot::new(get_tmp_path(), get_tmp_path()).snapshot_type(SnapshotType::Diff))
            .await
            .unwrap();
        vm.api_resume().await.unwrap();

        get_real_firecracker_installation()
            .snapshot_editor()
            .rebase_memory(base_snapshot.mem_file_path(), diff_snapshot.mem_file_path())
            .await
            .unwrap();
        shutdown_test_vm(&mut vm, ShutdownMethod::CtrlAltDel).await;
    })
}

#[test]
fn snapshot_editor_can_get_snapshot_version() {
    VmBuilder::new().run(|mut vm| async move {
        vm.api_pause().await.unwrap();
        let snapshot = vm
            .api_create_snapshot(CreateSnapshot::new(get_tmp_path(), get_tmp_path()))
            .await
            .unwrap();
        vm.api_resume().await.unwrap();

        let version = get_real_firecracker_installation()
            .snapshot_editor()
            .get_snapshot_version(snapshot.snapshot_path())
            .await
            .unwrap();
        assert_eq!(version.trim(), "v2.0.0");
        shutdown_test_vm(&mut vm, ShutdownMethod::CtrlAltDel).await;
    });
}

#[test]
fn snapshot_editor_can_get_snapshot_vcpu_states() {
    VmBuilder::new().run(|mut vm| async move {
        vm.api_pause().await.unwrap();
        let snapshot = vm
            .api_create_snapshot(CreateSnapshot::new(get_tmp_path(), get_tmp_path()))
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

        shutdown_test_vm(&mut vm, ShutdownMethod::CtrlAltDel).await;
    });
}

#[test]
fn snapshot_editor_can_get_snapshot_vm_state() {
    VmBuilder::new().run(|mut vm| async move {
        vm.api_pause().await.unwrap();
        let snapshot = vm
            .api_create_snapshot(CreateSnapshot::new(get_tmp_path(), get_tmp_path()))
            .await
            .unwrap();
        vm.api_resume().await.unwrap();

        let data = get_real_firecracker_installation()
            .snapshot_editor()
            .get_snapshot_vm_state(snapshot.snapshot_path())
            .await
            .unwrap();
        assert!(data.contains("kvm"));
        shutdown_test_vm(&mut vm, ShutdownMethod::CtrlAltDel).await;
    });
}

#[test]
fn metrics_task_can_receive_data() {
    VmBuilder::new()
        .metrics_system(MetricsSystem::new(get_tmp_path()))
        .run(|mut vm| async move {
            let mut metrics_task = spawn_metrics_task(vm.get_accessible_paths().metrics_path.clone().unwrap(), 100);
            let metrics = metrics_task.receiver.recv().await.unwrap();
            assert!(metrics.put_api_requests.actions_count > 0);
            shutdown_test_vm(&mut vm, ShutdownMethod::CtrlAltDel).await;
        });
}

#[test]
fn metrics_task_can_be_cancelled_via_join_handle() {
    VmBuilder::new()
        .metrics_system(MetricsSystem::new(get_tmp_path()))
        .run(|mut vm| async move {
            let mut metrics_task = spawn_metrics_task(vm.get_accessible_paths().metrics_path.clone().unwrap(), 100);
            metrics_task.join_handle.abort();
            assert!(
                tokio::time::timeout(Duration::from_secs(1), async { metrics_task.receiver.recv().await })
                    .await
                    .unwrap()
                    .is_none()
            );
            shutdown_test_vm(&mut vm, ShutdownMethod::CtrlAltDel).await;
        });
}
