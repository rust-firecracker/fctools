use std::{
    collections::HashMap,
    future::Future,
    io::Write,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    pin::Pin,
    sync::Arc,
    time::Duration,
};

use cidr::IpInet;
use fcnet_types::{FirecrackerIpStack, FirecrackerNetwork, FirecrackerNetworkOperation, FirecrackerNetworkType};
use fcnetd_client::FcnetdConnection;
use fctools::{
    fs_backend::{blocking::BlockingFsBackend, FsBackend},
    process_spawner::{DirectProcessSpawner, ProcessSpawner, SudoProcessSpawner},
    vm::{
        configuration::{InitMethod, VmConfiguration, VmConfigurationData},
        models::{
            BalloonDevice, BootSource, Drive, LoggerSystem, MachineConfiguration, MetricsSystem, MmdsConfiguration,
            MmdsVersion, NetworkInterface, VsockDevice,
        },
        ShutdownMethod,
    },
    vmm::{
        arguments::{
            command_modifier::NetnsCommandModifier,
            jailer::{JailerArguments, JailerCgroupVersion},
            VmmApiSocket, VmmArguments, VmmConfigurationOverride,
        },
        executor::{
            jailed::{FlatJailRenamer, JailedVmmExecutor},
            unrestricted::UnrestrictedVmmExecutor,
            VmmExecutor, VmmExecutorError,
        },
        installation::VmmInstallation,
        ownership::VmmOwnershipModel,
        process::VmmProcessState,
    },
};
use nix::unistd::{getegid, geteuid, Gid, Uid};
use rand::{Rng, RngCore};
use serde::Deserialize;
use tokio::{
    process::Child,
    sync::{Mutex, MutexGuard, OnceCell},
};
use uuid::Uuid;

static TEST_TOOLCHAIN: OnceCell<TestOptions> = OnceCell::const_new();

#[allow(unused)]
#[derive(Deserialize)]
pub struct TestOptions {
    pub toolchain: TestOptionsToolchain,
    pub waits: TestOptionsWaits,
    pub network_interface: String,
    pub jailer_uid: u32,
    pub jailer_gid: u32,
}

#[allow(unused)]
#[derive(Deserialize)]
pub struct TestOptionsToolchain {
    pub version: String,
    pub snapshot_version: String,
}

#[allow(unused)]
#[derive(Deserialize)]
pub struct TestOptionsWaits {
    pub shutdown_timeout_ms: u64,
    pub boot_wait_ms: u64,
    pub boot_socket_timeout_ms: u64,
}

impl TestOptions {
    #[allow(unused)]
    pub async fn get() -> &'static Self {
        TEST_TOOLCHAIN
            .get_or_init(|| async {
                let content = tokio::fs::read_to_string(get_test_path("options.json"))
                    .await
                    .expect("Could not read options.json");
                serde_json::from_str(&content).expect("options.json is malformed")
            })
            .await
    }

    pub fn get_blocking() -> Self {
        let content = std::fs::read_to_string(get_test_path("options.json")).expect("Could not read options.json");
        serde_json::from_str(&content).expect("options.json is malformed")
    }
}

// MISC UTILITIES

#[allow(unused)]
pub fn get_fake_firecracker_installation() -> VmmInstallation {
    get_real_firecracker_installation()
}

pub fn get_test_path(path: &str) -> PathBuf {
    let testdata_path = match std::env::var("FCTOOLS_TESTDATA_PATH") {
        Ok(path) => PathBuf::from(path),
        Err(_) => PathBuf::from("/opt/testdata"),
    };
    testdata_path.join(path)
}

#[allow(unused)]
pub fn get_real_firecracker_installation() -> VmmInstallation {
    VmmInstallation {
        firecracker_path: get_test_path("toolchain/firecracker"),
        jailer_path: get_test_path("toolchain/jailer"),
        snapshot_editor_path: get_test_path("toolchain/snapshot-editor"),
    }
}

pub fn get_tmp_path() -> PathBuf {
    PathBuf::from(format!("/tmp/{}", Uuid::new_v4()))
}

#[allow(unused)]
pub fn jail_join(path1: impl AsRef<Path>, path2: impl Into<PathBuf>) -> PathBuf {
    path1
        .as_ref()
        .join(path2.into().to_string_lossy().trim_start_matches("/"))
}

#[allow(unused)]
pub fn get_process_spawner() -> Arc<impl ProcessSpawner> {
    Arc::new(DirectProcessSpawner)
}

#[allow(unused)]
pub fn get_fs_backend() -> Arc<impl FsBackend> {
    Arc::new(BlockingFsBackend)
}

#[derive(Default)]
pub struct FailingRunner;

impl ProcessSpawner for FailingRunner {
    async fn spawn(
        &self,
        _path: &Path,
        _arguments: Vec<String>,
        _pipes_to_null: bool,
    ) -> Result<Child, std::io::Error> {
        Err(std::io::Error::other("Purposeful test failure"))
    }
}

#[allow(unused)]
pub enum TestExecutor {
    Unrestricted(UnrestrictedVmmExecutor),
    Jailed(JailedVmmExecutor<FlatJailRenamer>),
}

impl VmmExecutor for TestExecutor {
    fn get_socket_path(&self, installation: &VmmInstallation) -> Option<PathBuf> {
        match self {
            TestExecutor::Unrestricted(e) => e.get_socket_path(installation),
            TestExecutor::Jailed(e) => e.get_socket_path(installation),
        }
    }

    fn inner_to_outer_path(&self, installation: &VmmInstallation, inner_path: &Path) -> PathBuf {
        match self {
            TestExecutor::Unrestricted(e) => e.inner_to_outer_path(installation, inner_path),
            TestExecutor::Jailed(e) => e.inner_to_outer_path(installation, inner_path),
        }
    }

    fn is_traceless(&self) -> bool {
        match self {
            TestExecutor::Unrestricted(e) => e.is_traceless(),
            TestExecutor::Jailed(e) => e.is_traceless(),
        }
    }

    async fn prepare(
        &self,
        installation: &VmmInstallation,
        process_spawner: Arc<impl ProcessSpawner>,
        fs_backend: Arc<impl FsBackend>,
        outer_paths: Vec<PathBuf>,
        ownership_model: VmmOwnershipModel,
    ) -> Result<HashMap<PathBuf, PathBuf>, VmmExecutorError> {
        match self {
            TestExecutor::Unrestricted(e) => {
                e.prepare(installation, process_spawner, fs_backend, outer_paths, ownership_model)
                    .await
            }
            TestExecutor::Jailed(e) => {
                e.prepare(installation, process_spawner, fs_backend, outer_paths, ownership_model)
                    .await
            }
        }
    }

    async fn invoke(
        &self,
        installation: &VmmInstallation,
        process_spawner: Arc<impl ProcessSpawner>,
        config_override: VmmConfigurationOverride,
    ) -> Result<Child, VmmExecutorError> {
        match self {
            TestExecutor::Unrestricted(e) => e.invoke(installation, process_spawner, config_override).await,
            TestExecutor::Jailed(e) => e.invoke(installation, process_spawner, config_override).await,
        }
    }

    async fn cleanup(
        &self,
        installation: &VmmInstallation,
        process_spawner: Arc<impl ProcessSpawner>,
        fs_backend: Arc<impl FsBackend>,
        ownership_model: VmmOwnershipModel,
    ) -> Result<(), VmmExecutorError> {
        match self {
            TestExecutor::Unrestricted(e) => {
                e.cleanup(installation, process_spawner, fs_backend, ownership_model)
                    .await
            }
            TestExecutor::Jailed(e) => {
                e.cleanup(installation, process_spawner, fs_backend, ownership_model)
                    .await
            }
        }
    }
}

// VMM TEST FRAMEWORK

#[allow(unused)]
pub type TestVmmProcess = fctools::vmm::process::VmmProcess<TestExecutor, SudoProcessSpawner, BlockingFsBackend>;

#[allow(unused)]
pub async fn run_vmm_process_test<F, Fut>(closure: F)
where
    F: Fn(TestVmmProcess) -> Fut,
    F: 'static,
    Fut: Future<Output = ()>,
{
    async fn init_process(process: &mut TestVmmProcess) {
        process.wait_for_exit().await.unwrap_err();
        process.send_ctrl_alt_del().await.unwrap_err();
        process.send_sigkill().unwrap_err();
        process.take_pipes().unwrap_err();
        process.cleanup().await.unwrap_err();

        assert_eq!(process.state(), VmmProcessState::AwaitingPrepare);
        process.prepare().await.unwrap();
        assert_eq!(process.state(), VmmProcessState::AwaitingStart);
        process.invoke(VmmConfigurationOverride::NoOverride).await.unwrap();
        assert_eq!(process.state(), VmmProcessState::Started);
    }

    let (mut unrestricted_process, mut jailed_process) = get_vmm_processes(TestOptions::get().await);

    init_process(&mut jailed_process).await;
    init_process(&mut unrestricted_process).await;
    tokio::time::sleep(Duration::from_millis(TestOptions::get().await.waits.boot_wait_ms)).await;
    closure(unrestricted_process).await;
    println!("Succeeded with unrestricted VM");
    closure(jailed_process).await;
    println!("Succeeded with jailed VM");
}

fn get_vmm_processes(test_options: &TestOptions) -> (TestVmmProcess, TestVmmProcess) {
    let socket_path = get_tmp_path();

    let unrestricted_firecracker_arguments = VmmArguments::new(VmmApiSocket::Enabled(socket_path.clone()))
        .config_path(get_test_path("configs/unrestricted.json"));
    let jailer_firecracker_arguments =
        VmmArguments::new(VmmApiSocket::Enabled(socket_path)).config_path("/jailed.json");

    let jailer_arguments = JailerArguments::new(
        Uid::from_raw(test_options.jailer_uid),
        Gid::from_raw(test_options.jailer_gid),
        rand::thread_rng().next_u32().to_string().try_into().unwrap(),
    )
    .cgroup_version(JailerCgroupVersion::V2);
    let unrestricted_executor = UnrestrictedVmmExecutor::new(unrestricted_firecracker_arguments);
    let jailed_executor = JailedVmmExecutor::new(
        jailer_firecracker_arguments,
        jailer_arguments,
        FlatJailRenamer::default(),
    );

    (
        TestVmmProcess::new(
            TestExecutor::Unrestricted(unrestricted_executor),
            VmmOwnershipModel::UpgradedTemporarily,
            SudoProcessSpawner::new(),
            BlockingFsBackend,
            get_real_firecracker_installation(),
            vec![],
        ),
        TestVmmProcess::new(
            TestExecutor::Jailed(jailed_executor),
            VmmOwnershipModel::UpgradedTemporarily,
            SudoProcessSpawner::new(),
            BlockingFsBackend,
            get_real_firecracker_installation(),
            vec![
                get_test_path("assets/kernel"),
                get_test_path("assets/rootfs.ext4"),
                get_test_path("configs/jailed.json"),
            ],
        ),
    )
}

// VM TEST FRAMEWORK

#[allow(unused)]
pub type TestVm = fctools::vm::Vm<TestExecutor, SudoProcessSpawner, BlockingFsBackend>;

type PreStartHook = Box<dyn FnOnce(&mut TestVm) -> Pin<Box<dyn Future<Output = ()> + Send + '_>>>;

struct NetworkData {
    network_interface: NetworkInterface,
    network: FirecrackerNetwork,
    netns_name: String,
    boot_arg_append: String,
}

#[allow(unused)]
pub struct VmBuilder {
    init_method: InitMethod,
    logger_system: Option<LoggerSystem>,
    metrics_system: Option<MetricsSystem>,
    vsock_device: Option<VsockDevice>,
    pre_start_hook: Option<(PreStartHook, PreStartHook)>,
    balloon_device: Option<BalloonDevice>,
    unrestricted_network_data: Option<NetworkData>,
    jailed_network_data: Option<NetworkData>,
    boot_arg_append: String,
    mmds: bool,
}

#[allow(unused)]
impl VmBuilder {
    pub fn new() -> Self {
        Self {
            init_method: InitMethod::ViaApiCalls,
            logger_system: None,
            metrics_system: None,
            vsock_device: None,
            pre_start_hook: None,
            balloon_device: None,
            unrestricted_network_data: None,
            jailed_network_data: None,
            boot_arg_append: String::new(),
            mmds: false,
        }
    }

    pub fn init_method(mut self, init_method: InitMethod) -> Self {
        self.init_method = init_method;
        self
    }

    pub fn logger_system(mut self, logger_system: LoggerSystem) -> Self {
        self.logger_system = Some(logger_system);
        self
    }

    pub fn metrics_system(mut self, metrics_system: MetricsSystem) -> Self {
        self.metrics_system = Some(metrics_system);
        self
    }

    pub fn vsock_device(mut self) -> Self {
        self.vsock_device = Some(VsockDevice::new(rand::thread_rng().next_u32(), get_tmp_path()));
        self
    }

    pub fn pre_start_hook(
        mut self,
        hook: impl Fn(&mut TestVm) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> + Clone + 'static,
    ) -> Self {
        self.pre_start_hook = Some((Box::new(hook.clone()), Box::new(hook)));
        self
    }

    pub fn balloon_device(mut self, balloon_device: BalloonDevice) -> Self {
        self.balloon_device = Some(balloon_device);
        self
    }

    pub fn networking(mut self) -> Self {
        self.unrestricted_network_data = Some(self.setup_network());
        self.jailed_network_data = Some(self.setup_network());
        self
    }

    pub fn mmds(mut self) -> Self {
        self.mmds = true;
        self
    }

    fn setup_network(&self) -> NetworkData {
        fn inet(net_num: u16, num: u16) -> IpInet {
            format!("169.254.{}.{}/29", (8 * net_num + num) / 256, (8 * net_num + num) % 256)
                .parse()
                .unwrap()
        }

        let net_num = rand::thread_rng().gen_range(1..=3000);
        let guest_ip = inet(net_num, 1);
        let tap_ip = inet(net_num, 2);
        let tap_name = format!("tap{net_num}");
        let netns_name = format!("fcnetns{net_num}");
        let iface_name = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(async { TestOptions::get().await.network_interface.clone() });

        let network = FirecrackerNetwork {
            iface_name,
            tap_name: tap_name.clone(),
            tap_ip,
            network_type: FirecrackerNetworkType::Namespaced {
                netns_name: netns_name.clone(),
                veth1_name: format!("veth{net_num}"),
                veth2_name: format!("vpeer{net_num}"),
                veth1_ip: inet(net_num, 3),
                veth2_ip: inet(net_num, 4),
                forwarded_guest_ip: None,
            },
            nft_path: None,
            ip_stack: FirecrackerIpStack::V4,
            guest_ip,
        };
        let mut boot_arg_append = String::from(" ");
        boot_arg_append.push_str(network.guest_ip_boot_arg("eth0").as_str());

        NetworkData {
            network_interface: NetworkInterface::new("eth0", tap_name),
            network,
            netns_name,
            boot_arg_append,
        }
    }

    pub fn run<F, Fut>(self, function: F)
    where
        F: Fn(TestVm) -> Fut + Send,
        F: Clone + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        self.run_with_is_jailed(move |vm, _| function(vm));
    }

    pub fn run_with_is_jailed<F, Fut>(self, function: F)
    where
        F: Fn(TestVm, bool) -> Fut + Send,
        F: Clone + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        fn get_boot_arg(network_data: Option<&NetworkData>) -> String {
            let mut arg = "console=ttyS0 reboot=k panic=1 pci=off".to_string();
            if let Some(network_data) = network_data {
                arg.push_str(&network_data.boot_arg_append);
            }
            arg
        }

        let socket_path = get_tmp_path();

        let mut unrestricted_data = VmConfigurationData::new(
            BootSource::new(get_test_path("assets/kernel"))
                .boot_args(get_boot_arg(self.unrestricted_network_data.as_ref())),
            MachineConfiguration::new(1, 128).track_dirty_pages(true),
        )
        .drive(
            Drive::new("rootfs", true)
                .path_on_host(get_test_path("assets/rootfs.ext4"))
                .is_read_only(true),
        );
        let mut unrestricted_executor =
            UnrestrictedVmmExecutor::new(VmmArguments::new(VmmApiSocket::Enabled(socket_path.clone())));
        if let Some(ref network) = self.unrestricted_network_data {
            unrestricted_executor =
                unrestricted_executor.command_modifier(NetnsCommandModifier::new(&network.netns_name));
        }

        let mut jailed_data = VmConfigurationData::new(
            BootSource::new(get_test_path("assets/kernel")).boot_args(get_boot_arg(self.jailed_network_data.as_ref())),
            MachineConfiguration::new(1, 128).track_dirty_pages(true),
        )
        .drive(
            Drive::new("rootfs", true)
                .path_on_host(get_test_path("assets/rootfs.ext4"))
                .is_read_only(true),
        );

        let test_options = TestOptions::get_blocking();
        let mut jailer_arguments = JailerArguments::new(
            Uid::from_raw(test_options.jailer_uid),
            Gid::from_raw(test_options.jailer_gid),
            rand::thread_rng().next_u32().to_string().try_into().unwrap(),
        )
        .cgroup_version(JailerCgroupVersion::V2);
        if let Some(ref network) = self.jailed_network_data {
            jailer_arguments =
                jailer_arguments.network_namespace_path(format!("/var/run/netns/{}", network.netns_name));
        }

        let jailed_executor = TestExecutor::Jailed(JailedVmmExecutor::new(
            VmmArguments::new(VmmApiSocket::Enabled(socket_path)),
            jailer_arguments,
            FlatJailRenamer::default(),
        ));

        // add components from builder to data
        if let Some(logger) = self.logger_system {
            unrestricted_data = unrestricted_data.logger_system(logger.clone());
            jailed_data = jailed_data.logger_system(logger);
        }

        if let Some(metrics_system) = self.metrics_system {
            unrestricted_data = unrestricted_data.metrics_system(metrics_system.clone());
            jailed_data = jailed_data.metrics_system(metrics_system);
        }

        if let Some(vsock) = self.vsock_device {
            unrestricted_data = unrestricted_data.vsock_device(vsock.clone());
            jailed_data = jailed_data.vsock_device(vsock);
        }

        if let Some(balloon) = self.balloon_device {
            unrestricted_data = unrestricted_data.balloon_device(balloon.clone());
            jailed_data = jailed_data.balloon_device(balloon);
        }

        if let Some(ref network) = self.unrestricted_network_data {
            unrestricted_data = unrestricted_data.network_interface(network.network_interface.clone());
        }

        if let Some(ref network) = self.jailed_network_data {
            jailed_data = jailed_data.network_interface(network.network_interface.clone());
        }

        if self.mmds {
            let mmds_config = MmdsConfiguration::new(MmdsVersion::V2, vec!["eth0".to_string()]);
            unrestricted_data = unrestricted_data.mmds_configuration(mmds_config.clone());
            jailed_data = jailed_data.mmds_configuration(mmds_config);
        }

        let (pre_start_hook1, pre_start_hook2) = self.pre_start_hook.unzip();

        // run workers on tokio runtime
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                tokio::join!(
                    Self::test_worker(
                        self.unrestricted_network_data,
                        VmConfiguration::New {
                            init_method: self.init_method.clone(),
                            data: unrestricted_data
                        },
                        TestExecutor::Unrestricted(unrestricted_executor),
                        pre_start_hook1,
                        function.clone(),
                    ),
                    Self::test_worker(
                        self.jailed_network_data,
                        VmConfiguration::New {
                            init_method: self.init_method,
                            data: jailed_data
                        },
                        jailed_executor,
                        pre_start_hook2,
                        function
                    ),
                );
            });
    }

    async fn test_worker<F, Fut>(
        network_data: Option<NetworkData>,
        configuration: VmConfiguration,
        executor: TestExecutor,
        pre_start_hook: Option<PreStartHook>,
        function: F,
    ) where
        F: Fn(TestVm, bool) -> Fut + Send,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let (mut fcnetd_conn, mut fcnetd_child) = start_fcnetd().await;

        if let Some(ref network_data) = network_data {
            let lock = get_network_lock().await;
            fcnetd_conn
                .run(&network_data.network, FirecrackerNetworkOperation::Add)
                .await
                .unwrap();
            drop(lock);
        }

        let is_jailed = match executor {
            TestExecutor::Jailed(_) => true,
            TestExecutor::Unrestricted(_) => false,
        };

        let mut vm: fctools::vm::Vm<TestExecutor, SudoProcessSpawner, BlockingFsBackend> = TestVm::prepare(
            executor,
            VmmOwnershipModel::UpgradedTemporarily,
            SudoProcessSpawner::new(),
            BlockingFsBackend,
            get_real_firecracker_installation(),
            configuration,
        )
        .await
        .unwrap();
        if let Some(pre_start_hook) = pre_start_hook {
            pre_start_hook(&mut vm).await;
        }
        vm.start(Duration::from_millis(
            TestOptions::get().await.waits.boot_socket_timeout_ms,
        ))
        .await
        .unwrap();
        tokio::time::sleep(Duration::from_millis(TestOptions::get().await.waits.boot_wait_ms)).await;
        function(vm, is_jailed).await;

        if let Some(network_data) = network_data {
            let lock = get_network_lock().await;
            fcnetd_conn
                .run(&network_data.network, FirecrackerNetworkOperation::Delete)
                .await
                .unwrap();
            drop(lock);
        }

        fcnetd_child.kill().await.unwrap();
    }
}

#[allow(unused)]
pub async fn shutdown_test_vm(vm: &mut TestVm, shutdown_method: ShutdownMethod) {
    vm.shutdown(
        vec![shutdown_method],
        Duration::from_millis(TestOptions::get().await.waits.shutdown_timeout_ms),
    )
    .await
    .unwrap();
    vm.cleanup().await.unwrap();
}

static NETWORK_LOCKING_MUTEX: Mutex<()> = Mutex::const_new(());

#[allow(unused)]
struct NetworkLock<'a> {
    mutex_guard: MutexGuard<'a, ()>,
    file_lock: file_lock::FileLock,
}

async fn start_fcnetd() -> (FcnetdConnection, Child) {
    let socket_path = get_tmp_path();

    let child = SudoProcessSpawner::new()
        .spawn(
            &get_test_path("toolchain/fcnetd"),
            vec![
                "--uid".to_string(),
                geteuid().to_string(),
                "--gid".to_string(),
                getegid().to_string(),
                socket_path.to_string_lossy().into_owned(),
            ],
            true,
        )
        .await
        .unwrap();

    loop {
        if let Ok(metadata) = tokio::fs::metadata(&socket_path).await {
            if metadata.uid() == geteuid().as_raw() && metadata.gid() == getegid().as_raw() {
                break;
            }
        }
    }

    let connection = FcnetdConnection::connect(&socket_path).await.unwrap();
    (connection, child)
}

async fn get_network_lock<'a>() -> NetworkLock<'a> {
    let mutex_guard = NETWORK_LOCKING_MUTEX.lock().await;
    let file_lock = tokio::task::spawn_blocking(|| {
        let file_options = file_lock::FileOptions::new().write(true).create(true);
        let mut lock = file_lock::FileLock::lock("/tmp/fctools_test_net_lock", true, file_options).unwrap();
        lock.file.write(b"lock_data").unwrap();
        lock
    })
    .await
    .unwrap();

    NetworkLock { mutex_guard, file_lock }
}
