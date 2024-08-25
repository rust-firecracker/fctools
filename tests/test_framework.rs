use std::{
    collections::HashMap,
    future::Future,
    path::{Path, PathBuf},
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
    process::{VmmProcess, VmmProcessState},
    shell_spawner::{SameUserShellSpawner, ShellSpawner, SuShellSpawner},
    vm::{
        configuration::{NewVmConfiguration, NewVmConfigurationApplier, VmConfiguration},
        models::{VmBootSource, VmDrive, VmMachineConfiguration},
        Vm,
    },
};
use rand::RngCore;
use tokio::{process::Child, task::JoinSet};
use uuid::Uuid;

#[allow(unused)]
pub fn get_mock_firecracker_installation() -> FirecrackerInstallation {
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
        true
    }

    async fn spawn(&self, _shell_command: String) -> Result<Child, std::io::Error> {
        Err(std::io::Error::other("deliberately generated error in test"))
    }
}

#[allow(unused)]
pub type TestVmmProcess = VmmProcess<TestExecutor, TestShellSpawner>;

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

/// VMM TESTING

#[allow(unused)]
pub async fn vmm_test<F, Fut>(closure: F)
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
    tokio::time::sleep(Duration::from_millis(1500)).await;
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
        VmmProcess::new(
            TestExecutor::Unrestricted(unrestricted_executor),
            TestShellSpawner::SameUser(same_user_shell_spawner),
            get_real_firecracker_installation(),
            vec![],
        ),
        VmmProcess::new(
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

/// VM TESTING

#[allow(unused)]
pub struct NewVmBuilder {
    vcpu_count: u8,
    mem_size_mib: usize,
    applier: NewVmConfigurationApplier,
}

#[allow(unused)]
impl NewVmBuilder {
    pub fn new() -> Self {
        Self {
            vcpu_count: 1,
            mem_size_mib: 128,
            applier: NewVmConfigurationApplier::ViaApiCalls,
        }
    }

    pub fn vcpu_count(mut self, vcpu_count: u8) -> Self {
        self.vcpu_count = vcpu_count;
        self
    }

    pub fn mem_size_mib(mut self, mem_size_mib: usize) -> Self {
        self.mem_size_mib = mem_size_mib;
        self
    }

    pub fn applier(mut self, applier: NewVmConfigurationApplier) -> Self {
        self.applier = applier;
        self
    }

    pub fn run<F, Fut>(self, function: F)
    where
        F: Fn(Vm<TestExecutor, TestShellSpawner>) -> Fut + Send,
        F: Clone + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let socket_path = get_tmp_path();

        let unrestricted_configuration = VmConfiguration::New(
            NewVmConfiguration::new(
                VmBootSource::new(get_test_path("vmlinux-6.1")).boot_args("console=ttyS0 reboot=k panic=1 pci=off"),
                VmMachineConfiguration::new(self.vcpu_count, self.mem_size_mib),
            )
            .drive(VmDrive::new("rootfs", true).path_on_host(get_test_path("debian.ext4")))
            .applier(self.applier.clone()),
        );
        let unrestricted_executor = TestExecutor::Unrestricted(UnrestrictedVmmExecutor::new(
            FirecrackerArguments::new(FirecrackerApiSocket::Enabled(socket_path.clone())),
        ));
        let unrestricted_shell_spawner =
            TestShellSpawner::SameUser(SameUserShellSpawner::new(which::which("bash").unwrap()));

        let jailed_configuration = VmConfiguration::New(
            NewVmConfiguration::new(
                VmBootSource::new(get_test_path("vmlinux-6.1")).boot_args("console=ttyS0 reboot=k panic=1 pci=off"),
                VmMachineConfiguration::new(self.vcpu_count, self.mem_size_mib),
            )
            .drive(VmDrive::new("rootfs", true).path_on_host(get_test_path("debian.ext4")))
            .applier(self.applier),
        );
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

        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .worker_threads(2)
            .build()
            .unwrap()
            .block_on(async {
                let mut join_set = JoinSet::new();
                join_set.spawn(test_worker(
                    unrestricted_configuration,
                    unrestricted_executor,
                    unrestricted_shell_spawner,
                    function.clone(),
                ));
                join_set.spawn(test_worker(
                    jailed_configuration,
                    jailed_executor,
                    jailed_shell_spawner,
                    function,
                ));

                while let Some(result) = join_set.join_next().await {
                    result.unwrap();
                }
            });
    }
}

async fn test_worker<F, Fut>(
    configuration: VmConfiguration,
    executor: TestExecutor,
    shell_spawner: TestShellSpawner,
    function: F,
) where
    F: Fn(Vm<TestExecutor, TestShellSpawner>) -> Fut + Send,
    Fut: Future<Output = ()> + Send + 'static,
{
    let mut vm = Vm::prepare(
        executor,
        shell_spawner,
        get_real_firecracker_installation(),
        configuration,
    )
    .await
    .unwrap();
    vm.start(Duration::from_secs(1)).await.unwrap();
    tokio::time::sleep(Duration::from_millis(750)).await;
    function(vm).await;
}
