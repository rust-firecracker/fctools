use std::{path::PathBuf, str::FromStr, sync::Arc, time::Duration};

use cidr::IpInet;
use fctools::{
    executor::{
        arguments::{FirecrackerApiSocket, FirecrackerArguments, JailerArguments},
        installation::FirecrackerInstallation,
        FlatJailRenamer, JailMoveMethod, JailedVmmExecutor,
    },
    ext::nat::NatNetwork,
    shell::SudoShellSpawner,
    vm::{
        configuration::{NewVmConfiguration, NewVmConfigurationApplier, VmConfiguration},
        models::{VmBalloon, VmBootSource, VmDrive, VmMachineConfiguration, VmMetrics},
        Vm, VmShutdownMethod,
    },
};
use rand::RngCore;

#[tokio::test]
async fn t() {
    let nat_network = NatNetwork {
        tap_name: "tap0".to_string(),
        host_ip: IpInet::from_str("172.16.0.1/24").unwrap(),
        host_iface: "wlp1s0".to_string(),
        guest_ip: IpInet::from_str("172.16.0.2/24").unwrap(),
        shell_spawner: Arc::new(SudoShellSpawner {
            sudo_path: PathBuf::from("/usr/bin/sudo"),
            password: Some("495762".into()),
        }),
    };
    nat_network.create().await.unwrap();
    let iface = nat_network
        .get_vm_network_interface()
        .guest_mac("06:00:AC:10:00:02");
    let mut boot_args = "console=ttyS0 reboot=k panic=1 pci=off".to_string();
    nat_network.append_ip_boot_arg(&mut boot_args);
    dbg!(&boot_args);

    let configuration = VmConfiguration::New(
        NewVmConfiguration::new(
            VmBootSource::new("/opt/testdata/vmlinux-6.1").boot_args(boot_args),
            VmMachineConfiguration::new(1, 512),
        )
        .drive(VmDrive::new("rootfs", true).path_on_host("/opt/testdata/ubuntu-22.04.ext4"))
        .balloon(VmBalloon::new(256, false))
        .metrics(VmMetrics::new("/metrics"))
        .network_interface(iface)
        .applier(NewVmConfigurationApplier::ViaApiCalls),
    );

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

    tokio::time::sleep(Duration::from_secs(2)).await;

    dbg!(vm.api_get_info().await.unwrap());
    vm.shutdown(vec![VmShutdownMethod::CtrlAltDel], Duration::from_secs(1))
        .await
        .unwrap();
    vm.cleanup().await.unwrap();

    nat_network.delete().await.unwrap();
}
