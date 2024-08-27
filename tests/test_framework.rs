use std::{
    collections::HashMap,
    future::Future,
    io::Write,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use async_trait::async_trait;
use cidr::IpInet;
use fctools::{
    executor::{
        arguments::{FirecrackerApiSocket, FirecrackerArguments, FirecrackerConfigOverride, JailerArguments},
        command_modifier::NetnsCommandModifier,
        installation::FirecrackerInstallation,
        jailed::{FlatJailRenamer, JailedVmmExecutor},
        unrestricted::UnrestrictedVmmExecutor,
        VmmExecutor, VmmExecutorError,
    },
    ext::fcnet::{FcnetConfiguration, FcnetNetnsOptions},
    process::VmmProcessState,
    shell_spawner::{SameUserShellSpawner, ShellSpawner, SuShellSpawner},
    vm::{
        configuration::{NewVmBootMethod, VmConfiguration, VmConfigurationData},
        models::{
            VmBalloon, VmBootSource, VmDrive, VmLogger, VmMachineConfiguration, VmMetricsSystem, VmMmdsConfiguration,
            VmMmdsVersion, VmNetworkInterface, VmVsock,
        },
        VmShutdownMethod,
    },
};
use futures_util::future::BoxFuture;
use rand::{Rng, RngCore};
use tokio::{
    process::Child,
    sync::{Mutex, MutexGuard},
};
use uuid::Uuid;

// MISC UTILITIES

#[allow(unused)]
pub fn get_fake_firecracker_installation() -> FirecrackerInstallation {
    FirecrackerInstallation {
        firecracker_path: get_tmp_path(),
        jailer_path: get_tmp_path(),
        snapshot_editor_path: get_tmp_path(),
    }
}

pub fn get_test_path(path: &str) -> PathBuf {
    let testdata_path = match std::env::var("FCTOOLS_TESTDATA_PATH") {
        Ok(path) => PathBuf::from(path),
        Err(_) => PathBuf::from("/opt/testdata"),
    };
    testdata_path.join(path)
}

#[allow(unused)]
pub fn get_real_firecracker_installation() -> FirecrackerInstallation {
    FirecrackerInstallation {
        firecracker_path: get_test_path("firecracker"),
        jailer_path: get_test_path("jailer"),
        snapshot_editor_path: get_test_path("snapshot-editor"),
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
pub fn get_shell_spawner() -> impl ShellSpawner {
    SameUserShellSpawner::new(which::which("bash").unwrap())
}

#[derive(Default)]
pub struct FailingShellSpawner {}

#[async_trait]
impl ShellSpawner for FailingShellSpawner {
    fn increases_privileges(&self) -> bool {
        false
    }

    async fn spawn(&self, _shell_command: String) -> Result<Child, std::io::Error> {
        Err(std::io::Error::other("deliberately generated error in test"))
    }
}

#[allow(unused)]
pub enum TestExecutor {
    Unrestricted(UnrestrictedVmmExecutor),
    Jailed(JailedVmmExecutor<FlatJailRenamer>),
}

#[allow(unused)]
#[derive(Clone)]
pub enum TestShellSpawner {
    Su(SuShellSpawner),
    SameUser(SameUserShellSpawner),
}

#[async_trait]
impl ShellSpawner for TestShellSpawner {
    fn increases_privileges(&self) -> bool {
        match self {
            TestShellSpawner::Su(e) => e.increases_privileges(),
            TestShellSpawner::SameUser(e) => e.increases_privileges(),
        }
    }

    async fn spawn(&self, shell_command: String) -> Result<Child, tokio::io::Error> {
        match self {
            TestShellSpawner::Su(s) => s.spawn(shell_command).await,
            TestShellSpawner::SameUser(s) => s.spawn(shell_command).await,
        }
    }
}

#[async_trait]
impl VmmExecutor for TestExecutor {
    fn get_socket_path(&self) -> Option<PathBuf> {
        match self {
            TestExecutor::Unrestricted(e) => e.get_socket_path(),
            TestExecutor::Jailed(e) => e.get_socket_path(),
        }
    }

    fn inner_to_outer_path(&self, inner_path: &Path) -> PathBuf {
        match self {
            TestExecutor::Unrestricted(e) => e.inner_to_outer_path(inner_path),
            TestExecutor::Jailed(e) => e.inner_to_outer_path(inner_path),
        }
    }

    fn traceless(&self) -> bool {
        match self {
            TestExecutor::Unrestricted(e) => e.traceless(),
            TestExecutor::Jailed(e) => e.traceless(),
        }
    }

    async fn prepare(
        &self,
        shell_spawner: &impl ShellSpawner,
        outer_paths: Vec<PathBuf>,
    ) -> Result<HashMap<PathBuf, PathBuf>, VmmExecutorError> {
        match self {
            TestExecutor::Unrestricted(e) => e.prepare(shell_spawner, outer_paths).await,
            TestExecutor::Jailed(e) => e.prepare(shell_spawner, outer_paths).await,
        }
    }

    async fn invoke(
        &self,
        shell_spawner: &impl ShellSpawner,
        installation: &FirecrackerInstallation,
        config_override: FirecrackerConfigOverride,
    ) -> Result<Child, VmmExecutorError> {
        match self {
            TestExecutor::Unrestricted(e) => e.invoke(shell_spawner, installation, config_override).await,
            TestExecutor::Jailed(e) => e.invoke(shell_spawner, installation, config_override).await,
        }
    }

    async fn cleanup(&self, shell_spawner: &impl ShellSpawner) -> Result<(), VmmExecutorError> {
        match self {
            TestExecutor::Unrestricted(e) => e.cleanup(shell_spawner).await,
            TestExecutor::Jailed(e) => e.cleanup(shell_spawner).await,
        }
    }
}

#[allow(unused)]
pub fn env_get_shutdown_timeout() -> Duration {
    Duration::from_millis(match std::env::var("FCTOOLS_VM_SHUTDOWN_TIMEOUT") {
        Ok(value) => value
            .parse::<u64>()
            .expect("Shutdown timeout from env var is not a u64"),
        Err(_) => 7500,
    })
}

#[allow(unused)]
pub fn env_get_boot_wait() -> Duration {
    Duration::from_millis(match std::env::var("FCTOOLS_VM_BOOT_WAIT") {
        Ok(value) => value.parse::<u64>().expect("Boot wait from env var is not a u64"),
        Err(_) => 3000,
    })
}

#[allow(unused)]
pub fn env_get_boot_socket_wait() -> Duration {
    Duration::from_millis(match std::env::var("FCTOOLS_VM_BOOT_SOCKET_WAIT") {
        Ok(value) => value
            .parse::<u64>()
            .expect("Boot socket wait from env var is not a u64"),
        Err(_) => 7500,
    })
}

// VMM TEST FRAMEWORK

#[allow(unused)]
pub type TestVmmProcess = fctools::process::VmmProcess<TestExecutor, TestShellSpawner>;

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
        process.invoke(FirecrackerConfigOverride::NoOverride).await.unwrap();
        assert_eq!(process.state(), VmmProcessState::Started);
    }

    let (mut unrestricted_process, mut jailed_process) = get_vmm_processes();

    init_process(&mut jailed_process).await;
    init_process(&mut unrestricted_process).await;
    tokio::time::sleep(env_get_boot_wait() + Duration::from_secs(1)).await;
    closure(unrestricted_process).await;
    println!("Succeeded with unrestricted VM");
    closure(jailed_process).await;
    println!("Succeeded with jailed VM");
}

fn get_vmm_processes() -> (TestVmmProcess, TestVmmProcess) {
    let socket_path = get_tmp_path();

    let unrestricted_firecracker_arguments =
        FirecrackerArguments::new(FirecrackerApiSocket::Enabled(socket_path.clone()))
            .config_path(get_test_path("config.json"));
    let jailer_firecracker_arguments =
        FirecrackerArguments::new(FirecrackerApiSocket::Enabled(socket_path)).config_path("jail-config.json");

    let jailer_arguments = JailerArguments::new(
        unsafe { libc::geteuid() },
        unsafe { libc::getegid() },
        rand::thread_rng().next_u32().to_string(),
    );
    let unrestricted_executor = UnrestrictedVmmExecutor::new(unrestricted_firecracker_arguments);
    let jailed_executor = JailedVmmExecutor::new(
        jailer_firecracker_arguments,
        jailer_arguments,
        FlatJailRenamer::default(),
    );
    let su_shell_spawner = SuShellSpawner::new(std::env::var("ROOT_PWD").expect("No ROOT_PWD set"));
    let same_user_shell_spawner = SameUserShellSpawner::new(which::which("bash").unwrap());

    (
        TestVmmProcess::new(
            TestExecutor::Unrestricted(unrestricted_executor),
            TestShellSpawner::SameUser(same_user_shell_spawner),
            get_real_firecracker_installation(),
            vec![],
        ),
        TestVmmProcess::new(
            TestExecutor::Jailed(jailed_executor),
            TestShellSpawner::Su(su_shell_spawner),
            get_real_firecracker_installation(),
            vec![
                get_test_path("vmlinux-6.1"),
                get_test_path("debian.ext4"),
                get_test_path("jail-config.json"),
            ],
        ),
    )
}

// VM TEST FRAMEWORK

#[allow(unused)]
pub type TestVm = fctools::vm::Vm<TestExecutor, TestShellSpawner>;

type PreStartHook = Box<dyn FnOnce(&mut TestVm) -> BoxFuture<()>>;

struct NetworkData {
    network_interface: VmNetworkInterface,
    fcnet_configuration: FcnetConfiguration,
    netns_name: String,
    boot_arg_append: String,
}

#[allow(unused)]
pub struct VmBuilder {
    boot_method: NewVmBootMethod,
    logger: Option<VmLogger>,
    metrics_system: Option<VmMetricsSystem>,
    vsock: Option<VmVsock>,
    pre_start_hook: Option<(PreStartHook, PreStartHook)>,
    balloon: Option<VmBalloon>,
    unrestricted_network: Option<NetworkData>,
    jailed_network: Option<NetworkData>,
    boot_arg_append: String,
    mmds: bool,
}

#[allow(unused)]
pub struct SnapshottingContext {
    pub is_jailed: bool,
    pub shell_spawner: Arc<TestShellSpawner>,
}

impl SnapshottingContext {
    fn new(is_jailed: bool, shell_spawner: TestShellSpawner) -> Self {
        Self {
            is_jailed,
            shell_spawner: Arc::new(shell_spawner),
        }
    }
}

#[allow(unused)]
impl VmBuilder {
    pub fn new() -> Self {
        Self {
            boot_method: NewVmBootMethod::ViaApiCalls,
            logger: None,
            metrics_system: None,
            vsock: None,
            pre_start_hook: None,
            balloon: None,
            unrestricted_network: None,
            jailed_network: None,
            boot_arg_append: String::new(),
            mmds: false,
        }
    }

    pub fn boot_method(mut self, applier: NewVmBootMethod) -> Self {
        self.boot_method = applier;
        self
    }

    pub fn logger(mut self, logger: VmLogger) -> Self {
        self.logger = Some(logger);
        self
    }

    pub fn metrics_system(mut self, metrics_system: VmMetricsSystem) -> Self {
        self.metrics_system = Some(metrics_system);
        self
    }

    pub fn vsock(mut self, vsock: VmVsock) -> Self {
        self.vsock = Some(vsock);
        self
    }

    pub fn pre_start_hook(mut self, hook: impl Fn(&mut TestVm) -> BoxFuture<()> + Clone + 'static) -> Self {
        self.pre_start_hook = Some((Box::new(hook.clone()), Box::new(hook)));
        self
    }

    pub fn balloon(mut self, balloon: VmBalloon) -> Self {
        self.balloon = Some(balloon);
        self
    }

    pub fn networking(mut self) -> Self {
        self.unrestricted_network = Some(self.setup_network());
        self.jailed_network = Some(self.setup_network());
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

        let fcnet_configuration = FcnetConfiguration::netns(
            FcnetNetnsOptions::new()
                .veth1_ip(inet(net_num, 3))
                .veth1_name(format!("veth1{net_num}"))
                .veth2_ip(inet(net_num, 4))
                .netns_name(&netns_name)
                .guest_ip(guest_ip.address()),
        )
        .iface_name(std::env::var("FCTOOLS_NET_IFACE").expect("FCTOOLS_NET_IFACE not set"))
        .tap_name(&tap_name)
        .tap_ip(tap_ip);
        let mut boot_arg_append = String::from(" ");
        boot_arg_append.push_str(fcnet_configuration.get_guest_ip_boot_arg(&guest_ip, "eth0").as_str());

        NetworkData {
            network_interface: VmNetworkInterface::new("eth0", tap_name),
            fcnet_configuration,
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
        self.run_with_snapshotting_context(move |vm, _| function(vm));
    }

    pub fn run_with_snapshotting_context<F, Fut>(self, function: F)
    where
        F: Fn(TestVm, SnapshottingContext) -> Fut + Send,
        F: Clone + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        // setup executors, shell spawners and base data
        fn get_boot_arg(network: Option<&NetworkData>) -> String {
            let mut arg = "console=ttyS0 reboot=k panic=1 pci=off".to_string();
            if let Some(network) = network {
                arg.push_str(&network.boot_arg_append);
            }
            arg
        }

        let socket_path = get_tmp_path();

        let mut unrestricted_data = VmConfigurationData::new(
            VmBootSource::new(get_test_path("vmlinux-6.1")).boot_args(get_boot_arg(self.unrestricted_network.as_ref())),
            VmMachineConfiguration::new(1, 128),
        )
        .drive(VmDrive::new("rootfs", true).path_on_host(get_test_path("debian.ext4")));
        let mut unrestricted_executor = UnrestrictedVmmExecutor::new(FirecrackerArguments::new(
            FirecrackerApiSocket::Enabled(socket_path.clone()),
        ));
        if let Some(ref network) = self.unrestricted_network {
            unrestricted_executor =
                unrestricted_executor.command_modifier(NetnsCommandModifier::new(&network.netns_name));
        }

        let unrestricted_shell_spawner = match self.unrestricted_network {
            None => TestShellSpawner::SameUser(SameUserShellSpawner::new(which::which("bash").unwrap())),
            Some(_) => TestShellSpawner::Su(SuShellSpawner::new(std::env::var("ROOT_PWD").expect("No ROOT_PWD set"))),
        };

        let mut jailed_data = VmConfigurationData::new(
            VmBootSource::new(get_test_path("vmlinux-6.1")).boot_args(get_boot_arg(self.jailed_network.as_ref())),
            VmMachineConfiguration::new(1, 128),
        )
        .drive(VmDrive::new("rootfs", true).path_on_host(get_test_path("debian.ext4")));
        let mut jailer_arguments = JailerArguments::new(
            unsafe { libc::geteuid() },
            unsafe { libc::getegid() },
            rand::thread_rng().next_u32().to_string(),
        );
        if let Some(ref network) = self.jailed_network {
            jailer_arguments =
                jailer_arguments.network_namespace_path(format!("/var/run/netns/{}", network.netns_name));
        }

        let jailed_executor = TestExecutor::Jailed(JailedVmmExecutor::new(
            FirecrackerArguments::new(FirecrackerApiSocket::Enabled(socket_path)),
            jailer_arguments,
            FlatJailRenamer::default(),
        ));
        let jailed_shell_spawner =
            TestShellSpawner::Su(SuShellSpawner::new(std::env::var("ROOT_PWD").expect("No ROOT_PWD set")));

        // add components from builder to data
        if let Some(logger) = self.logger {
            unrestricted_data = unrestricted_data.logger(logger.clone());
            jailed_data = jailed_data.logger(logger);
        }

        if let Some(metrics_system) = self.metrics_system {
            unrestricted_data = unrestricted_data.metrics_system(metrics_system.clone());
            jailed_data = jailed_data.metrics_system(metrics_system);
        }

        if let Some(vsock) = self.vsock {
            unrestricted_data = unrestricted_data.vsock(vsock.clone());
            jailed_data = jailed_data.vsock(vsock);
        }

        if let Some(balloon) = self.balloon {
            unrestricted_data = unrestricted_data.balloon(balloon.clone());
            jailed_data = jailed_data.balloon(balloon);
        }

        if let Some(ref network) = self.unrestricted_network {
            unrestricted_data = unrestricted_data.network_interface(network.network_interface.clone());
        }

        if let Some(ref network) = self.jailed_network {
            jailed_data = jailed_data.network_interface(network.network_interface.clone());
        }

        if self.mmds {
            let mmds_config = VmMmdsConfiguration::new(VmMmdsVersion::V2, vec!["eth0".to_string()]);
            unrestricted_data = unrestricted_data.mmds_configuration(mmds_config.clone());
            jailed_data = jailed_data.mmds_configuration(mmds_config);
        }

        let (pre_start_hook1, pre_start_hook2) = match self.pre_start_hook {
            Some((a, b)) => (Some(a), Some(b)),
            None => (None, None),
        };

        // run workers on tokio runtime
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                tokio::join!(
                    Self::test_worker(
                        self.unrestricted_network,
                        VmConfiguration::New {
                            boot_method: self.boot_method.clone(),
                            data: unrestricted_data
                        },
                        SnapshottingContext::new(false, unrestricted_shell_spawner),
                        TestExecutor::Unrestricted(unrestricted_executor),
                        pre_start_hook1,
                        function.clone(),
                    ),
                    Self::test_worker(
                        self.jailed_network,
                        VmConfiguration::New {
                            boot_method: self.boot_method,
                            data: jailed_data
                        },
                        SnapshottingContext::new(true, jailed_shell_spawner.clone()),
                        jailed_executor,
                        pre_start_hook2,
                        function
                    ),
                );
            });
    }

    async fn test_worker<F, Fut>(
        network: Option<NetworkData>,
        configuration: VmConfiguration,
        run_context: SnapshottingContext,
        executor: TestExecutor,
        pre_start_hook: Option<PreStartHook>,
        function: F,
    ) where
        F: Fn(TestVm, SnapshottingContext) -> Fut + Send,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let fcnet_path = which::which("fcnet").expect("fcnet not installed onto PATH");

        let mut lock = None;
        if let Some(ref network) = network {
            lock = Some(get_network_lock().await);
            network
                .fcnet_configuration
                .add(&fcnet_path, run_context.shell_spawner.as_ref())
                .await
                .unwrap();
        }

        let mut vm: fctools::vm::Vm<TestExecutor, TestShellSpawner> = TestVm::prepare_arced(
            Arc::new(executor),
            run_context.shell_spawner.clone(),
            get_real_firecracker_installation().into(),
            configuration,
        )
        .await
        .unwrap();
        if let Some(pre_start_hook) = pre_start_hook {
            pre_start_hook(&mut vm).await;
        }
        vm.start(env_get_boot_socket_wait()).await.unwrap();
        tokio::time::sleep(env_get_boot_wait()).await;
        let cloned_shell_spawner = run_context.shell_spawner.clone();
        function(vm, run_context).await;

        if let Some(network) = network {
            network
                .fcnet_configuration
                .delete(&fcnet_path, cloned_shell_spawner.as_ref())
                .await
                .unwrap();
            let _ = network
                .fcnet_configuration
                .delete(fcnet_path, cloned_shell_spawner.as_ref())
                .await;
        }
    }
}

static NETWORK_LOCKING_MUTEX: Mutex<()> = Mutex::const_new(());

#[allow(unused)]
struct NetworkLock<'a> {
    mutex_guard: MutexGuard<'a, ()>,
    file_lock: file_lock::FileLock,
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

#[allow(unused)]
pub async fn shutdown_test_vm(vm: &mut TestVm, shutdown_method: VmShutdownMethod) {
    vm.shutdown(vec![shutdown_method], env_get_shutdown_timeout())
        .await
        .unwrap();
    vm.cleanup().await.unwrap();
}
