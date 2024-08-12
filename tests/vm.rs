use std::{path::PathBuf, time::Duration};

use fctools::{
    executor::{
        arguments::{FirecrackerApiSocket, FirecrackerArguments, JailerArguments},
        installation::FirecrackerInstallation,
        jailed::{FlatJailRenamer, JailedVmmExecutor},
    },
    ext::serial_console::VmmSerialConsole,
    shell_spawner::SudoShellSpawner,
    vm::{
        configuration::{NewVmConfiguration, NewVmConfigurationApplier, VmConfiguration},
        models::{VmBootSource, VmDrive, VmMachineConfiguration},
        Vm, VmShutdownMethod,
    },
};
use rand::RngCore;

#[tokio::test]
async fn t() {
    let configuration = VmConfiguration::New(
        NewVmConfiguration::new(
            VmBootSource::new("/opt/testdata/vmlinux-5.10").boot_args("console=ttyS0 reboot=k panic=1 pci=off"),
            VmMachineConfiguration::new(1, 512),
        )
        .drive(VmDrive::new("rootfs", true).path_on_host("/opt/testdata/ubuntu-22.04.ext4"))
        .applier(NewVmConfigurationApplier::ViaApiCalls),
    );

    let jailer_arguments = JailerArguments::new(1000, 1000, rand::thread_rng().next_u32().to_string());
    let firecracker_arguments = FirecrackerArguments::new(FirecrackerApiSocket::Enabled(PathBuf::from("/tmp/fc.sock")));
    let mut vm = Vm::prepare(
        JailedVmmExecutor::new(firecracker_arguments, jailer_arguments, FlatJailRenamer::default()),
        SudoShellSpawner::with_password("495762"),
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

    tokio::time::sleep(Duration::from_secs(1)).await;
    let mut sercon = VmmSerialConsole::new(vm.take_pipes().unwrap());

    dbg!(vm.api_get_info().await.unwrap());
    vm.shutdown(vec![VmShutdownMethod::CtrlAltDel], Duration::from_secs(1))
        .await
        .unwrap();
    vm.cleanup().await.unwrap();
}
