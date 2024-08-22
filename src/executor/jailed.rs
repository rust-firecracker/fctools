use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use async_trait::async_trait;
use tokio::{fs, process::Child};

use crate::shell_spawner::ShellSpawner;

use super::{
    arguments::{FirecrackerApiSocket, FirecrackerArguments, FirecrackerConfigOverride, JailerArguments},
    command_modifier::{apply_command_modifier_chain, CommandModifier},
    create_file_with_tree, force_chown, force_mkdir,
    installation::FirecrackerInstallation,
    FirecrackerExecutorError, VmmExecutor,
};

/// An executor that uses the "jailer" binary for maximum security and isolation, dropping privileges to then
/// run "firecracker". This executor, due to jailer design, can only run as root, even though the "firecracker"
/// process itself won't.
#[derive(Debug)]
pub struct JailedVmmExecutor<R: JailRenamer + 'static> {
    /// The arguments passed to the "firecracker" binary
    firecracker_arguments: FirecrackerArguments,
    /// The arguments passed to the "jailer" binary
    jailer_arguments: JailerArguments,
    /// The method of how to move VM resources into the jail
    jail_move_method: JailMoveMethod,
    /// The jail renamer that will be applied to VM resource paths during the move process
    jail_renamer: R,
    command_modifier_chain: Vec<Box<dyn CommandModifier>>,
}

impl<R: JailRenamer + 'static> JailedVmmExecutor<R> {
    pub fn new(
        firecracker_arguments: FirecrackerArguments,
        jailer_arguments: JailerArguments,
        jail_renamer: R,
    ) -> Self {
        Self {
            firecracker_arguments,
            jailer_arguments,
            jail_move_method: JailMoveMethod::Copy,
            jail_renamer,
            command_modifier_chain: Vec::new(),
        }
    }

    pub fn jail_move_method(mut self, jail_move_method: JailMoveMethod) -> Self {
        self.jail_move_method = jail_move_method;
        self
    }

    pub fn command_modifier(mut self, command_modifier: impl Into<Box<dyn CommandModifier>>) -> Self {
        self.command_modifier_chain.push(command_modifier.into());
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
    Copy,
    HardLink,
    /// First try to hard link, then resort to copying as a fallback
    HardLinkWithCopyFallback,
}

#[async_trait]
impl<T: JailRenamer + 'static> VmmExecutor for JailedVmmExecutor<T> {
    fn get_socket_path(&self) -> Option<PathBuf> {
        match &self.firecracker_arguments.api_socket {
            FirecrackerApiSocket::Disabled => None,
            FirecrackerApiSocket::Enabled(socket_path) => Some(self.get_jail_path().jail_join(&socket_path)),
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
            .map_err(FirecrackerExecutorError::IoError)?
        {
            force_mkdir(chroot_base_dir, shell_spawner).await?;
        }
        force_chown(chroot_base_dir, shell_spawner).await?; // grants access to jail as well

        // Create jail and delete previous one if necessary
        let jail_path = self.get_jail_path();
        if fs::try_exists(&jail_path)
            .await
            .map_err(FirecrackerExecutorError::IoError)?
        {
            fs::remove_dir_all(&jail_path)
                .await
                .map_err(FirecrackerExecutorError::IoError)?;
        }
        fs::create_dir_all(&jail_path)
            .await
            .map_err(FirecrackerExecutorError::IoError)?;

        // Ensure socket parent directory exists so that the firecracker process can bind inside of it
        if let FirecrackerApiSocket::Enabled(ref socket_path) = self.firecracker_arguments.api_socket {
            if let Some(socket_parent_dir) = socket_path.parent() {
                fs::create_dir_all(jail_path.jail_join(socket_parent_dir))
                    .await
                    .map_err(FirecrackerExecutorError::IoError)?;
            }
        }

        // Ensure argument paths exist
        if let Some(ref log_path) = self.firecracker_arguments.log_path {
            create_file_with_tree(jail_path.jail_join(log_path)).await?;
        }
        if let Some(ref metrics_path) = self.firecracker_arguments.metrics_path {
            create_file_with_tree(jail_path.jail_join(metrics_path)).await?;
        }

        // Apply jail renamer and move in the resources in parallel (via a join set)
        let mut path_mappings = HashMap::with_capacity(outer_paths.len());
        let mut join_handles = Vec::new();

        for outer_path in outer_paths {
            if !fs::try_exists(&outer_path)
                .await
                .map_err(FirecrackerExecutorError::IoError)?
            {
                return Err(FirecrackerExecutorError::ExpectedResourceMissing(outer_path.clone()));
            }

            force_chown(&outer_path, shell_spawner).await?;

            let inner_path = self
                .jail_renamer
                .rename_for_jail(&outer_path)
                .map_err(FirecrackerExecutorError::ToInnerPathFailed)?;
            let expanded_inner_path = jail_path.jail_join(inner_path.as_ref());
            path_mappings.insert(outer_path.clone(), inner_path);

            // Inexpensively clone into the future
            let jail_move_method = self.jail_move_method;

            join_handles.push(tokio::spawn(async move {
                if let Some(new_path_parent_dir) = expanded_inner_path.parent() {
                    fs::create_dir_all(new_path_parent_dir).await?;
                }
                match jail_move_method {
                    JailMoveMethod::Copy => fs::copy(outer_path, expanded_inner_path).await.map(|_| ()),
                    JailMoveMethod::HardLink => fs::hard_link(outer_path, expanded_inner_path).await,
                    JailMoveMethod::HardLinkWithCopyFallback => {
                        let hardlink_result = fs::hard_link(&outer_path, &expanded_inner_path).await;
                        if let Err(_) = hardlink_result {
                            fs::copy(&outer_path, &expanded_inner_path).await.map(|_| ())
                        } else {
                            hardlink_result
                        }
                    }
                }
            }));
        }

        for join_handle in join_handles {
            join_handle
                .await
                .map_err(FirecrackerExecutorError::TaskJoinFailed)?
                .map_err(FirecrackerExecutorError::IoError)?;
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
        let mut shell_command = format!(
            "{} {jailer_args} -- {firecracker_args}",
            installation.jailer_path.to_string_lossy()
        );
        apply_command_modifier_chain(&mut shell_command, &self.command_modifier_chain);

        shell
            .spawn(shell_command)
            .await
            .map_err(FirecrackerExecutorError::ShellSpawnFailed)
    }

    async fn cleanup(&self, _shell_spawner: &impl ShellSpawner) -> Result<(), FirecrackerExecutorError> {
        let jail_path = self.get_jail_path();
        let jail_parent_path = jail_path
            .parent()
            .ok_or(FirecrackerExecutorError::ExpectedDirectoryParentMissing)?;

        // Delete entire jail (../{id}/root) recursively
        fs::remove_dir_all(jail_parent_path)
            .await
            .map_err(FirecrackerExecutorError::IoError)
    }
}

impl<R: JailRenamer + 'static> JailedVmmExecutor<R> {
    fn get_jail_path(&self) -> PathBuf {
        let chroot_base_dir = match self.jailer_arguments.chroot_base_dir {
            Some(ref path) => path.clone(),
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
pub enum JailRenamerError {
    PathHasNoFilename,
    PathIsUnmapped(PathBuf),
    Other(Box<dyn std::error::Error + Send>),
}

/// A trait defining a method of conversion between an outer path and an inner path. This conversion
/// should always produce the same path (or error) for the same given outside-jail path.
pub trait JailRenamer: Send + Sync + Clone {
    fn rename_for_jail(&self, outer_path: &Path) -> Result<PathBuf, JailRenamerError>;
}

/// A resolver that transforms a host path with filename (including extension) "p" into /p
/// inside the jail. Given that files have unique names, this should be enough for most scenarios.
#[derive(Debug, Clone, Default)]
pub struct FlatJailRenamer {}

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

    pub fn map(&mut self, outside_path: impl Into<PathBuf>, jail_path: impl Into<PathBuf>) -> &mut Self {
        self.mappings.insert(outside_path.into(), jail_path.into());
        self
    }

    pub fn map_all(&mut self, mappings: impl IntoIterator<Item = (PathBuf, PathBuf)>) -> &mut Self {
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

    use crate::executor::jailed::{JailJoin, JailRenamerError, MappingJailRenamer};

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
        let mut renamer = MappingJailRenamer::new();
        renamer
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
