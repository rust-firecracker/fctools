use std::{path::PathBuf, str::FromStr, time::Duration};

use cidr::IpInet;
use fctools::{
    executor::{
        arguments::{FirecrackerApiSocket, FirecrackerArguments, JailerArguments},
        installation::FirecrackerInstallation,
        FlatJailRenamer, JailMoveMethod, JailedVmmExecutor,
    },
    ext::fcnet::FcnetConfiguration,
    shell::{SuShellSpawner, SudoShellSpawner},
    vm::{
        configuration::{NewVmConfiguration, NewVmConfigurationApplier, VmConfiguration},
        models::{VmBalloon, VmBootSource, VmDrive, VmMachineConfiguration, VmMetrics},
        Vm, VmShutdownMethod,
    },
};
use rand::RngCore;

#[tokio::test]
async fn t() {
    let configuration = VmConfiguration::New(
        NewVmConfiguration::new(
            VmBootSource::new("/opt/testdata/vmlinux-6.1")
                .boot_args("console=ttyS0 reboot=k panic=1 pci=off"),
            VmMachineConfiguration::new(1, 512),
        )
        .drive(VmDrive::new("rootfs", true).path_on_host("/opt/testdata/ubuntu-22.04.ext4"))
        .balloon(VmBalloon::new(256, false))
        .metrics(VmMetrics::new("/metrics"))
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
}

#[tokio::test]
async fn fcnet_execution() {
    let configuration = FcnetConfiguration::simple().iface_name("wlp1s0");
    let fcnet_path = PathBuf::from("/home/kanpov/.cargo/bin/fcnet");
    let shell_spawner = SuShellSpawner {
        su_path: PathBuf::from("/usr/bin/su"),
        password: "495762".to_string(),
    };

    dbg!(configuration.get_guest_ip_boot_arg(&IpInet::from_str("172.16.0.2/24").unwrap(), "eth0"));
    dbg!(configuration.get_guest_routing_command());
    configuration
        .add(&fcnet_path, &shell_spawner)
        .await
        .unwrap();
    configuration
        .check(&fcnet_path, &shell_spawner)
        .await
        .unwrap();
    configuration
        .delete(&fcnet_path, &shell_spawner)
        .await
        .unwrap();
}
