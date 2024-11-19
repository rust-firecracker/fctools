use std::{os::unix::fs::FileTypeExt, time::Duration};

use fctools::{
    extension::{metrics::spawn_metrics_task, snapshot_editor::SnapshotEditorExt},
    runtime::tokio::TokioRuntime,
    vm::{
        api::VmApi,
        models::{CreateSnapshot, MetricsSystem, SnapshotType},
    },
};
use futures_util::StreamExt;
use test_framework::{
    get_real_firecracker_installation, get_tmp_fifo_path, get_tmp_path, shutdown_test_vm, TestOptions, TestVm,
    VmBuilder,
};
use tokio::fs::metadata;

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
            .snapshot_editor::<TokioRuntime>()
            .rebase_memory(base_snapshot.mem_file_path, diff_snapshot.mem_file_path)
            .await
            .unwrap();
        shutdown_test_vm(&mut vm).await;
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
            .snapshot_editor::<TokioRuntime>()
            .get_snapshot_version(snapshot.snapshot_path)
            .await
            .unwrap();
        assert_eq!(version.trim(), TestOptions::get().await.toolchain.snapshot_version);
        shutdown_test_vm(&mut vm).await;
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
            .snapshot_editor::<TokioRuntime>()
            .get_snapshot_vcpu_states(snapshot.snapshot_path)
            .await
            .unwrap();
        let first_line = data.lines().next().unwrap();
        assert!(first_line.contains("vcpu 0:"));

        shutdown_test_vm(&mut vm).await;
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
            .snapshot_editor::<TokioRuntime>()
            .get_snapshot_vm_state(snapshot.snapshot_path)
            .await
            .unwrap();
        assert!(data.contains("kvm"));
        shutdown_test_vm(&mut vm).await;
    });
}

#[test]
fn metrics_task_can_receive_data_from_plaintext() {
    VmBuilder::new()
        .metrics_system(MetricsSystem::new(get_tmp_path()))
        .run(|vm| test_metrics_recv(false, vm));
}

#[test]
fn metrics_task_can_receive_data_from_fifo() {
    VmBuilder::new()
        .metrics_system(MetricsSystem::new(get_tmp_fifo_path()))
        .run(|vm| test_metrics_recv(true, vm));
}

async fn test_metrics_recv(is_fifo: bool, mut vm: TestVm) {
    let metrics_path = vm.get_accessible_paths().metrics_path.clone().unwrap();
    let file_type = metadata(&metrics_path).await.unwrap().file_type();

    if is_fifo {
        assert!(file_type.is_fifo());
    } else {
        assert!(!file_type.is_fifo());
        assert!(file_type.is_file());
    }

    let mut metrics_task =
        spawn_metrics_task::<TokioRuntime>(vm.get_accessible_paths().metrics_path.clone().unwrap(), 100);
    let metrics = metrics_task.receiver.next().await.unwrap();
    assert!(metrics.put_api_requests.actions_count > 0);
    shutdown_test_vm(&mut vm).await;
}

#[test]
fn metrics_task_can_be_cancelled_via_join_handle() {
    VmBuilder::new()
        .metrics_system(MetricsSystem::new(get_tmp_path()))
        .run(|mut vm| async move {
            let mut metrics_task =
                spawn_metrics_task::<TokioRuntime>(vm.get_accessible_paths().metrics_path.clone().unwrap(), 100);
            drop(metrics_task.task);
            tokio::time::timeout(Duration::from_secs(1), async {
                while metrics_task.receiver.next().await.is_some() {}
            })
            .await
            .unwrap();
            shutdown_test_vm(&mut vm).await;
        });
}
