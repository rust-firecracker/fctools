use std::{
    ffi::OsString,
    path::Path,
    process::{ExitStatus, Output},
};

use crate::{runtime::Runtime, vmm::installation::VmmInstallation};

/// An extension that provides bindings to functionality exposed by Firecracker's "snapshot-editor" binary.
/// Internally this performs sanity checks and then spawns and awaits a "snapshot-editor" process.
pub trait SnapshotEditorExt {
    /// Get a [SnapshotEditor] binding that is bound to this [VmmInstallation]'s lifetime.
    fn snapshot_editor<R: Runtime>(&self, runtime: R) -> SnapshotEditor<'_, R>;
}

impl SnapshotEditorExt for VmmInstallation {
    fn snapshot_editor<R: Runtime>(&self, runtime: R) -> SnapshotEditor<'_, R> {
        SnapshotEditor {
            path: self.get_snapshot_editor_path(),
            runtime,
        }
    }
}

/// A struct exposing bindings to a "snapshot-editor" binary of this [VmmInstallation].
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SnapshotEditor<'p, R: Runtime> {
    path: &'p Path,
    runtime: R,
}

/// An error that can be emitted by a "snapshot-editor" invocation.
#[derive(Debug)]
pub enum SnapshotEditorError {
    /// Running the "snapshot-editor" process yielded an I/O error.
    ProcessRunError(std::io::Error),
    /// The "snapshot-editor" process exited with a non-zero exit status.
    ExitedWithNonZeroStatus(ExitStatus),
    /// The provided paths were not in UTF-8 format. Non-UTF-8 paths are currently
    /// not supported by the extension.
    NonUTF8Path,
}

impl std::error::Error for SnapshotEditorError {}

impl std::fmt::Display for SnapshotEditorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SnapshotEditorError::ProcessRunError(err) => {
                write!(f, "Running the snapshot-editor-process failed: {err}")
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
    pub async fn rebase_memory<P: AsRef<Path> + Send, Q: AsRef<Path> + Send>(
        &self,
        base_memory_path: P,
        diff_memory_path: Q,
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
    pub async fn get_snapshot_version<P: AsRef<Path> + Send>(
        &self,
        snapshot_path: P,
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
    pub async fn get_snapshot_vcpu_states<P: AsRef<Path> + Send>(
        &self,
        snapshot_path: P,
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
    pub async fn get_snapshot_vm_state<P: AsRef<Path> + Send>(
        &self,
        snapshot_path: P,
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
        let output = self
            .runtime
            .run_process(
                self.path.as_os_str(),
                args.iter().map(OsString::from).collect::<Vec<_>>().as_slice(),
                true,
                false,
            )
            .await
            .map_err(SnapshotEditorError::ProcessRunError)?;

        if !output.status.success() {
            return Err(SnapshotEditorError::ExitedWithNonZeroStatus(output.status));
        }

        Ok(output)
    }
}
