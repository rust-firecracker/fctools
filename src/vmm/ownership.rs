use std::{
    os::unix::process::ExitStatusExt,
    path::{Path, PathBuf},
    process::ExitStatus,
    sync::LazyLock,
};

use crate::{
    process_spawner::ProcessSpawner,
    runtime::{Runtime, RuntimeChild},
    syscall::SyscallBackend,
};

pub(crate) static PROCESS_UID: LazyLock<u32> = LazyLock::new(|| (SyscallBackend::get().geteuid)());
pub(crate) static PROCESS_GID: LazyLock<u32> = LazyLock::new(|| (SyscallBackend::get().getegid)());

/// The model used for managing the ownership of resources between the controlling process
/// (the Rust application using fctools) and the VMM process ("firecracker").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VmmOwnershipModel {
    /// The resources are fully shared between control and VMM processes. Either both run
    /// as root or both are run rootlessly. The latter scenario is incompatible with jailing,
    /// while the former supports jailing only when the jailer doesn't drop permissions of
    /// the VMM process from root.
    Shared,
    /// The control process is rootless and upgrades resources to root where the VMM process
    /// runs. Compatible with jailing only when it doesn't drop permissions of the VMM process.
    UpgradedPermanently,
    /// The control process is rootless, the jailer is upgraded and run as root, after which
    /// permissions of the VMM process are dropped down to those of the control process and
    /// the upgrade is "reverted" as if ownership was shared originally.
    UpgradedTemporarily,
    /// The control process runs as root and so does the jailer, but the jailer drops
    /// permissions of the VMM process to rootless so resources of the control process need to
    /// be made accessible to the VMM process.
    Downgraded {
        /// The UID of the VMM process.
        uid: u32,
        /// The GID of the VMM process.
        gid: u32,
    },
}

impl VmmOwnershipModel {
    #[inline]
    pub(crate) fn as_downgrade(&self) -> Option<(u32, u32)> {
        match self {
            VmmOwnershipModel::UpgradedTemporarily => Some((*PROCESS_UID, *PROCESS_GID)),
            VmmOwnershipModel::Downgraded { uid, gid } => Some((*uid, *gid)),
            _ => None,
        }
    }

    #[inline]
    fn is_upgrade(&self) -> bool {
        matches!(
            self,
            VmmOwnershipModel::UpgradedTemporarily | VmmOwnershipModel::UpgradedPermanently
        )
    }
}

/// An error that can occur when changing the owner to accommodate for [VmmOwnershipModel]s other
/// than the shared model.
#[derive(Debug)]
pub enum ChangeOwnerError {
    ProcessSpawnFailed(std::io::Error),
    ProcessWaitFailed(std::io::Error),
    ProcessExitedWithWrongStatus(ExitStatus),
    RecursiveChownError(std::io::Error),
    FlatChownError(std::io::Error),
}

impl std::error::Error for ChangeOwnerError {}

impl std::fmt::Display for ChangeOwnerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ChangeOwnerError::ProcessSpawnFailed(err) => write!(f, "Spawning a chown process failed: {err}"),
            ChangeOwnerError::ProcessWaitFailed(err) => {
                write!(f, "Waiting on the completion of a chown process failed: {err}")
            }
            ChangeOwnerError::ProcessExitedWithWrongStatus(exit_status) => {
                write!(f, "The chown process exited with a non-zero exit status: {exit_status}")
            }
            ChangeOwnerError::RecursiveChownError(err) => {
                write!(f, "An recursive in-process chown failed: {err}")
            }
            ChangeOwnerError::FlatChownError(err) => write!(f, "A flat in-process chown failed: {err}"),
        }
    }
}

/// For implementors of custom executors: upgrades the owner of the given [Path] using the given [ProcessSpawner]
/// and [Runtime], if the [VmmOwnershipModel] requires the upgrade (otherwise, no-ops). This spawns an elevated
/// coreutils "chown" process via the [ProcessSpawner] and waits on it internally.
pub async fn upgrade_owner<R: Runtime, S: ProcessSpawner>(
    path: &Path,
    ownership_model: VmmOwnershipModel,
    process_spawner: &S,
    runtime: &R,
) -> Result<(), ChangeOwnerError> {
    if ownership_model.is_upgrade() {
        let mut process = process_spawner
            .spawn(
                &PathBuf::from("chown"),
                vec![
                    "-f".to_string(),
                    "-R".to_string(),
                    format!("{}:{}", *PROCESS_UID, *PROCESS_GID),
                    path.to_string_lossy().into_owned(),
                ],
                false,
                runtime,
            )
            .await
            .map_err(ChangeOwnerError::ProcessSpawnFailed)?;
        let exit_status = process.wait().await.map_err(ChangeOwnerError::ProcessWaitFailed)?;

        // code 256 means that a concurrent chown is being called and the chown will still be applied, so this error can
        // "safely" be ignored, which is better than inducing the overhead of global locking on chown paths.
        if !exit_status.success() && exit_status.into_raw() != 256 {
            return Err(ChangeOwnerError::ProcessExitedWithWrongStatus(exit_status));
        }
    }

    Ok(())
}

/// For implementors of custom executors: downgrades the owner of the given [Path] recursively using the
/// given [Runtime]'s recursive implementation, if the [VmmOwnershipModel] requires the downgrade (otherwise, no-ops).
pub async fn downgrade_owner_recursively<R: Runtime>(
    path: &Path,
    ownership_model: VmmOwnershipModel,
    runtime: &R,
) -> Result<(), ChangeOwnerError> {
    if let Some((uid, gid)) = ownership_model.as_downgrade() {
        runtime
            .fs_chown_all(path, uid, gid)
            .await
            .map_err(ChangeOwnerError::RecursiveChownError)
    } else {
        Ok(())
    }
}

/// For implementors of custom executors: downgrades the owner of a given [Path], which should be a single
/// flat file or directory, by invoking chown once if the [VmmOwnershipModel] requires the downgrade (otherwise,
/// no-ops).
pub fn downgrade_owner(path: &Path, ownership_model: VmmOwnershipModel) -> Result<(), ChangeOwnerError> {
    if let Some((uid, gid)) = ownership_model.as_downgrade() {
        (SyscallBackend::get().chown)(path, uid, gid).map_err(ChangeOwnerError::FlatChownError)
    } else {
        Ok(())
    }
}
