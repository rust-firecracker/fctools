use std::path::{Path, PathBuf};

use crate::{
    process_spawner::ProcessSpawner,
    runtime::{Runtime, RuntimeChild},
    vmm::{
        arguments::{command_modifier::CommandModifier, jailer::JailerArguments, VmmApiSocket, VmmArguments},
        installation::VmmInstallation,
        ownership::{downgrade_owner_recursively, upgrade_owner, PROCESS_GID, PROCESS_UID},
        resource::ResourceType,
    },
};

use super::{process_handle::ProcessHandle, VmmExecutor, VmmExecutorContext, VmmExecutorError};

/// A [VmmExecutor] that uses the "jailer" binary for maximum security and isolation, dropping privileges to then
/// run "firecracker". The "jailer", by design, can only run as "root", even though the "firecracker" process itself
/// won't do so unless explicitly configured to run as UID 0 and GID 0, which corresponds to "root".
/// A [JailedVmmExecutor] is tied to a [LocalPathResolver] it uses in order to function.
#[derive(Debug)]
pub struct JailedVmmExecutor<P: LocalPathResolver> {
    vmm_arguments: VmmArguments,
    jailer_arguments: JailerArguments,
    local_path_resolver: P,
    command_modifier_chain: Vec<Box<dyn CommandModifier>>,
}

impl<P: LocalPathResolver> JailedVmmExecutor<P> {
    /// Create a new [JailedVmmExecutor] from [VmmArguments], [JailerArguments] and the specified [LocalPathResolver]
    /// implementation's instance.
    pub fn new(vmm_arguments: VmmArguments, jailer_arguments: JailerArguments, local_path_resolver: P) -> Self {
        Self {
            vmm_arguments,
            jailer_arguments,
            local_path_resolver,
            command_modifier_chain: Vec::new(),
        }
    }

    /// Add a [CommandModifier] implementation to the end of the [CommandModifier] chain.
    pub fn command_modifier<M: CommandModifier>(mut self, command_modifier: M) -> Self {
        self.command_modifier_chain.push(Box::new(command_modifier));
        self
    }

    /// Sequentially insert an iterator of boxed [CommandModifier]s to the end of the [CommandModifier] chain.
    pub fn command_modifiers<I: IntoIterator<Item = Box<dyn CommandModifier>>>(mut self, command_modifiers: I) -> Self {
        self.command_modifier_chain.extend(command_modifiers);
        self
    }
}

impl<P: LocalPathResolver> VmmExecutor for JailedVmmExecutor<P> {
    fn get_socket_path(&self, installation: &VmmInstallation) -> Option<PathBuf> {
        match &self.vmm_arguments.api_socket {
            VmmApiSocket::Disabled => None,
            VmmApiSocket::Enabled(socket_path) => Some(self.get_paths(installation).1.jail_join(socket_path)),
        }
    }

    fn resolve_effective_path(&self, installation: &VmmInstallation, local_path: PathBuf) -> PathBuf {
        self.get_paths(installation).1.jail_join(&local_path)
    }

    async fn prepare<S: ProcessSpawner, R: Runtime>(
        &self,
        context: VmmExecutorContext<'_, S, R>,
    ) -> Result<(), VmmExecutorError> {
        // Create the jail and delete the previous one if necessary
        let (chroot_base_dir, jail_path) = self.get_paths(&context.installation);
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
            .fs_exists(&jail_path)
            .await
            .map_err(VmmExecutorError::FilesystemError)?
        {
            context
                .runtime
                .fs_remove_dir_all(&jail_path)
                .await
                .map_err(VmmExecutorError::FilesystemError)?;
        }

        context
            .runtime
            .fs_create_dir_all(&jail_path)
            .await
            .map_err(VmmExecutorError::FilesystemError)?;

        // Ensure that the socket parent directory exists so that the firecracker process can bind inside of it
        if let VmmApiSocket::Enabled(ref socket_path) = self.vmm_arguments.api_socket {
            if let Some(socket_parent_dir) = socket_path.parent() {
                context
                    .runtime
                    .fs_create_dir_all(&jail_path.jail_join(socket_parent_dir))
                    .await
                    .map_err(VmmExecutorError::FilesystemError)?;
            }
        }

        for resource in context.resources.iter().chain(self.vmm_arguments.get_resources()) {
            match resource.get_type() {
                ResourceType::Created(_) | ResourceType::Produced => {
                    resource.start_initialization(jail_path.jail_join(&resource.get_source_path()), None)
                }
                ResourceType::Moved(_) => {
                    let local_path = self
                        .local_path_resolver
                        .resolve_local_path(&resource.get_source_path())
                        .map_err(VmmExecutorError::LocalPathResolverError)?;
                    let effective_path = jail_path.jail_join(&local_path);
                    resource.start_initialization(effective_path, Some(local_path))
                }
            }
            .map_err(VmmExecutorError::ResourceSystemError)?;
        }

        Ok(())
    }

    async fn invoke<S: ProcessSpawner, R: Runtime>(
        &self,
        context: VmmExecutorContext<'_, S, R>,
        config_path: Option<PathBuf>,
    ) -> Result<ProcessHandle<R>, VmmExecutorError> {
        downgrade_owner_recursively(
            &self.get_paths(&context.installation).1,
            context.ownership_model,
            &context.runtime,
        )
        .await
        .map_err(VmmExecutorError::ChangeOwnerError)?;

        let (uid, gid) = match context.ownership_model.as_downgrade() {
            Some(values) => values,
            None => (*PROCESS_UID, *PROCESS_GID),
        };

        let mut arguments = self
            .jailer_arguments
            .join(uid, gid, context.installation.get_firecracker_path());
        let mut binary_path = context.installation.get_jailer_path().to_owned();
        arguments.push("--".to_string());
        arguments.extend(self.vmm_arguments.join(config_path));

        for command_modifier in &self.command_modifier_chain {
            command_modifier.apply(&mut binary_path, &mut arguments);
        }

        // Nulling the pipes is redundant since the jailer can do this itself via daemonization
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
                    .get_firecracker_path()
                    .file_name()
                    .and_then(|f| f.to_str())
                    .unwrap_or("firecracker")
            ));

            let exit_status = process.wait().await.map_err(VmmExecutorError::ProcessWaitError)?;
            if !exit_status.success() {
                return Err(VmmExecutorError::ProcessExitedWithNonZeroStatus(exit_status));
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
                if let Ok(pid_string) = context.runtime.fs_read_to_string(&pid_file_path).await {
                    if let Ok(pid) = pid_string.trim_end().parse() {
                        break pid;
                    }
                }
            };

            Ok(ProcessHandle::from_pidfd(pid, context.runtime).map_err(VmmExecutorError::PidfdAllocationError)?)
        } else {
            Ok(ProcessHandle::from_child(process, false))
        }
    }

    async fn cleanup<S: ProcessSpawner, R: Runtime>(
        &self,
        context: VmmExecutorContext<'_, S, R>,
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
            .fs_remove_dir_all(jail_parent_path)
            .await
            .map_err(VmmExecutorError::FilesystemError)
    }
}

impl<P: LocalPathResolver> JailedVmmExecutor<P> {
    fn get_paths(&self, installation: &VmmInstallation) -> (PathBuf, PathBuf) {
        let chroot_base_dir = self
            .jailer_arguments
            .chroot_base_dir
            .clone()
            .unwrap_or_else(|| PathBuf::from("/srv/jailer"));

        // Example of a resulting jail_path: /srv/jailer/firecracker/1/root
        let jail_path = chroot_base_dir
            .join(
                installation
                    .get_firecracker_path()
                    .file_name()
                    .and_then(|f| f.to_str())
                    .unwrap_or("firecracker"),
            )
            .join(self.jailer_arguments.jail_id.as_ref())
            .join("root");

        (chroot_base_dir, jail_path)
    }
}

/// An error that can be emitted by a [LocalPathResolver] implementation.
#[derive(Debug)]
pub enum LocalPathResolverError {
    /// The provided source path had no filename.
    SourcePathHasNoFilename,
    /// Another type of error occurred. This error variant is reserved for custom [LocalPathResolver] implementations
    /// and is not used by the built-in [FlatLocalPathResolver].
    Other(Box<dyn std::error::Error + Send>),
}

impl std::error::Error for LocalPathResolverError {}

impl std::fmt::Display for LocalPathResolverError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LocalPathResolverError::SourcePathHasNoFilename => write!(f, "The provided source path had no filename"),
            LocalPathResolverError::Other(err) => write!(f, "Another error occurred: {err}"),
        }
    }
}

/// A trait defining a method of resolving a resource's local path from its source path. This conversion
/// should always produce the same local path (or error) for the same given source path.
pub trait LocalPathResolver: Send + Sync + Clone {
    /// Convert the provided source path to a local path within the jail.
    fn resolve_local_path(&self, source_path: &Path) -> Result<PathBuf, LocalPathResolverError>;
}

/// A [LocalPathResolver] that transforms a source path with filename (including extension) "p" into a "/p" local path.
/// Given that files have unique names, this should be sufficient for most production scenarios.
#[derive(Debug, Clone, Default)]
pub struct FlatLocalPathResolver;

impl LocalPathResolver for FlatLocalPathResolver {
    fn resolve_local_path(&self, outside_path: &Path) -> Result<PathBuf, LocalPathResolverError> {
        Ok(PathBuf::from(
            "/".to_owned()
                + &outside_path
                    .file_name()
                    .ok_or(LocalPathResolverError::SourcePathHasNoFilename)?
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

    use super::{FlatLocalPathResolver, LocalPathResolver};

    #[test]
    fn jail_join_performs_correctly() {
        assert_eq!(
            PathBuf::from("/jail").jail_join(&PathBuf::from("/inner")),
            PathBuf::from("/jail/inner")
        );
    }

    #[test]
    fn flat_local_path_resolver_moves_correctly() {
        let renamer = FlatLocalPathResolver::default();
        assert_local_path_resolver(&renamer, "/opt/file", "/file");
        assert_local_path_resolver(&renamer, "/tmp/some_path.txt", "/some_path.txt");
        assert_local_path_resolver(&renamer, "/some/complex/outside/path/filename.ext4", "/filename.ext4");
    }

    fn assert_local_path_resolver<P: LocalPathResolver>(renamer: &P, path: &str, expectation: &str) {
        assert_eq!(
            renamer
                .resolve_local_path(&PathBuf::from(path))
                .unwrap()
                .to_str()
                .unwrap(),
            expectation
        );
    }
}
