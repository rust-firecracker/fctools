use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
};

use tokio::{process::Child, task::JoinSet};

use crate::{
    fs_backend::FsBackend,
    process_spawner::ProcessSpawner,
    vmm::{
        arguments::{
            command_modifier::CommandModifier, jailer::JailerArguments, VmmApiSocket, VmmArguments,
            VmmConfigurationOverride,
        },
        installation::VmmInstallation,
    },
};

use super::{change_owner, create_file_with_tree, VmmExecutor, VmmExecutorError, PROCESS_GID, PROCESS_UID};

/// A [VmmExecutor] that uses the "jailer" binary for maximum security and isolation, dropping privileges to then
/// run "firecracker". This executor, due to jailer design, can only run as root, even though the "firecracker"
/// process itself won't unless configured to run as UID 0 and GID 0 (root).
#[derive(Debug)]
pub struct JailedVmmExecutor<R: JailRenamer + 'static> {
    vmm_arguments: VmmArguments,
    jailer_arguments: JailerArguments,
    jail_move_method: JailMoveMethod,
    jail_renamer: R,
    downgrade: Option<(u32, u32)>,
    command_modifier_chain: Vec<Box<dyn CommandModifier>>,
}

impl<R: JailRenamer + 'static> JailedVmmExecutor<R> {
    pub fn new(vmm_arguments: VmmArguments, jailer_arguments: JailerArguments, jail_renamer: R) -> Self {
        let mut downgrade = None;
        if jailer_arguments.uid != *PROCESS_UID || jailer_arguments.gid != *PROCESS_GID {
            downgrade = Some((jailer_arguments.uid, jailer_arguments.gid));
        }

        Self {
            vmm_arguments,
            jailer_arguments,
            jail_move_method: JailMoveMethod::Copy,
            jail_renamer,
            downgrade,
            command_modifier_chain: Vec::new(),
        }
    }

    pub fn jail_move_method(mut self, jail_move_method: JailMoveMethod) -> Self {
        self.jail_move_method = jail_move_method;
        self
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

impl<T: JailRenamer + 'static> VmmExecutor for JailedVmmExecutor<T> {
    fn get_socket_path(&self, installation: &VmmInstallation) -> Option<PathBuf> {
        match &self.vmm_arguments.api_socket {
            VmmApiSocket::Disabled => None,
            VmmApiSocket::Enabled(socket_path) => Some(self.get_jail_path(installation).jail_join(&socket_path)),
        }
    }

    fn inner_to_outer_path(&self, installation: &VmmInstallation, inner_path: &Path) -> PathBuf {
        self.get_jail_path(installation).jail_join(inner_path)
    }

    fn is_traceless(&self) -> bool {
        true
    }

    fn ownership_downgrade(&self) -> Option<(u32, u32)> {
        self.downgrade
    }

    async fn prepare(
        &self,
        installation: &VmmInstallation,
        process_spawner: Arc<impl ProcessSpawner>,
        fs_backend: Arc<impl FsBackend>,
        outer_paths: Vec<PathBuf>,
    ) -> Result<HashMap<PathBuf, PathBuf>, VmmExecutorError> {
        // Ensure chroot base dir exists and is accessible
        let chroot_base_dir = match &self.jailer_arguments.chroot_base_dir {
            Some(dir) => &dir,
            None => &PathBuf::from("/srv/jailer"),
        };

        // Create jail and delete previous one if necessary
        let jail_path = Arc::new(self.get_jail_path(installation));
        if process_spawner.upgrades_ownership() {
            change_owner(&chroot_base_dir, *PROCESS_UID, *PROCESS_GID, process_spawner.as_ref()).await?;
        }

        if fs_backend
            .check_exists(&jail_path)
            .await
            .map_err(VmmExecutorError::FsBackendError)?
        {
            fs_backend
                .remove_dir_all(&jail_path)
                .await
                .map_err(VmmExecutorError::FsBackendError)?;
        }
        fs_backend
            .create_dir_all(&jail_path)
            .await
            .map_err(VmmExecutorError::FsBackendError)?;

        let mut join_set = JoinSet::new();

        // Ensure socket parent directory exists so that the firecracker process can bind inside of it
        if let VmmApiSocket::Enabled(ref socket_path) = self.vmm_arguments.api_socket {
            if let Some(socket_parent_dir) = socket_path.parent() {
                let socket_parent_dir = socket_parent_dir.to_owned();
                let fs_backend = fs_backend.clone();
                let jail_path = jail_path.clone();
                join_set.spawn(async move {
                    fs_backend
                        .create_dir_all(&jail_path.jail_join(&socket_parent_dir))
                        .await
                        .map_err(VmmExecutorError::FsBackendError)
                });
            }
        }

        // Ensure argument paths exist
        if let Some(ref log_path) = self.vmm_arguments.log_path.clone() {
            join_set.spawn(create_file_with_tree(
                fs_backend.clone(),
                process_spawner.clone(),
                self.downgrade,
                jail_path.jail_join(log_path),
            ));
        }

        if let Some(ref metrics_path) = self.vmm_arguments.metrics_path.clone() {
            join_set.spawn(create_file_with_tree(
                fs_backend.clone(),
                process_spawner.clone(),
                self.downgrade,
                jail_path.jail_join(metrics_path),
            ));
        }

        // Apply jail renamer and move in the resources in parallel (via a join set)
        let mut path_mappings = HashMap::with_capacity(outer_paths.len());

        for outer_path in outer_paths {
            if !fs_backend
                .check_exists(&outer_path)
                .await
                .map_err(VmmExecutorError::FsBackendError)?
            {
                return Err(VmmExecutorError::ExpectedResourceMissing(outer_path.clone()));
            }

            let inner_path = self
                .jail_renamer
                .rename_for_jail(&outer_path)
                .map_err(VmmExecutorError::JailRenamerFailed)?;
            let expanded_inner_path = jail_path.jail_join(inner_path.as_ref());
            path_mappings.insert(outer_path.clone(), inner_path);

            let jail_move_method = self.jail_move_method;
            let fs_backend = fs_backend.clone();

            join_set.spawn(async move {
                if let Some(new_path_parent_dir) = expanded_inner_path.parent() {
                    fs_backend
                        .create_dir_all(new_path_parent_dir)
                        .await
                        .map_err(VmmExecutorError::FsBackendError)?;
                }

                match jail_move_method {
                    JailMoveMethod::Copy => fs_backend
                        .copy(&outer_path, &expanded_inner_path)
                        .await
                        .map_err(VmmExecutorError::FsBackendError),
                    JailMoveMethod::HardLink => fs_backend
                        .hard_link(&outer_path, &expanded_inner_path)
                        .await
                        .map_err(VmmExecutorError::FsBackendError),
                    JailMoveMethod::HardLinkWithCopyFallback => {
                        let hardlink_result = fs_backend
                            .hard_link(&outer_path, &expanded_inner_path)
                            .await
                            .map_err(VmmExecutorError::FsBackendError);
                        if let Err(_) = hardlink_result {
                            fs_backend
                                .copy(&outer_path, &expanded_inner_path)
                                .await
                                .map_err(VmmExecutorError::FsBackendError)
                        } else {
                            hardlink_result
                        }
                    }
                }
            });
        }

        while let Some(result) = join_set.join_next().await {
            result.map_err(VmmExecutorError::TaskJoinFailed)??;
        }

        if let Some((uid, gid)) = self.downgrade {
            change_owner(&jail_path, uid, gid, process_spawner.as_ref()).await?;
        }

        Ok(path_mappings)
    }

    async fn invoke(
        &self,
        installation: &VmmInstallation,
        process_spawner: Arc<impl ProcessSpawner>,
        configuration_override: VmmConfigurationOverride,
    ) -> Result<Child, VmmExecutorError> {
        let mut arguments = self.jailer_arguments.join(&installation.firecracker_path);
        let mut binary_path = installation.jailer_path.clone();
        arguments.push("--".to_string());
        arguments.extend(self.vmm_arguments.join(configuration_override));

        for command_modifier in &self.command_modifier_chain {
            command_modifier.apply(&mut binary_path, &mut arguments);
        }

        // nulling the pipes is redundant since jailer can do this itself via daemonization
        process_spawner
            .spawn(&binary_path, arguments, false)
            .await
            .map_err(VmmExecutorError::ProcessSpawnFailed)
    }

    async fn cleanup(
        &self,
        installation: &VmmInstallation,
        process_spawner: Arc<impl ProcessSpawner>,
        fs_backend: Arc<impl FsBackend>,
    ) -> Result<(), VmmExecutorError> {
        let jail_path = self.get_jail_path(installation);

        if process_spawner.upgrades_ownership() {
            change_owner(&jail_path, *PROCESS_UID, *PROCESS_GID, process_spawner.as_ref()).await?;
        }

        let jail_parent_path = jail_path
            .parent()
            .ok_or(VmmExecutorError::ExpectedDirectoryParentMissing)?;
        fs_backend
            .remove_dir_all(jail_parent_path)
            .await
            .map_err(VmmExecutorError::FsBackendError)
    }
}

impl<R: JailRenamer + 'static> JailedVmmExecutor<R> {
    fn get_jail_path(&self, installation: &VmmInstallation) -> PathBuf {
        let chroot_base_dir = match self.jailer_arguments.chroot_base_dir {
            Some(ref path) => path.clone(),
            None => PathBuf::from("/srv/jailer"),
        };

        // example: /srv/jailer/firecracker/1/root
        chroot_base_dir
            .join(
                match installation.firecracker_path.file_name().map(|f| f.to_str()).flatten() {
                    Some(filename) => filename,
                    None => "firecracker",
                },
            )
            .join(self.jailer_arguments.jail_id.as_ref())
            .join("root")
    }
}

/// An error that can be emitted by a [JailRenamer].
#[derive(Debug, thiserror::Error)]
pub enum JailRenamerError {
    #[error("The given path which is supposed to be a file has no filename")]
    PathHasNoFilename,
    #[error("A conversion of the outer path `{0}` to an inner path was not configured")]
    PathIsUnmapped(PathBuf),
    #[error("Another error occurred: `{0}`")]
    Other(Box<dyn std::error::Error + Send>),
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
                    .ok_or(JailRenamerError::PathHasNoFilename)?
                    .to_string_lossy(),
        ))
    }
}

/// A jail renamer that uses a lookup table from host to jail in order to transform paths.
#[derive(Debug, Clone)]
pub struct MappingJailRenamer {
    mappings: HashMap<PathBuf, PathBuf>,
}

impl MappingJailRenamer {
    pub fn new() -> Self {
        Self {
            mappings: HashMap::new(),
        }
    }

    pub fn map(mut self, outside_path: impl Into<PathBuf>, jail_path: impl Into<PathBuf>) -> Self {
        self.mappings.insert(outside_path.into(), jail_path.into());
        self
    }

    pub fn map_all(mut self, mappings: impl IntoIterator<Item = (PathBuf, PathBuf)>) -> Self {
        self.mappings.extend(mappings);
        self
    }
}

impl From<HashMap<PathBuf, PathBuf>> for MappingJailRenamer {
    fn from(value: HashMap<PathBuf, PathBuf>) -> Self {
        Self { mappings: value }
    }
}

impl JailRenamer for MappingJailRenamer {
    fn rename_for_jail(&self, outside_path: &Path) -> Result<PathBuf, JailRenamerError> {
        let jail_path = self
            .mappings
            .get(outside_path)
            .ok_or_else(|| JailRenamerError::PathIsUnmapped(outside_path.to_owned()))?;
        Ok(jail_path.clone())
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

    use crate::vmm::executor::jailed::{JailJoin, JailRenamerError, MappingJailRenamer};

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

    #[test]
    fn mapping_jail_renamer_moves_correctly() {
        let renamer = MappingJailRenamer::new()
            .map("/etc/a", "/tmp/a")
            .map("/opt/b", "/etc/b")
            .map("/tmp/c", "/c");
        assert_renamer(&renamer, "/etc/a", "/tmp/a");
        assert_renamer(&renamer, "/opt/b", "/etc/b");
        assert_renamer(&renamer, "/tmp/c", "/c");
        assert_matches::assert_matches!(
            renamer.rename_for_jail(PathBuf::from("/tmp/unknown").as_ref()),
            Err(JailRenamerError::PathIsUnmapped(_))
        );
    }

    fn assert_renamer(renamer: &impl JailRenamer, path: &str, expectation: &str) {
        assert_eq!(
            renamer.rename_for_jail(&PathBuf::from(path)).unwrap().to_str().unwrap(),
            expectation
        );
    }
}
