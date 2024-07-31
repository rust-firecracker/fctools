use std::{path::PathBuf, time::Duration};

use fctools::{
    executor::{
        arguments::{FirecrackerApiSocket, FirecrackerArguments, JailerArguments},
        installation::FirecrackerInstallation,
        FlatJailRenamer, JailMoveMethod, JailedVmmExecutor,
    },
    shell::SudoShellSpawner,
    vm::{
        configuration::{NewVmConfiguration, NewVmConfigurationApplier, VmConfiguration},
        models::{VmBootSource, VmDrive, VmMachineConfiguration},
        Vm, VmShutdownMethod,
    },
};
use rand::RngCore;

static VM_COUNT: u8 = 1;

#[tokio::test]
async fn t() {
    let configuration = VmConfiguration::New(
        NewVmConfiguration::new(
            VmBootSource::new("/opt/testdata/vmlinux-6.1")
                .boot_args("console=ttyS0 reboot=k panic=1 pci=off"),
            VmMachineConfiguration::new(1, 512),
        )
        .drive(VmDrive::new("rootfs", true).path_on_host("/opt/testdata/ubuntu-22.04.ext4"))
        .applier(NewVmConfigurationApplier::ViaApiCalls),
    );

    let mut vms = Vec::new();
    for _ in 1..=VM_COUNT {
        let mut vm = Vm::prepare(
            JailedVmmExecutor {
                firecracker_arguments: FirecrackerArguments::new(FirecrackerApiSocket::Enabled(
                    PathBuf::from("/tmp/fc.sock"),
                )),
                jailer_arguments: JailerArguments::new(
                    1000,
                    1000,
                    rand::thread_rng().next_u32().to_string(),
                ),
                jail_move_method: JailMoveMethod::Copy,
                jail_renamer: FlatJailRenamer::default(),
            },
            SudoShellSpawner {
                sudo_path: PathBuf::from("/usr/bin/sudo"),
                password: Some("495762".into()),
            },
            FirecrackerInstallation {
                firecracker_path: PathBuf::from("/opt/testdata/firecracker"),
                jailer_path: PathBuf::from("/opt/testdata/jailer"),
                snapshot_editor_path: PathBuf::from("/opt/testdata/snapshot-editor"),
            },
            configuration.clone(),
        )
        .await
        .unwrap();
        vm.start(Duration::from_secs(1)).await.unwrap();
        vms.push(vm);
    }

    tokio::time::sleep(Duration::from_secs(2)).await;

    for mut vm in vms {
        dbg!(vm.api_get_info().await.unwrap());
        vm.shutdown(vec![VmShutdownMethod::CtrlAltDel], Duration::from_secs(1))
            .await
            .unwrap();
        vm.cleanup().await.unwrap();
    }
}
