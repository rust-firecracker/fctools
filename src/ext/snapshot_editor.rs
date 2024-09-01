use std::{
    path::{Path, PathBuf},
    process::{ExitStatus, Output, Stdio},
};

use tokio::process::Command;

use crate::executor::installation::VmmInstallation;

pub trait SnapshotEditorExt {
    fn snapshot_editor(&self) -> SnapshotEditor<'_>;
}

impl SnapshotEditorExt for VmmInstallation {
    fn snapshot_editor(&self) -> SnapshotEditor<'_> {
        SnapshotEditor {
            path: &self.snapshot_editor_path,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SnapshotEditor<'a> {
    path: &'a PathBuf,
}

#[derive(Debug, thiserror::Error)]
pub enum SnapshotEditorError {
    #[error("Forking the snapshot-editor process failed: `{0}`")]
    ProcessForkFailed(tokio::io::Error),
    #[error("The snapshot-editor exited with a non-zero exit status: `{0}`")]
    ExitedWithNonZeroStatus(ExitStatus),
    #[error("A given path was not in UTF-8. Non-UTF-8 paths are unsupported.")]
    NonUTF8Path,
}

impl<'a> SnapshotEditor<'a> {
    pub async fn rebase_memory(
        &self,
        base_memory_path: impl AsRef<Path> + Send,
        diff_memory_path: impl AsRef<Path> + Send,
    ) -> Result<(), SnapshotEditorError> {
        self.fork(&[
            "edit-memory",
            "rebase",
            "--memory-path",
            base_memory_path
                .as_ref()
                .to_str()
                .ok_or(SnapshotEditorError::NonUTF8Path)?,
            "--diff-path",
            diff_memory_path
                .as_ref()
                .to_str()
                .ok_or(SnapshotEditorError::NonUTF8Path)?,
        ])
        .await
        .map(|_| ())
    }

    pub async fn get_snapshot_version(
        &self,
        snapshot_path: impl AsRef<Path> + Send,
    ) -> Result<String, SnapshotEditorError> {
        let output = self
            .fork(&[
                "info-vmstate",
                "version",
                "--vmstate-path",
                snapshot_path
                    .as_ref()
                    .to_str()
                    .ok_or(SnapshotEditorError::NonUTF8Path)?,
            ])
            .await?;
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }

    pub async fn get_snapshot_vcpu_states(
        &self,
        snapshot_path: impl AsRef<Path> + Send,
    ) -> Result<String, SnapshotEditorError> {
        let output = self
            .fork(&[
                "info-vmstate",
                "vcpu-states",
                "--vmstate-path",
                snapshot_path
                    .as_ref()
                    .to_str()
                    .ok_or(SnapshotEditorError::NonUTF8Path)?,
            ])
            .await?;
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }

    pub async fn get_snapshot_vm_state(
        &self,
        snapshot_path: impl AsRef<Path> + Send,
    ) -> Result<String, SnapshotEditorError> {
        let output = self
            .fork(&[
                "info-vmstate",
                "vm-state",
                "--vmstate-path",
                snapshot_path
                    .as_ref()
                    .to_str()
                    .ok_or(SnapshotEditorError::NonUTF8Path)?,
            ])
            .await?;
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }

    async fn fork(&self, args: &[&str]) -> Result<Output, SnapshotEditorError> {
        let mut command = Command::new(self.path);
        command.args(args);
        command.stdout(Stdio::piped());
        command.stderr(Stdio::piped());
        command.stdin(Stdio::null());

        let output = command.output().await.map_err(SnapshotEditorError::ProcessForkFailed)?;
        if !output.status.success() {
            return Err(SnapshotEditorError::ExitedWithNonZeroStatus(output.status));
        }

        Ok(output)
    }
}
