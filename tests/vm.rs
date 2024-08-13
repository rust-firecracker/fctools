use std::{path::PathBuf, str::FromStr, time::Duration};

use cidr::IpInet;
use fctools::{
    executor::{
        arguments::{FirecrackerApiSocket, FirecrackerArguments, JailerArguments},
        installation::FirecrackerInstallation,
        jailed::{FlatJailRenamer, JailedVmmExecutor},
    },
    ext::fcnet::FcnetConfiguration,
    shell_spawner::SudoShellSpawner,
    vm::{
        configuration::{NewVmConfiguration, NewVmConfigurationApplier, VmConfiguration},
        models::{VmBootSource, VmDrive, VmMachineConfiguration, VmNetworkInterface},
        Vm, VmShutdownMethod,
    },
};
use rand::RngCore;
use tokio::task::JoinSet;

static VM_COUNT: u32 = 10;

#[tokio::test]
async fn t() {
    let mut data = Vec::new();
    let fcnet_path = PathBuf::from("/home/kanpov/.cargo/bin/fcnet");
    let fcnet_spawner = SudoShellSpawner::with_password("495762");

    for i in 0..VM_COUNT {
        let tap_ip =
            IpInet::from_str(format!("169.254.{}.{}/30", (4 * i + 1) / 256, (4 * i + 1) % 256).as_str()).unwrap();
        let guest_ip =
            IpInet::from_str(format!("169.254.{}.{}/30", (4 * i + 2) / 256, (4 * i + 2) % 256).as_str()).unwrap();
        let tap_name = format!("tap{i}");

        let fcnet_config = FcnetConfiguration::simple()
            .iface_name("wlp7s0")
            .tap_name(tap_name.clone())
            .tap_ip(tap_ip);
        let ip_boot_arg = fcnet_config.get_guest_ip_boot_arg(&guest_ip, "eth0");
        fcnet_config.add(&fcnet_path, &fcnet_spawner).await.unwrap();

        let configuration = VmConfiguration::New(
            NewVmConfiguration::new(
                VmBootSource::new("/opt/testdata/vmlinux-5.10")
                    .boot_args(format!("console=ttyS0 reboot=k panic=1 pci=off {ip_boot_arg}")),
                VmMachineConfiguration::new(1, 512),
            )
            .drive(VmDrive::new("rootfs", true).path_on_host("/opt/testdata/debian.ext4"))
            .network_interface(VmNetworkInterface::new("eth0", tap_name))
            .applier(NewVmConfigurationApplier::ViaApiCalls),
        );

        let jailer_arguments = JailerArguments::new(1000, 1000, rand::thread_rng().next_u32().to_string());
        let firecracker_arguments =
            FirecrackerArguments::new(FirecrackerApiSocket::Enabled(PathBuf::from("/tmp/fc.sock")));
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
        vm.start(Duration::from_secs(2)).await.unwrap();

        data.push((vm, fcnet_config));
    }

    tokio::time::sleep(Duration::from_secs(2)).await;

    let mut join_set = JoinSet::new();
    for (mut vm, fcnet_config) in data {
        let fcnet_path = fcnet_path.clone();
        let fcnet_spawner = fcnet_spawner.clone();
        join_set.spawn(async move {
            vm.shutdown(vec![VmShutdownMethod::CtrlAltDel], Duration::from_secs(2))
                .await
                .unwrap();
            vm.cleanup().await.unwrap();
            fcnet_config.delete(fcnet_path, &fcnet_spawner).await.unwrap();
        });
    }

    while let Some(Ok(())) = join_set.join_next().await {}
}
