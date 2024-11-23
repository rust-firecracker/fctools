use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use futures_util::TryFutureExt;

use crate::{
    process_spawner::ProcessSpawner,
    runtime::{Runtime, RuntimeFilesystem, RuntimeJoinSet, RuntimeProcess},
    vmm::{
        arguments::{command_modifier::CommandModifier, jailer::JailerArguments, VmmApiSocket, VmmArguments},
        installation::VmmInstallation,
        ownership::{downgrade_owner_recursively, upgrade_owner, PROCESS_GID, PROCESS_UID},
        resource::{CreatedVmmResource, MovedVmmResource},
    },
};

use super::{process_handle::ProcessHandle, VmmExecutor, VmmExecutorError, VmmOwnershipModel};

/// A [VmmExecutor] that uses the "jailer" binary for maximum security and isolation, dropping privileges to then
/// run "firecracker". This executor, due to jailer design, can only run as root, even though the "firecracker"
/// process itself won't unless configured to run as UID 0 and GID 0 (root).
#[derive(Debug)]
pub struct JailedVmmExecutor<J: JailRenamer + 'static> {
    vmm_arguments: VmmArguments,
    jailer_arguments: JailerArguments,
    jail_renamer: J,
    command_modifier_chain: Vec<Box<dyn CommandModifier>>,
}

impl<J: JailRenamer + 'static> JailedVmmExecutor<J> {
    pub fn new(vmm_arguments: VmmArguments, jailer_arguments: JailerArguments, jail_renamer: J) -> Self {
        Self {
            vmm_arguments,
            jailer_arguments,
            jail_renamer,
            command_modifier_chain: Vec::new(),
        }
    }

    pub fn command_modifier(mut self, command_modifier: impl CommandModifier + 'static) -> Self {
        self.command_modifier_chain.push(Box::new(command_modifier));
        self
    }

    pub fn command_modifiers(mut self, command_modifiers: impl IntoIterator<Item = Box<dyn CommandModifier>>) -> Self {
        self.command_modifier_chain.extend(command_modifiers);
        self
    }
}

/// The method of moving resources into the jail
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum JailMoveMethod {
    /// Copy the file/directory
    Copy,
    /// Hard-link the file/directory (symlinking is incompatible with chroot, thus is not supported)
    HardLink,
    /// First try to hard link the file/directory, then resort to copying as a fallback and
    /// ignore the error that occurred when hard-linking
    HardLinkWithCopyFallback,
}

impl<J: JailRenamer + 'static> VmmExecutor for JailedVmmExecutor<J> {
    fn get_socket_path(&self, installation: &VmmInstallation) -> Option<PathBuf> {
        match &self.vmm_arguments.api_socket {
            VmmApiSocket::Disabled => None,
            VmmApiSocket::Enabled(socket_path) => Some(self.get_paths(installation).1.jail_join(socket_path)),
        }
    }

    async fn prepare<R: Runtime>(
        &mut self,
        installation: &VmmInstallation,
        process_spawner: Arc<impl ProcessSpawner>,
        ownership_model: VmmOwnershipModel,
        moved_resources: Vec<&mut MovedVmmResource>,
        mut created_resources: Vec<&mut CreatedVmmResource>,
    ) -> Result<(), VmmExecutorError> {
        // Create jail and delete previous one if necessary
        let (chroot_base_dir, jail_path) = self.get_paths(installation);
        upgrade_owner::<R>(&chroot_base_dir, ownership_model, process_spawner.as_ref())
            .await
            .map_err(VmmExecutorError::ChangeOwnerError)?;

        if R::Filesystem::check_exists(&jail_path)
            .await
            .map_err(VmmExecutorError::FilesystemError)?
        {
            R::Filesystem::remove_dir_all(&jail_path)
                .await
                .map_err(VmmExecutorError::FilesystemError)?;
        }

        R::Filesystem::create_dir_all(&jail_path)
            .await
            .map_err(VmmExecutorError::FilesystemError)?;

        let mut join_set: RuntimeJoinSet<_, R::Executor> = RuntimeJoinSet::new();

        // Ensure socket parent directory exists so that the firecracker process can bind inside of it
        if let VmmApiSocket::Enabled(ref socket_path) = self.vmm_arguments.api_socket {
            if let Some(socket_parent_dir) = socket_path.parent() {
                let socket_parent_dir = socket_parent_dir.to_owned();
                let jail_path = jail_path.clone();

                join_set.spawn(async move {
                    let expanded_path = jail_path.jail_join(&socket_parent_dir);

                    R::Filesystem::create_dir_all(&expanded_path)
                        .await
                        .map_err(VmmExecutorError::FilesystemError)
                });
            }
        }

        // Apply created resources
        if let Some(ref mut logs) = self.vmm_arguments.logs {
            created_resources.push(logs);
        }

        if let Some(ref mut metrics) = self.vmm_arguments.metrics {
            created_resources.push(metrics);
        }

        for created_resource in created_resources {
            join_set.spawn(
                created_resource
                    .apply::<R>(jail_path.jail_join(created_resource.local_path()), ownership_model)
                    .map_err(VmmExecutorError::ResourceError),
            );
        }

        // Apply moved resources
        for moved_resource in moved_resources {
            let local_path = self
                .jail_renamer
                .rename_for_jail(moved_resource.source_path())
                .map_err(VmmExecutorError::JailRenamerFailed)?;
            let effective_path = jail_path.jail_join(&local_path);
            join_set.spawn(
                moved_resource
                    .apply::<R>(effective_path, local_path, ownership_model, process_spawner.clone())
                    .map_err(VmmExecutorError::ResourceError),
            );
        }

        join_set.wait().await.unwrap_or(Err(VmmExecutorError::TaskJoinFailed))?;

        downgrade_owner_recursively::<R>(&jail_path, ownership_model)
            .await
            .map_err(VmmExecutorError::ChangeOwnerError)?;

        Ok(())
    }

    async fn invoke<R: Runtime>(
        &mut self,
        installation: &VmmInstallation,
        process_spawner: Arc<impl ProcessSpawner>,
        config_path: Option<PathBuf>,
        ownership_model: VmmOwnershipModel,
    ) -> Result<ProcessHandle<R::Process>, VmmExecutorError> {
        let (uid, gid) = match ownership_model.as_downgrade() {
            Some(values) => values,
            None => (*PROCESS_UID, *PROCESS_GID),
        };

        let mut arguments = self.jailer_arguments.join(uid, gid, &installation.firecracker_path);
        let mut binary_path = installation.jailer_path.clone();
        arguments.push("--".to_string());
        arguments.extend(self.vmm_arguments.join(config_path));

        for command_modifier in &self.command_modifier_chain {
            command_modifier.apply(&mut binary_path, &mut arguments);
        }

        // nulling the pipes is redundant since jailer can do this itself via daemonization
        let mut process = process_spawner
            .spawn::<R>(&binary_path, arguments, false)
            .await
            .map_err(VmmExecutorError::ProcessSpawnFailed)?;

        if self.jailer_arguments.daemonize || self.jailer_arguments.exec_in_new_pid_ns {
            let (_, jail_path) = self.get_paths(installation);
            let pid_file_path = jail_path.join(format!(
                "{}.pid",
                installation
                    .firecracker_path
                    .file_name()
                    .and_then(|f| f.to_str())
                    .unwrap_or("firecracker")
            ));

            let exit_status = process.wait().await.map_err(VmmExecutorError::ProcessWaitError)?;
            if !exit_status.success() {
                return Err(VmmExecutorError::ProcessExitedWithIncorrectStatus(exit_status));
            }

            upgrade_owner::<R>(&pid_file_path, ownership_model, process_spawner.as_ref())
                .await
                .map_err(VmmExecutorError::ChangeOwnerError)?;

            let pid;

            loop {
                if let Ok(pid_string) = R::Filesystem::read_to_string(&pid_file_path).await {
                    if let Ok(read_pid) = pid_string.trim_end().parse() {
                        pid = read_pid;
                        break;
                    }
                }
            }

            Ok(ProcessHandle::with_pidfd::<R>(pid).map_err(VmmExecutorError::PidfdAllocationError)?)
        } else {
            Ok(ProcessHandle::with_child(process, false))
        }
    }

    async fn cleanup<R: Runtime>(
        &mut self,
        installation: &VmmInstallation,
        process_spawner: Arc<impl ProcessSpawner>,
        ownership_model: VmmOwnershipModel,
        _created_resources: Vec<&mut CreatedVmmResource>,
    ) -> Result<(), VmmExecutorError> {
        let (_, jail_path) = self.get_paths(installation);

        upgrade_owner::<R>(&jail_path, ownership_model, process_spawner.as_ref())
            .await
            .map_err(VmmExecutorError::ChangeOwnerError)?;

        let Some(jail_parent_path) = jail_path.parent() else {
            return Err(VmmExecutorError::ExpectedDirectoryParentMissing(jail_path));
        };

        R::Filesystem::remove_dir_all(jail_parent_path)
            .await
            .map_err(VmmExecutorError::FilesystemError)
    }
}

impl<R: JailRenamer + 'static> JailedVmmExecutor<R> {
    fn get_paths(&self, installation: &VmmInstallation) -> (PathBuf, PathBuf) {
        let chroot_base_dir = self
            .jailer_arguments
            .chroot_base_dir
            .clone()
            .unwrap_or(PathBuf::from("/srv/jailer"));

        // example: /srv/jailer/firecracker/1/root
        let jail_path = chroot_base_dir
            .join(
                installation
                    .firecracker_path
                    .file_name()
                    .and_then(|f| f.to_str())
                    .unwrap_or("firecracker"),
            )
            .join(self.jailer_arguments.jail_id.as_ref())
            .join("root");

        (chroot_base_dir, jail_path)
    }
}

/// An error that can be emitted by a [JailRenamer].
#[derive(Debug)]
pub enum JailRenamerError {
    PathHasNoFilename(PathBuf),
    Other(Box<dyn std::error::Error + Send>),
}

impl std::error::Error for JailRenamerError {}

impl std::fmt::Display for JailRenamerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            JailRenamerError::PathHasNoFilename(path) => {
                write!(f, "A supposed file has no filename: {}", path.display())
            }
            JailRenamerError::Other(err) => write!(f, "Another error occurred: {err}"),
        }
    }
}

/// A trait defining a method of conversion between an outer path and an inner path. This conversion
/// should always produce the same path (or error) for the same given outside-jail path.
pub trait JailRenamer: Send + Sync + Clone {
    /// Rename the outer path to an inner path.
    fn rename_for_jail(&self, outer_path: &Path) -> Result<PathBuf, JailRenamerError>;
}

/// A resolver that transforms a host path with filename (including extension) "p" into /p
/// inside the jail. Given that files have unique names, this should be enough for most scenarios.
#[derive(Debug, Clone, Default)]
pub struct FlatJailRenamer;

impl JailRenamer for FlatJailRenamer {
    fn rename_for_jail(&self, outside_path: &Path) -> Result<PathBuf, JailRenamerError> {
        Ok(PathBuf::from(
            "/".to_owned()
                + &outside_path
                    .file_name()
                    .ok_or_else(|| JailRenamerError::PathHasNoFilename(outside_path.to_owned()))?
                    .to_string_lossy(),
        ))
    }
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

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::vmm::executor::jailed::JailJoin;

    use super::{FlatJailRenamer, JailRenamer};

    #[test]
    fn jail_join_performs_correctly() {
        assert_eq!(
            PathBuf::from("/jail").jail_join(&PathBuf::from("/inner")),
            PathBuf::from("/jail/inner")
        );
    }

    #[test]
    fn flat_jail_renamer_moves_correctly() {
        let renamer = FlatJailRenamer::default();
        assert_renamer(&renamer, "/opt/file", "/file");
        assert_renamer(&renamer, "/tmp/some_path.txt", "/some_path.txt");
        assert_renamer(&renamer, "/some/complex/outside/path/filename.ext4", "/filename.ext4");
    }

    fn assert_renamer(renamer: &impl JailRenamer, path: &str, expectation: &str) {
        assert_eq!(
            renamer.rename_for_jail(&PathBuf::from(path)).unwrap().to_str().unwrap(),
            expectation
        );
    }
}
