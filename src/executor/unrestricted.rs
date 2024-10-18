use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
};

use tokio::{process::Child, task::JoinSet};

use crate::{fs_backend::FsBackend, runner::Runner};

use super::{
    argument_modifier::{apply_argument_modifier_chain, ArgumentModifier},
    arguments::{ConfigurationFileOverride, VmmApiSocket, VmmArguments},
    create_file_with_tree, force_chown,
    installation::VmmInstallation,
    join_on_set, VmmExecutor, VmmExecutorError,
};

/// An executor that uses the "firecracker" binary directly, without jailing it or ensuring it doesn't run as root.
/// This executor allows rootless execution, given that the user has access to /dev/kvm.
#[derive(Debug)]
pub struct UnrestrictedVmmExecutor {
    vmm_arguments: VmmArguments,
    argument_modifier_chain: Vec<Box<dyn ArgumentModifier>>,
    remove_metrics_on_cleanup: bool,
    remove_logs_on_cleanup: bool,
    pipes_to_null: bool,
    id: Option<VmmId>,
}

impl UnrestrictedVmmExecutor {
    pub fn new(vmm_arguments: VmmArguments) -> Self {
        Self {
            vmm_arguments,
            argument_modifier_chain: Vec::new(),
            remove_metrics_on_cleanup: false,
            remove_logs_on_cleanup: false,
            pipes_to_null: false,
            id: None,
        }
    }

    pub fn argument_modifier(mut self, argument_modifier: impl ArgumentModifier + 'static) -> Self {
        self.argument_modifier_chain.push(Box::new(argument_modifier));
        self
    }

    pub fn argument_modifiers(
        mut self,
        argument_modifiers: impl IntoIterator<Item = Box<dyn ArgumentModifier>>,
    ) -> Self {
        self.argument_modifier_chain.extend(argument_modifiers);
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

    pub fn pipes_to_null(mut self) -> Self {
        self.pipes_to_null = true;
        self
    }

    pub fn id(mut self, id: VmmId) -> Self {
        self.id = Some(id);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct VmmId(String);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum VmmIdParseError {
    TooShort,
    TooLong,
    ContainsInvalidCharacter,
}

impl VmmId {
    pub fn new(id: impl Into<String>) -> Result<VmmId, VmmIdParseError> {
        let id = id.into();

        if id.len() < 5 {
            return Err(VmmIdParseError::TooShort);
        }

        if id.len() > 60 {
            return Err(VmmIdParseError::TooLong);
        }

        if id.chars().any(|c| !c.is_ascii_alphanumeric() && c != '-') {
            return Err(VmmIdParseError::ContainsInvalidCharacter);
        }

        Ok(Self(id))
    }
}

impl AsRef<str> for VmmId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl From<VmmId> for String {
    fn from(value: VmmId) -> Self {
        value.0
    }
}

#[cfg(test)]
mod tests {
    use crate::executor::unrestricted::{VmmId, VmmIdParseError};

    #[test]
    fn vmm_id_rejects_when_too_short() {
        for l in 0..5 {
            let str = (0..l).map(|_| "l").collect::<String>();
            assert_eq!(VmmId::new(str), Err(VmmIdParseError::TooShort));
        }
    }

    #[test]
    fn vmm_id_rejects_when_too_long() {
        for l in 61..100 {
            let str = (0..l).map(|_| "L").collect::<String>();
            assert_eq!(VmmId::new(str), Err(VmmIdParseError::TooLong));
        }
    }

    #[test]
    fn vmm_id_rejects_when_invalid_character() {
        for c in ['~', '_', '$', '#', '+'] {
            let str = (0..10).map(|_| c).collect::<String>();
            assert_eq!(VmmId::new(str), Err(VmmIdParseError::ContainsInvalidCharacter));
        }
    }

    #[test]
    fn vmm_id_accepts_valid() {
        for str in ["vmm-id", "longer-id", "L1Nda74-", "very-loNg-ID"] {
            VmmId::new(str).unwrap();
        }
    }
}

impl VmmExecutor for UnrestrictedVmmExecutor {
    fn get_socket_path(&self, _installation: &VmmInstallation) -> Option<PathBuf> {
        match &self.vmm_arguments.api_socket {
            VmmApiSocket::Disabled => None,
            VmmApiSocket::Enabled(path) => Some(path.clone()),
        }
    }

    fn inner_to_outer_path(&self, _installation: &VmmInstallation, inner_path: &Path) -> PathBuf {
        inner_path.to_owned()
    }

    fn traceless(&self) -> bool {
        false
    }

    async fn prepare(
        &self,
        _installation: &VmmInstallation,
        runner: Arc<impl Runner>,
        fs_backend: Arc<impl FsBackend>,
        outer_paths: Vec<PathBuf>,
    ) -> Result<HashMap<PathBuf, PathBuf>, VmmExecutorError> {
        let mut join_set = JoinSet::new();

        for path in outer_paths.clone() {
            let fs_backend = fs_backend.clone();
            let runner = runner.clone();
            join_set.spawn(async move {
                if !fs_backend
                    .check_exists(&path)
                    .await
                    .map_err(VmmExecutorError::FsBackendError)?
                {
                    return Err(VmmExecutorError::ExpectedResourceMissing(path));
                }

                force_chown(&path, runner.as_ref()).await
            });
        }

        if let VmmApiSocket::Enabled(socket_path) = self.vmm_arguments.api_socket.clone() {
            let fs_backend = fs_backend.clone();
            let runner = runner.clone();
            join_set.spawn(async move {
                if fs_backend
                    .check_exists(&socket_path)
                    .await
                    .map_err(VmmExecutorError::FsBackendError)?
                {
                    force_chown(&socket_path, runner.as_ref()).await?;
                    fs_backend
                        .remove_file(&socket_path)
                        .await
                        .map_err(VmmExecutorError::FsBackendError)?;
                }

                Ok(())
            });
        }

        // Ensure argument paths exist
        if let Some(ref log_path) = self.vmm_arguments.log_path {
            join_set.spawn(create_file_with_tree(fs_backend.clone(), log_path.clone()));
        }
        if let Some(ref metrics_path) = self.vmm_arguments.metrics_path {
            join_set.spawn(create_file_with_tree(fs_backend.clone(), metrics_path.clone()));
        }

        join_on_set(join_set).await?;
        Ok(outer_paths.into_iter().map(|path| (path.clone(), path)).collect())
    }

    async fn invoke(
        &self,
        installation: &VmmInstallation,
        runner: Arc<impl Runner>,
        config_override: ConfigurationFileOverride,
    ) -> Result<Child, VmmExecutorError> {
        let mut args = self.vmm_arguments.join(config_override);
        apply_argument_modifier_chain(&mut args, &self.argument_modifier_chain);
        if let Some(ref id) = self.id {
            args.push("--id".to_string());
            args.push(id.as_ref().to_owned());
        }

        let child = runner
            .run(&installation.firecracker_path, args, self.pipes_to_null)
            .await
            .map_err(VmmExecutorError::RunFailed)?;
        Ok(child)
    }

    async fn cleanup(
        &self,
        _installation: &VmmInstallation,
        runner: Arc<impl Runner>,
        fs_backend: Arc<impl FsBackend>,
    ) -> Result<(), VmmExecutorError> {
        let mut join_set: JoinSet<Result<(), VmmExecutorError>> = JoinSet::new();

        if let VmmApiSocket::Enabled(socket_path) = self.vmm_arguments.api_socket.clone() {
            let runner = runner.clone();
            let fs_backend = fs_backend.clone();
            join_set.spawn(async move {
                if fs_backend
                    .check_exists(&socket_path)
                    .await
                    .map_err(VmmExecutorError::FsBackendError)?
                {
                    force_chown(&socket_path, runner.as_ref()).await?;
                    fs_backend
                        .remove_file(&socket_path)
                        .await
                        .map_err(VmmExecutorError::FsBackendError)?;
                }
                Ok(())
            });
        }

        if self.remove_logs_on_cleanup {
            if let Some(ref log_path) = self.vmm_arguments.log_path {
                let fs_backend = fs_backend.clone();
                let log_path = log_path.clone();
                join_set.spawn(async move {
                    fs_backend
                        .remove_file(&log_path)
                        .await
                        .map_err(VmmExecutorError::FsBackendError)
                });
            }
        }

        if self.remove_metrics_on_cleanup {
            if let Some(ref metrics_path) = self.vmm_arguments.metrics_path {
                let fs_backend = fs_backend.clone();
                let metrics_path = metrics_path.clone();
                join_set.spawn(async move {
                    fs_backend
                        .remove_file(&metrics_path)
                        .await
                        .map_err(VmmExecutorError::FsBackendError)
                });
            }
        }

        join_on_set(join_set).await
    }
}
