use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use async_trait::async_trait;
use fctools::{
    executor::{
        arguments::FirecrackerConfigOverride,
        installation::FirecrackerInstallation,
        jailed::{FlatJailRenamer, JailedVmmExecutor},
        unrestricted::UnrestrictedVmmExecutor,
        VmmExecutor, VmmExecutorError,
    },
    process::VmmProcess,
    shell_spawner::{SameUserShellSpawner, ShellSpawner, SuShellSpawner},
};
use tokio::process::Child;
use uuid::Uuid;

#[allow(unused)]
pub fn get_mock_firecracker_installation() -> FirecrackerInstallation {
    FirecrackerInstallation {
        firecracker_path: get_tmp_path(),
        jailer_path: get_tmp_path(),
        snapshot_editor_path: get_tmp_path(),
    }
}

pub fn get_tmp_path() -> PathBuf {
    PathBuf::from(format!("/tmp/{}", Uuid::new_v4()))
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
pub struct EnvironmentPaths {
    pub kernel: PathBuf,
    pub rootfs: PathBuf,
    pub jail_config: PathBuf,
    pub config: PathBuf,
    pub firecracker: PathBuf,
    pub jailer: PathBuf,
    pub snapshot_editor: PathBuf,
}

#[allow(unused)]
pub type TestVmmProcess = VmmProcess<TestExecutor, TestShellSpawner>;

#[allow(unused)]
pub fn get_environment_paths() -> EnvironmentPaths {
    let path = |s: &str| format!("/opt/testdata/{s}").into();

    EnvironmentPaths {
        kernel: path("vmlinux-6.1"),
        rootfs: path("debian.ext4"),
        jail_config: path("jail-config.json"),
        config: path("config.json"),
        firecracker: path("firecracker"),
        jailer: path("jailer"),
        snapshot_editor: path("snapshot-editor"),
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
