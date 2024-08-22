use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use async_trait::async_trait;
use tokio::{fs, process::Child};

use crate::shell_spawner::ShellSpawner;

use super::{
    arguments::{FirecrackerApiSocket, FirecrackerArguments, FirecrackerConfigOverride},
    command_modifier::{apply_command_modifier_chain, CommandModifier},
    create_file_with_tree, force_chown,
    installation::FirecrackerInstallation,
    FirecrackerExecutorError, VmmExecutor,
};

/// An executor that uses the "firecracker" binary directly, without jailing it or ensuring it doesn't run as root.
/// This executor allows rootless execution, given that the user has access to /dev/kvm.
#[derive(Debug)]
pub struct UnrestrictedVmmExecutor {
    /// Arguments passed to the "firecracker" binary
    firecracker_arguments: FirecrackerArguments,
    command_modifier_chain: Vec<Box<dyn CommandModifier>>,
    remove_metrics_on_cleanup: bool,
    remove_logs_on_cleanup: bool,
}

impl UnrestrictedVmmExecutor {
    pub fn new(firecracker_arguments: FirecrackerArguments) -> Self {
        Self {
            firecracker_arguments,
            command_modifier_chain: Vec::new(),
            remove_metrics_on_cleanup: false,
            remove_logs_on_cleanup: false,
        }
    }

    pub fn command_modifier(mut self, command_modifier: impl Into<Box<dyn CommandModifier>>) -> Self {
        self.command_modifier_chain.push(command_modifier.into());
        self
    }

    pub fn command_modifiers(mut self, command_modifiers: impl IntoIterator<Item = Box<dyn CommandModifier>>) -> Self {
        self.command_modifier_chain.extend(command_modifiers);
        self
    }

    pub fn remove_metrics_on_cleanup(mut self) -> Self {
        self.remove_metrics_on_cleanup = true;
        self
    }

    pub fn remove_logs_on_cleanup(mut self) -> Self {
        self.remove_logs_on_cleanup = true;
        self
    }
}

#[async_trait]
impl VmmExecutor for UnrestrictedVmmExecutor {
    fn get_socket_path(&self) -> Option<PathBuf> {
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
            if !fs::try_exists(path).await.map_err(FirecrackerExecutorError::IoError)? {
                return Err(FirecrackerExecutorError::ExpectedResourceMissing(path.clone()));
            }
            force_chown(&path, shell_spawner).await?;
        }

        if let FirecrackerApiSocket::Enabled(ref socket_path) = self.firecracker_arguments.api_socket {
            if fs::try_exists(socket_path)
                .await
                .map_err(FirecrackerExecutorError::IoError)?
            {
                force_chown(socket_path, shell_spawner).await?;
                fs::remove_file(socket_path)
                    .await
                    .map_err(FirecrackerExecutorError::IoError)?;
            }
        }

        // Ensure argument paths exist
        if let Some(ref log_path) = self.firecracker_arguments.log_path {
            create_file_with_tree(log_path).await?;
        }
        if let Some(ref metrics_path) = self.firecracker_arguments.metrics_path {
            create_file_with_tree(metrics_path).await?;
        }

        Ok(outer_paths.into_iter().map(|path| (path.clone(), path)).collect())
    }

    async fn invoke(
        &self,
        shell: &impl ShellSpawner,
        installation: &FirecrackerInstallation,
        config_override: FirecrackerConfigOverride,
    ) -> Result<Child, FirecrackerExecutorError> {
        let arguments = self.firecracker_arguments.join(config_override);
        let mut shell_command = format!("{} {arguments}", installation.firecracker_path.to_string_lossy());
        apply_command_modifier_chain(&mut shell_command, &self.command_modifier_chain);

        let child = shell
            .spawn(shell_command)
            .await
            .map_err(FirecrackerExecutorError::ShellSpawnFailed)?;
        Ok(child)
    }

    async fn cleanup(&self, shell_spawner: &impl ShellSpawner) -> Result<(), FirecrackerExecutorError> {
        if let FirecrackerApiSocket::Enabled(ref socket_path) = self.firecracker_arguments.api_socket {
            if fs::try_exists(socket_path)
                .await
                .map_err(FirecrackerExecutorError::IoError)?
            {
                force_chown(socket_path, shell_spawner).await?;
                fs::remove_file(socket_path)
                    .await
                    .map_err(FirecrackerExecutorError::IoError)?;
            }
        }

        if self.remove_logs_on_cleanup {
            if let Some(ref log_path) = self.firecracker_arguments.log_path {
                fs::remove_file(log_path)
                    .await
                    .map_err(FirecrackerExecutorError::IoError)?;
            }
        }

        if self.remove_metrics_on_cleanup {
            if let Some(ref metrics_path) = self.firecracker_arguments.metrics_path {
                fs::remove_file(metrics_path)
                    .await
                    .map_err(FirecrackerExecutorError::IoError)?;
            }
        }

        Ok(())
    }
}
