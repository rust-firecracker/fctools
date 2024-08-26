use std::{
    collections::HashMap,
    future::Future,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use async_trait::async_trait;
use fctools::{
    executor::{
        arguments::{FirecrackerApiSocket, FirecrackerArguments, FirecrackerConfigOverride, JailerArguments},
        installation::FirecrackerInstallation,
        jailed::{FlatJailRenamer, JailedVmmExecutor},
        unrestricted::UnrestrictedVmmExecutor,
        VmmExecutor, VmmExecutorError,
    },
    process::VmmProcessState,
    shell_spawner::{SameUserShellSpawner, ShellSpawner, SuShellSpawner},
    vm::{
        configuration::{NewVmBootMethod, VmConfiguration, VmConfigurationData},
        models::{VmBalloon, VmBootSource, VmDrive, VmLogger, VmMachineConfiguration, VmMetricsSystem, VmVsock},
        VmShutdownMethod,
    },
};
use futures_util::future::BoxFuture;
use rand::RngCore;
use tokio::process::Child;
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
        Err(_) => 2500,
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

#[allow(unused)]
pub struct NewVmBuilder {
    boot_method: NewVmBootMethod,
    logger: Option<VmLogger>,
    metrics_system: Option<VmMetricsSystem>,
    vsock: Option<VmVsock>,
    pre_start_hook: Option<(PreStartHook, PreStartHook)>,
    balloon: Option<VmBalloon>,
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
impl NewVmBuilder {
    pub fn new() -> Self {
        Self {
            boot_method: NewVmBootMethod::ViaApiCalls,
            logger: None,
            metrics_system: None,
            vsock: None,
            pre_start_hook: None,
            balloon: None,
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
        let socket_path = get_tmp_path();

        let mut unrestricted_data = VmConfigurationData::new(
            VmBootSource::new(get_test_path("vmlinux-6.1")).boot_args("console=ttyS0 reboot=k panic=1 pci=off"),
            VmMachineConfiguration::new(1, 128),
        )
        .drive(VmDrive::new("rootfs", true).path_on_host(get_test_path("debian.ext4")));
        let unrestricted_executor = TestExecutor::Unrestricted(UnrestrictedVmmExecutor::new(
            FirecrackerArguments::new(FirecrackerApiSocket::Enabled(socket_path.clone())),
        ));
        let unrestricted_shell_spawner =
            TestShellSpawner::SameUser(SameUserShellSpawner::new(which::which("bash").unwrap()));

        let mut jailed_data = VmConfigurationData::new(
            VmBootSource::new(get_test_path("vmlinux-6.1")).boot_args("console=ttyS0 reboot=k panic=1 pci=off"),
            VmMachineConfiguration::new(1, 128),
        )
        .drive(VmDrive::new("rootfs", true).path_on_host(get_test_path("debian.ext4")));
        let jailed_executor = TestExecutor::Jailed(JailedVmmExecutor::new(
            FirecrackerArguments::new(FirecrackerApiSocket::Enabled(socket_path)),
            JailerArguments::new(
                unsafe { libc::geteuid() },
                unsafe { libc::getegid() },
                rand::thread_rng().next_u32().to_string(),
            ),
            FlatJailRenamer::default(),
        ));
        let jailed_shell_spawner =
            TestShellSpawner::Su(SuShellSpawner::new(std::env::var("ROOT_PWD").expect("No ROOT_PWD set")));

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

        let (pre_start_hook1, pre_start_hook2) = match self.pre_start_hook {
            Some((a, b)) => (Some(a), Some(b)),
            None => (None, None),
        };

        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                tokio::join!(
                    Self::test_worker(
                        VmConfiguration::New {
                            boot_method: self.boot_method.clone(),
                            data: unrestricted_data
                        },
                        SnapshottingContext::new(false, unrestricted_shell_spawner),
                        unrestricted_executor,
                        pre_start_hook1,
                        function.clone(),
                    ),
                    Self::test_worker(
                        VmConfiguration::New {
                            boot_method: self.boot_method,
                            data: jailed_data
                        },
                        SnapshottingContext::new(true, jailed_shell_spawner),
                        jailed_executor,
                        pre_start_hook2,
                        function
                    ),
                );
            });
    }

    async fn test_worker<F, Fut>(
        configuration: VmConfiguration,
        run_context: SnapshottingContext,
        executor: TestExecutor,
        pre_start_hook: Option<PreStartHook>,
        function: F,
    ) where
        F: Fn(TestVm, SnapshottingContext) -> Fut + Send,
        Fut: Future<Output = ()> + Send + 'static,
    {
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
        function(vm, run_context).await;
    }
}

#[allow(unused)]
pub async fn shutdown_test_vm(vm: &mut TestVm, shutdown_method: VmShutdownMethod) {
    vm.shutdown(vec![shutdown_method], env_get_shutdown_timeout())
        .await
        .unwrap();
    vm.cleanup().await.unwrap();
}
