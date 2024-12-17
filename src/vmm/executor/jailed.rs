use std::path::{Path, PathBuf};

use futures_util::TryFutureExt;

use crate::{
    process_spawner::ProcessSpawner,
    runtime::{Runtime, RuntimeFilesystem, RuntimeJoinSet, RuntimeProcess},
    vmm::{
        arguments::{command_modifier::CommandModifier, jailer::JailerArguments, VmmApiSocket, VmmArguments},
        installation::VmmInstallation,
        ownership::{downgrade_owner_recursively, upgrade_owner, PROCESS_GID, PROCESS_UID},
        resource::VmmResourceReferences,
    },
};

use super::{process_handle::ProcessHandle, VmmExecutor, VmmExecutorContext, VmmExecutorError};

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

impl<J: JailRenamer + 'static> VmmExecutor for JailedVmmExecutor<J> {
    fn get_socket_path(&self, installation: &VmmInstallation) -> Option<PathBuf> {
        match &self.vmm_arguments.api_socket {
            VmmApiSocket::Disabled => None,
            VmmApiSocket::Enabled(socket_path) => Some(self.get_paths(installation).1.jail_join(socket_path)),
        }
    }

    fn local_to_effective_path(&self, installation: &VmmInstallation, local_path: PathBuf) -> PathBuf {
        self.get_paths(installation).1.jail_join(&local_path)
    }

    async fn prepare<S: ProcessSpawner, R: Runtime>(
        &mut self,
        context: VmmExecutorContext<S, R>,
        mut resource_references: VmmResourceReferences<'_>,
    ) -> Result<(), VmmExecutorError> {
        // Create jail and delete previous one if necessary
        let (chroot_base_dir, jail_path) = self.get_paths(context.installation.as_ref());
        upgrade_owner(
            &chroot_base_dir,
            context.ownership_model,
            &context.process_spawner,
            &context.runtime,
        )
        .await
        .map_err(VmmExecutorError::ChangeOwnerError)?;

        if context
            .runtime
            .filesystem()
            .check_exists(&jail_path)
            .await
            .map_err(VmmExecutorError::FilesystemError)?
        {
            context
                .runtime
                .filesystem()
                .remove_dir_all(&jail_path)
                .await
                .map_err(VmmExecutorError::FilesystemError)?;
        }

        context
            .runtime
            .filesystem()
            .create_dir_all(&jail_path)
            .await
            .map_err(VmmExecutorError::FilesystemError)?;

        let mut join_set = RuntimeJoinSet::new(context.runtime.clone());

        // Ensure socket parent directory exists so that the firecracker process can bind inside of it
        if let VmmApiSocket::Enabled(ref socket_path) = self.vmm_arguments.api_socket {
            if let Some(socket_parent_dir) = socket_path.parent() {
                let socket_parent_dir = socket_parent_dir.to_owned();
                let jail_path = jail_path.clone();
                let runtime = context.runtime.clone();

                join_set.spawn(async move {
                    let expanded_path = jail_path.jail_join(&socket_parent_dir);

                    runtime
                        .filesystem()
                        .create_dir_all(&expanded_path)
                        .await
                        .map_err(VmmExecutorError::FilesystemError)
                });
            }
        }

        // Apply created resources
        if let Some(ref mut logs) = self.vmm_arguments.logs {
            resource_references.created_resources.push(logs);
        }

        if let Some(ref mut metrics) = self.vmm_arguments.metrics {
            resource_references.created_resources.push(metrics);
        }

        for created_resource in resource_references.created_resources {
            join_set.spawn(
                created_resource
                    .initialize(
                        jail_path.jail_join(created_resource.local_path()),
                        context.ownership_model,
                        context.runtime.clone(),
                    )
                    .map_err(VmmExecutorError::ResourceError),
            );
        }

        // Apply moved resources
        for moved_resource in resource_references.moved_resources {
            let local_path = self
                .jail_renamer
                .rename_for_jail(moved_resource.source_path())
                .map_err(VmmExecutorError::JailRenamerFailed)?;
            let effective_path = jail_path.jail_join(&local_path);
            join_set.spawn(
                moved_resource
                    .initialize(
                        effective_path,
                        local_path,
                        context.ownership_model,
                        context.process_spawner.clone(),
                        context.runtime.clone(),
                    )
                    .map_err(VmmExecutorError::ResourceError),
            );
        }

        // Apply produced resources
        for produced_resource in resource_references.produced_resources {
            join_set.spawn(
                produced_resource
                    .initialize(
                        jail_path.jail_join(produced_resource.local_path()),
                        context.ownership_model,
                        context.runtime.clone(),
                    )
                    .map_err(VmmExecutorError::ResourceError),
            );
        }

        join_set.wait().await.unwrap_or(Err(VmmExecutorError::TaskJoinFailed))?;

        downgrade_owner_recursively(&jail_path, context.ownership_model, &context.runtime)
            .await
            .map_err(VmmExecutorError::ChangeOwnerError)?;

        Ok(())
    }

    async fn invoke<S: ProcessSpawner, R: Runtime>(
        &mut self,
        context: VmmExecutorContext<S, R>,
        config_path: Option<PathBuf>,
    ) -> Result<ProcessHandle<R::Process>, VmmExecutorError> {
        let (uid, gid) = match context.ownership_model.as_downgrade() {
            Some(values) => values,
            None => (*PROCESS_UID, *PROCESS_GID),
        };

        let mut arguments = self
            .jailer_arguments
            .join(uid, gid, &context.installation.firecracker_path);
        let mut binary_path = context.installation.jailer_path.clone();
        arguments.push("--".to_string());
        arguments.extend(self.vmm_arguments.join(config_path));

        for command_modifier in &self.command_modifier_chain {
            command_modifier.apply(&mut binary_path, &mut arguments);
        }

        // nulling the pipes is redundant since jailer can do this itself via daemonization
        let mut process = context
            .process_spawner
            .spawn(&binary_path, arguments, false, &context.runtime)
            .await
            .map_err(VmmExecutorError::ProcessSpawnFailed)?;

        if self.jailer_arguments.daemonize || self.jailer_arguments.exec_in_new_pid_ns {
            let (_, jail_path) = self.get_paths(&context.installation);
            let pid_file_path = jail_path.join(format!(
                "{}.pid",
                context
                    .installation
                    .firecracker_path
                    .file_name()
                    .and_then(|f| f.to_str())
                    .unwrap_or("firecracker")
            ));

            let exit_status = process.wait().await.map_err(VmmExecutorError::ProcessWaitError)?;
            if !exit_status.success() {
                return Err(VmmExecutorError::ProcessExitedWithIncorrectStatus(exit_status));
            }

            upgrade_owner(
                &pid_file_path,
                context.ownership_model,
                &context.process_spawner,
                &context.runtime,
            )
            .await
            .map_err(VmmExecutorError::ChangeOwnerError)?;

            let pid = loop {
                if let Ok(pid_string) = context.runtime.filesystem().read_to_string(&pid_file_path).await {
                    if let Ok(pid) = pid_string.trim_end().parse() {
                        break pid;
                    }
                }
            };

            Ok(ProcessHandle::with_pidfd(pid, context.runtime).map_err(VmmExecutorError::PidfdAllocationError)?)
        } else {
            Ok(ProcessHandle::with_child(process, false))
        }
    }

    async fn cleanup<S: ProcessSpawner, R: Runtime>(
        &mut self,
        context: VmmExecutorContext<S, R>,
        _resource_references: VmmResourceReferences<'_>,
    ) -> Result<(), VmmExecutorError> {
        let (_, jail_path) = self.get_paths(&context.installation);

        upgrade_owner(
            &jail_path,
            context.ownership_model,
            &context.process_spawner,
            &context.runtime,
        )
        .await
        .map_err(VmmExecutorError::ChangeOwnerError)?;

        let Some(jail_parent_path) = jail_path.parent() else {
            return Err(VmmExecutorError::ExpectedDirectoryParentMissing(jail_path));
        };

        context
            .runtime
            .filesystem()
            .remove_dir_all(jail_parent_path)
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
