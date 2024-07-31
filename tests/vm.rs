use std::{path::PathBuf, time::Duration};

use fctools::{
    arguments::{FirecrackerApiSocket, FirecrackerArguments, JailerArguments},
    executor::{FlatPathConverter, JailMoveMethod, JailedFirecrackerExecutor},
    installation::FirecrackerInstallation,
    shell::SuShellSpawner,
};
use fctools::{
    configuration::{NewVmConfiguration, NewVmConfigurationApplier, VmConfiguration},
    models::{
        VmBalloon, VmBootSource, VmDrive, VmIoEngine, VmLogger, VmMachineConfiguration, VmMetrics,
        VmVsock,
    },
    vm::{Vm, VmShutdownMethod},
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
        .metrics(VmMetrics::new("/opt/metrics.txt"))
        .vsock(VmVsock::new(5, "/opt/uds.sock"))
        .applier(NewVmConfigurationApplier::ViaJsonConfiguration(
            PathBuf::from("/opt/conf.json"),
        )),
    );

    let mut vm = Vm::prepare(
        JailedFirecrackerExecutor {
            firecracker_arguments: FirecrackerArguments::new(FirecrackerApiSocket::Enabled(
                PathBuf::from("/tmp/fc.sock"),
            )),
            jailer_arguments: JailerArguments::new(1000, 1000, 1),
            jail_move_method: JailMoveMethod::Copy,
            jail_path_converter: FlatPathConverter::default(),
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

    vm.shutdown(vec![VmShutdownMethod::CtrlAltDel], Duration::from_secs(1))
        .await
        .unwrap();
    dbg!(vm.get_standard_paths());
    println!(
        "{}",
        fs::read_to_string(vm.get_standard_paths().get_log_path().unwrap().as_path())
            .await
            .unwrap()
    );
    vm.cleanup().await.unwrap();
    dbg!(vm.state());
}
