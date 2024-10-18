use std::{
    path::{Path, PathBuf},
    process::{ExitStatus, Output, Stdio},
};

use tokio::process::Command;

use crate::vmm::installation::VmmInstallation;

/// An extension that provides bindings to functionality exposed by Firecracker's "snapshot-editor" binary.
/// Internally this performs sanity checks and then spawns and awaits a "snapshot-editor" process.
pub trait SnapshotEditorExt {
    /// Get a SnapshotEditor bindings struct that is bound to this installation's lifetime.
    fn snapshot_editor(&self) -> SnapshotEditor<'_>;
}

impl SnapshotEditorExt for VmmInstallation {
    fn snapshot_editor(&self) -> SnapshotEditor<'_> {
        SnapshotEditor {
            path: &self.snapshot_editor_path,
        }
    }
}

/// A struct exposing bindings to a "snapshot-editor" binary of this installation.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SnapshotEditor<'p> {
    path: &'p PathBuf,
}

/// An error that can be emitted by a "snapshot-editor" invocation.
#[derive(Debug, thiserror::Error)]
pub enum SnapshotEditorError {
    #[error("Forking the snapshot-editor process failed: `{0}`")]
    ProcessSpawnFailed(tokio::io::Error),
    #[error("Waiting on the exit of the snapshot-editor process failed: `{0}`")]
    ProcessWaitFailed(tokio::io::Error),
    #[error("The snapshot-editor exited with a non-zero exit status: `{0}`")]
    ExitedWithNonZeroStatus(ExitStatus),
    #[error("A given path was not in UTF-8. Non-UTF-8 paths are unsupported.")]
    NonUTF8Path,
}

impl<'p> SnapshotEditor<'p> {
    /// Rebase base_memory_path onto diff_memory_path.
    pub async fn rebase_memory(
        &self,
        base_memory_path: impl AsRef<Path> + Send,
        diff_memory_path: impl AsRef<Path> + Send,
    ) -> Result<(), SnapshotEditorError> {
        self.run(&[
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

    /// Get the version of a given snapshot.
    pub async fn get_snapshot_version(
        &self,
        snapshot_path: impl AsRef<Path> + Send,
    ) -> Result<String, SnapshotEditorError> {
        let output = self
            .run(&[
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

    /// Get dbg!-produced vCPU states of a given snapshot. The dbg! format is difficult to parse,
    /// so the merit of invoking this programmatically is limited.
    pub async fn get_snapshot_vcpu_states(
        &self,
        snapshot_path: impl AsRef<Path> + Send,
    ) -> Result<String, SnapshotEditorError> {
        let output = self
            .run(&[
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

    /// Get a dbg!-produced full dump of a VM's state. The dbg! format is difficult to parse,
    /// so the merit of invoking this programmatically is limited.
    pub async fn get_snapshot_vm_state(
        &self,
        snapshot_path: impl AsRef<Path> + Send,
    ) -> Result<String, SnapshotEditorError> {
        let output = self
            .run(&[
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

    async fn run(&self, args: &[&str]) -> Result<Output, SnapshotEditorError> {
        let mut command = Command::new(self.path);
        command.args(args);
        command.stdout(Stdio::piped());
        command.stderr(Stdio::null());
        command.stdin(Stdio::null());

        let child = command.spawn().map_err(SnapshotEditorError::ProcessSpawnFailed)?;
        let output = child
            .wait_with_output()
            .await
            .map_err(SnapshotEditorError::ProcessWaitFailed)?;

        if !output.status.success() {
            return Err(SnapshotEditorError::ExitedWithNonZeroStatus(output.status));
        }

        Ok(output)
    }
}
