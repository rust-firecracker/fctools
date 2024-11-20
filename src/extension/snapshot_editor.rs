use std::{
    marker::PhantomData,
    path::{Path, PathBuf},
    process::{Command, ExitStatus, Output, Stdio},
};

use crate::{
    runtime::{Runtime, RuntimeProcess},
    vmm::installation::VmmInstallation,
};

/// An extension that provides bindings to functionality exposed by Firecracker's "snapshot-editor" binary.
/// Internally this performs sanity checks and then spawns and awaits a "snapshot-editor" process.
pub trait SnapshotEditorExt {
    /// Get a [SnapshotEditor] binding that is bound to this [VmmInstallation]'s lifetime.
    fn snapshot_editor<R: Runtime>(&self) -> SnapshotEditor<'_, R>;
}

impl SnapshotEditorExt for VmmInstallation {
    fn snapshot_editor<R: Runtime>(&self) -> SnapshotEditor<'_, R> {
        SnapshotEditor {
            path: &self.snapshot_editor_path,
            runtime: PhantomData,
        }
    }
}

/// A struct exposing bindings to a "snapshot-editor" binary of this [VmmInstallation].
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SnapshotEditor<'p, R: Runtime> {
    path: &'p PathBuf,
    runtime: PhantomData<R>,
}

/// An error that can be emitted by a "snapshot-editor" invocation.
#[derive(Debug)]
pub enum SnapshotEditorError {
    ProcessSpawnFailed(std::io::Error),
    ProcessWaitFailed(std::io::Error),
    ExitedWithNonZeroStatus(ExitStatus),
    NonUTF8Path,
}

impl std::error::Error for SnapshotEditorError {}

impl std::fmt::Display for SnapshotEditorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SnapshotEditorError::ProcessSpawnFailed(err) => {
                write!(f, "Spawning the snapshot-editor-process failed: {err}")
            }
            SnapshotEditorError::ProcessWaitFailed(err) => {
                write!(f, "Waiting on the exit of the snapshot-editor process failed: {err}")
            }
            SnapshotEditorError::ExitedWithNonZeroStatus(exit_status) => write!(
                f,
                "The snapshot-editor process exited with a non-zero exit status: {exit_status}"
            ),
            SnapshotEditorError::NonUTF8Path => write!(f, "A given path was non-UTF-8, which is unsupported"),
        }
    }
}

impl<'p, R: Runtime> SnapshotEditor<'p, R> {
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

        let output = R::Process::output(command)
            .await
            .map_err(SnapshotEditorError::ProcessSpawnFailed)?;

        if !output.status.success() {
            return Err(SnapshotEditorError::ExitedWithNonZeroStatus(output.status));
        }

        Ok(output)
    }
}
