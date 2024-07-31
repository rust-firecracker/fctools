use std::{path::PathBuf, time::Duration};

use fctools::{
    executor::{
        arguments::{FirecrackerApiSocket, FirecrackerArguments, JailerArguments},
        installation::FirecrackerInstallation,
        FlatJailRenamer, JailMoveMethod, JailedVmmExecutor,
    },
    shell::SuShellSpawner,
    vm::{
        configuration::{NewVmConfiguration, NewVmConfigurationApplier, VmConfiguration},
        models::{
            VmBalloon, VmBootSource, VmDrive, VmIoEngine, VmLogger, VmMachineConfiguration,
            VmMetrics, VmVsock,
        },
        Vm, VmShutdownMethod,
    },
};
use tokio::fs;

#[tokio::test]
async fn t() {
    let configuration = VmConfiguration::New(
        NewVmConfiguration::new(
            VmBootSource::new("/opt/testdata/vmlinux-6.1")
                .boot_args("console=ttyS0 reboot=k panic=1 pci=off"),
            VmMachineConfiguration::new(1, 512),
        )
        .drive(
            VmDrive::new("rootfs", true)
                .path_on_host("/opt/testdata/ubuntu-22.04.ext4")
                .is_read_only(true)
                .io_engine(VmIoEngine::Sync),
        )
        .balloon(VmBalloon::new(128, false).stats_polling_interval_s(1))
        .logger(
            VmLogger::new()
                .log_path("/opt/path.txt")
                .show_log_origin(true),
        )
        .metrics(VmMetrics::new("/opt/metrics.fifo"))
        .vsock(VmVsock::new(5, "/opt/uds.sock"))
        .applier(NewVmConfigurationApplier::ViaApiCalls),
    );

    let mut vm = Vm::prepare(
        JailedVmmExecutor {
            firecracker_arguments: FirecrackerArguments::new(FirecrackerApiSocket::Enabled(
                PathBuf::from("/tmp/fc.sock"),
            )),
            jailer_arguments: JailerArguments::new(1000, 1000, 1),
            jail_move_method: JailMoveMethod::Copy,
            jail_renamer: FlatJailRenamer::default(),
        },
        SuShellSpawner {
            su_path: PathBuf::from("/usr/bin/su"),
            password: "495762".into(),
        },
        FirecrackerInstallation {
            firecracker_path: PathBuf::from("/opt/testdata/firecracker"),
            jailer_path: PathBuf::from("/opt/testdata/jailer"),
            snapshot_editor_path: PathBuf::from("/opt/testdata/snapshot-editor"),
        },
        configuration,
    )
    .await
    .unwrap();
    vm.start(Duration::from_secs(1)).await.unwrap();
    tokio::time::sleep(Duration::from_secs(1)).await;
    dbg!(vm.api_get_firecracker_version().await.unwrap());
    dbg!(vm.get_standard_paths());
    println!(
        "{}",
        fs::read_to_string(vm.get_standard_paths().get_log_path().unwrap().as_path())
            .await
            .unwrap()
    );

    dbg!(vm.api_get_balloon_statistics().await.unwrap());

    vm.shutdown(vec![VmShutdownMethod::CtrlAltDel], Duration::from_secs(1))
        .await
        .unwrap();

    vm.cleanup().await.unwrap();
    dbg!(vm.state());
}
