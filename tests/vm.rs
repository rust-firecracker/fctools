use std::{net::IpAddr, path::PathBuf, str::FromStr, time::Duration};

use cidr::IpInet;
use fctools::{
    executor::{
        arguments::{FirecrackerApiSocket, FirecrackerArguments, JailerArguments},
        installation::FirecrackerInstallation,
        FlatJailRenamer, JailMoveMethod, JailedVmmExecutor,
    },
    ext::fcnet::{FcnetConfiguration, FcnetNetnsOptions},
    shell::{SuShellSpawner, SudoShellSpawner},
    vm::{
        configuration::{NewVmConfiguration, NewVmConfigurationApplier, VmConfiguration},
        models::{VmBalloon, VmBootSource, VmDrive, VmMachineConfiguration, VmMetrics, VmNetworkInterface},
        Vm, VmShutdownMethod,
    },
};
use rand::RngCore;

#[tokio::test]
async fn t() {
    let fcnet = FcnetConfiguration::netns(
        FcnetNetnsOptions::new().forwarded_guest_ip(IpAddr::from_str("192.168.0.3").unwrap()),
    )
    .iface_name("wlp7s0");
    let fcnet_path = PathBuf::from("/home/kanpov/.cargo/bin/fcnet");
    let shell_spawner = SuShellSpawner::new("495762");

    let ip_boot_arg = dbg!(fcnet.get_guest_ip_boot_arg(&IpInet::from_str("172.16.0.2/24").unwrap(), "eth0"));
    fcnet.add(&fcnet_path, &shell_spawner).await.unwrap();

    let configuration = VmConfiguration::New(
        NewVmConfiguration::new(
            VmBootSource::new("/opt/testdata/vmlinux-6.10")
                .boot_args(format!("console=ttyS0 reboot=k panic=1 pci=off {ip_boot_arg}")),
            VmMachineConfiguration::new(1, 512),
        )
        .drive(VmDrive::new("rootfs", true).path_on_host("/opt/testdata/ubuntu-22.04.ext4"))
        .balloon(VmBalloon::new(256, false))
        .metrics(VmMetrics::new("/metrics"))
        .network_interface(VmNetworkInterface::new("eth0", "tap0").guest_mac("06:00:AC:10:00:02"))
        .applier(NewVmConfigurationApplier::ViaApiCalls),
    );

    let mut vm = Vm::prepare(
        JailedVmmExecutor {
            firecracker_arguments: FirecrackerArguments::new(FirecrackerApiSocket::Enabled(PathBuf::from(
                "/tmp/fc.sock",
            ))),
            jailer_arguments: JailerArguments::new(1000, 1000, rand::thread_rng().next_u32().to_string())
                .network_namespace_path("/var/run/netns/fcnet"),
            jail_move_method: JailMoveMethod::Copy,
            jail_renamer: FlatJailRenamer::default(),
        },
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

    dbg!(vm.api_get_info().await.unwrap());
    vm.shutdown(vec![VmShutdownMethod::CtrlAltDel], Duration::from_secs(1))
        .await
        .unwrap();
    vm.cleanup().await.unwrap();

    fcnet.delete(&fcnet_path, &shell_spawner).await.unwrap();
}
