use std::{
    collections::HashMap,
    io,
    path::{Path, PathBuf},
    process::ExitStatus,
};

use arguments::{
    FirecrackerApiSocket, FirecrackerArguments, FirecrackerConfigOverride, JailerArguments,
};
use async_trait::async_trait;
use installation::FirecrackerInstallation;
use tokio::{
    fs,
    process::Child,
    task::{JoinError, JoinSet},
};

use crate::shell_spawner::ShellSpawner;

pub mod arguments;
pub mod installation;

#[derive(Debug)]
pub enum FirecrackerExecutorError {
    FilesystemError(io::Error),
    ShellWaitFailed(io::Error),
    ChownExitedWithWrongStatus(ExitStatus),
    MkdirExitedWithWrongStatus(ExitStatus),
    TaskJoinFailed(JoinError),
    ShellSpawnFailed(io::Error),
    ExpectedResourceMissing(PathBuf),
    ExpectedDirectoryParentMissing,
    ToInnerPathFailed(ToInnerPathError),
    Other(Box<dyn std::error::Error + Send>),
}

/// A trait that manages the execution of a Firecracker VMM process by setting up the environment, correctly invoking
/// the process and cleaning up the environment. This allows modularity between different modes of VMM execution.
#[async_trait]
pub trait VmmExecutor {
    /// Get the host location of the VMM socket, if one exists.
    fn get_outer_socket_path(&self) -> Option<PathBuf>;

    /// Resolves an inner path into an outer path.
    fn inner_to_outer_path(&self, inner_path: &Path) -> PathBuf;

    /// Prepare all transient resources for the VM invocation.
    async fn prepare(
        &self,
        shell_spawner: &impl ShellSpawner,
        outer_paths: Vec<PathBuf>,
    ) -> Result<HashMap<PathBuf, PathBuf>, FirecrackerExecutorError>;

    /// Invoke the VM on the given FirecrackerInstallation and return the spawned tokio Child.
    async fn invoke(
        &self,
        shell_spawner: &impl ShellSpawner,
        installation: &FirecrackerInstallation,
        config_override: FirecrackerConfigOverride,
    ) -> Result<Child, FirecrackerExecutorError>;

    /// Clean up all transient resources of the VM invocation.
    async fn cleanup(
        &self,
        shell_spawner: &impl ShellSpawner,
    ) -> Result<(), FirecrackerExecutorError>;
}

/// An executor that uses the "firecracker" binary directly, without jailing it or ensuring it doesn't run as root.
/// This executor allows rootless execution, given that the user has access to /dev/kvm.
#[derive(Debug)]
pub struct UnrestrictedVmmExecutor {
    /// Arguments passed to the "firecracker" binary
    pub firecracker_arguments: FirecrackerArguments,
}

#[async_trait]
impl VmmExecutor for UnrestrictedVmmExecutor {
    fn get_outer_socket_path(&self) -> Option<PathBuf> {
        match &self.firecracker_arguments.api_socket {
            FirecrackerApiSocket::Disabled => None,
            FirecrackerApiSocket::Enabled(path) => Some(path.clone()),
        }
    }

    fn inner_to_outer_path(&self, inner_path: &Path) -> PathBuf {
        inner_path.to_owned()
    }

    async fn prepare(
        &self,
        shell_spawner: &impl ShellSpawner,
        outer_paths: Vec<PathBuf>,
    ) -> Result<HashMap<PathBuf, PathBuf>, FirecrackerExecutorError> {
        for path in &outer_paths {
            if !fs::try_exists(path)
                .await
                .map_err(FirecrackerExecutorError::FilesystemError)?
            {
                return Err(FirecrackerExecutorError::ExpectedResourceMissing(
                    path.clone(),
                ));
            }
            force_chown(&path, shell_spawner).await?;
        }

        if let FirecrackerApiSocket::Enabled(socket_path) = &self.firecracker_arguments.api_socket {
            if fs::try_exists(socket_path)
                .await
                .map_err(FirecrackerExecutorError::FilesystemError)?
            {
                force_chown(socket_path, shell_spawner).await?;
                fs::remove_file(socket_path)
                    .await
                    .map_err(FirecrackerExecutorError::FilesystemError)?;
            }
        }

        Ok(outer_paths
            .into_iter()
            .map(|path| (path.clone(), path))
            .collect())
    }

    async fn invoke(
        &self,
        shell: &impl ShellSpawner,
        installation: &FirecrackerInstallation,
        config_override: FirecrackerConfigOverride,
    ) -> Result<Child, FirecrackerExecutorError> {
        let arguments = self.firecracker_arguments.join(config_override);
        let shell_command = format!(
            "{} {arguments}",
            installation.firecracker_path.to_string_lossy()
        );
        let child = shell
            .spawn(shell_command)
            .await
            .map_err(FirecrackerExecutorError::ShellSpawnFailed)?;
        Ok(child)
    }

    async fn cleanup(
        &self,
        shell_spawner: &impl ShellSpawner,
    ) -> Result<(), FirecrackerExecutorError> {
        if let FirecrackerApiSocket::Enabled(socket_path) = &self.firecracker_arguments.api_socket {
            if fs::try_exists(socket_path)
                .await
                .map_err(FirecrackerExecutorError::FilesystemError)?
            {
                force_chown(socket_path, shell_spawner).await?;
                fs::remove_file(socket_path)
                    .await
                    .map_err(FirecrackerExecutorError::FilesystemError)?;
            }
        }

        create_file_with_tree(&self.firecracker_arguments.log_path).await?;
        create_file_with_tree(&self.firecracker_arguments.metrics_path).await?;

        Ok(())
    }
}

/// An executor that uses the "jailer" binary for maximum security and isolation, dropping privileges to then
/// run "firecracker". This executor, due to jailer design, can only run as root, even though the "firecracker"
/// process itself won't.
#[derive(Debug)]
pub struct JailedVmmExecutor<T: ToInnerPath + 'static> {
    /// The arguments passed to the "firecracker" binary
    pub firecracker_arguments: FirecrackerArguments,
    /// The arguments passed to the "jailer" binary
    pub jailer_arguments: JailerArguments,
    /// The method of how to move VM resources into the jail
    pub jail_move_method: JailMoveMethod,
    /// The jail renamer that will be applied to VM resource paths during the move process
    pub jail_path_converter: T,
}

/// The method of moving resources into the jail
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum JailMoveMethod {
    Copy,
    HardLink,
    /// First try to hard link, then resort to copying as a fallback
    HardLinkWithCopyFallback,
}

#[async_trait]
impl<R: ToInnerPath + 'static> VmmExecutor for JailedVmmExecutor<R> {
    fn get_outer_socket_path(&self) -> Option<PathBuf> {
        match &self.firecracker_arguments.api_socket {
            FirecrackerApiSocket::Disabled => None,
            FirecrackerApiSocket::Enabled(socket_path) => {
                Some(self.get_jail_path().jail_join(&socket_path))
            }
        }
    }

    fn inner_to_outer_path(&self, inner_path: &Path) -> PathBuf {
        self.get_jail_path().jail_join(inner_path)
    }

    async fn prepare(
        &self,
        shell_spawner: &impl ShellSpawner,
        outer_paths: Vec<PathBuf>,
    ) -> Result<HashMap<PathBuf, PathBuf>, FirecrackerExecutorError> {
        // Ensure chroot base dir exists and is accessible
        let chroot_base_dir = match &self.jailer_arguments.chroot_base_dir {
            Some(dir) => &dir,
            None => &PathBuf::from("/srv/jailer"),
        };
        if !fs::try_exists(chroot_base_dir)
            .await
            .map_err(FirecrackerExecutorError::FilesystemError)?
        {
            force_mkdir(chroot_base_dir, shell_spawner).await?;
        }
        force_chown(chroot_base_dir, shell_spawner).await?; // grants access to jail as well

        // Create jail and delete previous one if necessary
        let jail_path = self.get_jail_path();
        if fs::try_exists(&jail_path)
            .await
            .map_err(FirecrackerExecutorError::FilesystemError)?
        {
            fs::remove_dir_all(&jail_path)
                .await
                .map_err(FirecrackerExecutorError::FilesystemError)?;
        }
        fs::create_dir_all(&jail_path)
            .await
            .map_err(FirecrackerExecutorError::FilesystemError)?;

        // Ensure socket parent directory exists so that the firecracker process can bind inside of it
        if let FirecrackerApiSocket::Enabled(socket_path) = &self.firecracker_arguments.api_socket {
            if let Some(socket_parent_dir) = socket_path.parent() {
                fs::create_dir_all(jail_path.jail_join(socket_parent_dir))
                    .await
                    .map_err(FirecrackerExecutorError::FilesystemError)?;
            }
        }

        // Ensure argument paths exist
        create_file_with_tree(
            &self
                .firecracker_arguments
                .log_path
                .as_ref()
                .map(|p| jail_path.join(p)),
        )
        .await?;
        create_file_with_tree(
            &self
                .firecracker_arguments
                .metrics_path
                .as_ref()
                .map(|p| jail_path.jail_join(p)),
        )
        .await?;

        // Apply jail renamer and move in the resources in parallel (via a join set)
        let mut join_set = JoinSet::new();
        let mut path_mappings = HashMap::with_capacity(outer_paths.len());

        for outer_path in outer_paths {
            if !fs::try_exists(&outer_path)
                .await
                .map_err(FirecrackerExecutorError::FilesystemError)?
            {
                return Err(FirecrackerExecutorError::ExpectedResourceMissing(
                    outer_path.clone(),
                ));
            }

            force_chown(&outer_path, shell_spawner).await?;

            let inner_path = self
                .jail_path_converter
                .to_inner_path(&outer_path)
                .map_err(FirecrackerExecutorError::ToInnerPathFailed)?;
            let expanded_inner_path = jail_path.jail_join(inner_path.as_ref());
            path_mappings.insert(outer_path.clone(), inner_path);

            // Inexpensively clone into the future
            let jail_move_method = self.jail_move_method;

            join_set.spawn(async move {
                if let Some(new_path_parent_dir) = expanded_inner_path.parent() {
                    fs::create_dir_all(new_path_parent_dir).await?;
                }
                match jail_move_method {
                    JailMoveMethod::Copy => {
                        fs::copy(outer_path, expanded_inner_path).await.map(|_| ())
                    }
                    JailMoveMethod::HardLink => {
                        fs::hard_link(outer_path, expanded_inner_path).await
                    }
                    JailMoveMethod::HardLinkWithCopyFallback => {
                        let hardlink_result =
                            fs::hard_link(&outer_path, &expanded_inner_path).await;
                        if let Err(_) = hardlink_result {
                            fs::copy(&outer_path, &expanded_inner_path)
                                .await
                                .map(|_| ())
                        } else {
                            hardlink_result
                        }
                    }
                }
            });
        }

        while let Some(result) = join_set.join_next().await {
            result
                .map_err(FirecrackerExecutorError::TaskJoinFailed)?
                .map_err(FirecrackerExecutorError::FilesystemError)?;
        }

        Ok(path_mappings)
    }

    async fn invoke(
        &self,
        shell: &impl ShellSpawner,
        installation: &FirecrackerInstallation,
        config_override: FirecrackerConfigOverride,
    ) -> Result<Child, FirecrackerExecutorError> {
        let jailer_args = self.jailer_arguments.join(&installation.firecracker_path);
        let firecracker_args = self.firecracker_arguments.join(config_override);
        let shell_command = format!(
            "{} {jailer_args} -- {firecracker_args}",
            installation.jailer_path.to_string_lossy()
        );
        shell
            .spawn(shell_command)
            .await
            .map_err(FirecrackerExecutorError::ShellSpawnFailed)
    }

    async fn cleanup(
        &self,
        _shell_spawner: &impl ShellSpawner,
    ) -> Result<(), FirecrackerExecutorError> {
        let jail_path = self.get_jail_path();
        let jail_parent_path = jail_path
            .parent()
            .ok_or(FirecrackerExecutorError::ExpectedDirectoryParentMissing)?;

        // Delete entire jail (../{id}/root) recursively
        fs::remove_dir_all(jail_parent_path)
            .await
            .map_err(FirecrackerExecutorError::FilesystemError)
    }
}

impl<R: ToInnerPath + 'static> JailedVmmExecutor<R> {
    fn get_jail_path(&self) -> PathBuf {
        let chroot_base_dir = match &self.jailer_arguments.chroot_base_dir {
            Some(dir) => dir.clone(),
            None => PathBuf::from("/srv/jailer"),
        };
        // example: /srv/jailer/firecracker/1/root
        chroot_base_dir
            .join("firecracker")
            .join(self.jailer_arguments.jail_id.to_string())
            .join("root")
    }
}

#[derive(Debug)]
pub enum ToInnerPathError {
    PathHasNoFilename,
    PathIsUnmapped(PathBuf),
    Other(Box<dyn std::error::Error + Send>),
}

/// A trait defining a method of conversion between an outer path and an inner path. This conversion
/// should always produce the same path (or error) for the same given outside-jail path.
pub trait ToInnerPath: Send + Sync + Clone {
    fn to_inner_path(&self, outer_path: &Path) -> Result<PathBuf, ToInnerPathError>;
}

/// A resolver that transforms a host path with filename (including extension) "p" into /p
/// inside the jail. Given that files have unique names, this should be enough for most scenarios.
#[derive(Debug, Clone, Default)]
pub struct FlatPathConverter {}

impl ToInnerPath for FlatPathConverter {
    fn to_inner_path(&self, outside_path: &Path) -> Result<PathBuf, ToInnerPathError> {
        Ok(PathBuf::from(
            "/".to_owned()
                + &outside_path
                    .file_name()
                    .ok_or(ToInnerPathError::PathHasNoFilename)?
                    .to_string_lossy(),
        ))
    }
}

/// A jail renamer that uses a lookup table from host to jail in order to transform paths.
#[derive(Debug, Clone)]
pub struct MappingPathConverter {
    mappings: HashMap<PathBuf, PathBuf>,
}

impl MappingPathConverter {
    pub fn new() -> Self {
        Self {
            mappings: HashMap::new(),
        }
    }

    pub fn map(
        &mut self,
        outside_path: impl Into<PathBuf>,
        jail_path: impl Into<PathBuf>,
    ) -> &mut Self {
        self.mappings.insert(outside_path.into(), jail_path.into());
        self
    }

    pub fn map_all(&mut self, mappings: impl IntoIterator<Item = (PathBuf, PathBuf)>) -> &mut Self {
        self.mappings.extend(mappings);
        self
    }
}

impl From<HashMap<PathBuf, PathBuf>> for MappingPathConverter {
    fn from(value: HashMap<PathBuf, PathBuf>) -> Self {
        Self { mappings: value }
    }
}

impl ToInnerPath for MappingPathConverter {
    fn to_inner_path(&self, outside_path: &Path) -> Result<PathBuf, ToInnerPathError> {
        let jail_path = self
            .mappings
            .get(outside_path)
            .ok_or_else(|| ToInnerPathError::PathIsUnmapped(outside_path.to_owned()))?;
        Ok(jail_path.clone())
    }
}

pub(crate) async fn force_chown(
    path: &Path,
    shell_spawner: &impl ShellSpawner,
) -> Result<(), FirecrackerExecutorError> {
    if shell_spawner.belongs_to_process() {
        return Ok(());
    }

    // SAFETY: calling FFI libc functions that return the process UID and GID can never result in UB
    let uid = unsafe { libc::geteuid() };
    let gid = unsafe { libc::getegid() };

    let mut child = shell_spawner
        .spawn(format!("chown -R {uid}:{gid} {}", path.to_string_lossy()))
        .await
        .map_err(FirecrackerExecutorError::ShellSpawnFailed)?;
    let exit_status = child
        .wait()
        .await
        .map_err(FirecrackerExecutorError::ShellWaitFailed)?;

    if !exit_status.success() {
        return Err(FirecrackerExecutorError::ChownExitedWithWrongStatus(
            exit_status,
        ));
    }

    Ok(())
}

async fn force_mkdir(
    path: &Path,
    shell_spawner: &impl ShellSpawner,
) -> Result<(), FirecrackerExecutorError> {
    if shell_spawner.belongs_to_process() {
        fs::create_dir_all(path)
            .await
            .map_err(FirecrackerExecutorError::FilesystemError)?;
        return Ok(());
    }

    let mut child = shell_spawner
        .spawn(format!("mkdir -p {}", path.to_string_lossy()))
        .await
        .map_err(FirecrackerExecutorError::ShellSpawnFailed)?;
    let exit_status = child
        .wait()
        .await
        .map_err(FirecrackerExecutorError::ShellWaitFailed)?;

    if !exit_status.success() {
        return Err(FirecrackerExecutorError::MkdirExitedWithWrongStatus(
            exit_status,
        ));
    }

    Ok(())
}

async fn create_file_with_tree(path: &Option<PathBuf>) -> Result<(), FirecrackerExecutorError> {
    if let Some(path) = path {
        if let Some(parent_path) = path.parent() {
            fs::create_dir_all(parent_path)
                .await
                .map_err(FirecrackerExecutorError::FilesystemError)?;
        }

        fs::File::create(path)
            .await
            .map_err(FirecrackerExecutorError::FilesystemError)?;
    }

    Ok(())
}

/// Custom extension to PathBuf that allows joining two absolute paths (outside jail and inside jail).
trait JailJoin {
    fn jail_join(&self, other_path: &Path) -> PathBuf;
}

impl JailJoin for PathBuf {
    fn jail_join(&self, other_path: &Path) -> PathBuf {
        let other_path = other_path.to_string_lossy();
        self.join(other_path.trim_start_matches("/"))
    }
}
