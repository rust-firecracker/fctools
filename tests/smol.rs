use std::{sync::Arc, time::Duration};

use async_executor::Executor;
use async_io::{block_on, Timer};
use fctools::{
    process_spawner::DirectProcessSpawner,
    runtime::smol::SmolRuntime,
    vm::{
        configuration::{InitMethod, VmConfiguration, VmConfigurationData},
        models::{BootSource, Drive, MachineConfiguration},
        shutdown::{VmShutdownAction, VmShutdownMethod},
        Vm,
    },
    vmm::{
        arguments::{VmmApiSocket, VmmArguments},
        executor::unrestricted::UnrestrictedVmmExecutor,
        ownership::VmmOwnershipModel,
    },
};
use test_framework::{get_real_firecracker_installation, get_test_path, get_tmp_path};

mod test_framework;

#[test]
fn t() {
    let executor = Arc::new(Executor::new());
    SmolRuntime::initialize(executor.clone());

    let task = executor.spawn(async move {
        let socket_path = get_tmp_path();

        let configuration_data = VmConfigurationData::new(
            BootSource::new(get_test_path("assets/kernel"))
                .boot_args("console=ttyS0 reboot=k panic=1 pci=off root=/dev/vda"),
            MachineConfiguration::new(1, 128).track_dirty_pages(true),
        )
        .drive(
            Drive::new("rootfs", true)
                .path_on_host(get_test_path("assets/rootfs.ext4"))
                .is_read_only(true),
        );
        let executor = UnrestrictedVmmExecutor::new(VmmArguments::new(VmmApiSocket::Enabled(socket_path.clone())));

        let configuration = VmConfiguration::New {
            init_method: InitMethod::ViaApiCalls,
            data: configuration_data,
        };

        let mut vm = Vm::<_, _, SmolRuntime>::prepare(
            executor,
            VmmOwnershipModel::Shared,
            DirectProcessSpawner,
            get_real_firecracker_installation(),
            configuration,
        )
        .await
        .unwrap();
        vm.start(Duration::from_secs(5)).await.unwrap();
        Timer::after(Duration::from_secs(3)).await;

        let outcome = vm
            .shutdown(VmShutdownAction {
                method: VmShutdownMethod::CtrlAltDel,
                timeout: None,
                graceful: true,
            })
            .await
            .unwrap();
        assert!(outcome.graceful);
        vm.cleanup().await.unwrap();
    });

    block_on(executor.run(task));
}
