use std::{
    os::unix::process::ExitStatusExt,
    path::{Path, PathBuf},
    process::ExitStatus,
    sync::LazyLock,
};

use nix::unistd::{Gid, Uid};

use crate::{
    fs_backend::{FsBackend, FsBackendError},
    process_spawner::ProcessSpawner,
};

pub(crate) static PROCESS_UID: LazyLock<Uid> = LazyLock::new(|| nix::unistd::geteuid());
pub(crate) static PROCESS_GID: LazyLock<Gid> = LazyLock::new(|| nix::unistd::getegid());

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
        uid: Uid,
        /// The GID of the VMM process.
        gid: Gid,
    },
}

impl VmmOwnershipModel {
    #[inline]
    pub(crate) fn as_downgrade(&self) -> Option<(Uid, Gid)> {
        match self {
            VmmOwnershipModel::UpgradedTemporarily => Some((*PROCESS_UID, *PROCESS_GID)),
            VmmOwnershipModel::Downgraded { uid, gid } => Some((*uid, *gid)),
            _ => None,
        }
    }

    #[inline]
    fn is_upgrade(&self) -> bool {
        match self {
            VmmOwnershipModel::UpgradedTemporarily => true,
            VmmOwnershipModel::UpgradedPermanently => true,
            _ => false,
        }
    }
}

/// An error that can occur when changing the owner to accommodate for [VmmOwnershipModel]s other
/// than the shared model.
#[derive(Debug, thiserror::Error)]
pub enum ChangeOwnerError {
    #[error("Spawning a \"chown\" process failed: {0}")]
    ProcessSpawnFailed(std::io::Error),
    #[error("Waiting on the completion of the \"chown\" process failed: {0}")]
    ProcessWaitFailed(std::io::Error),
    #[error("The \"chown\" process exited with a non-zero exit status: {0}")]
    ProcessExitedWithWrongStatus(ExitStatus),
    #[error("An in-process recursive chown implementation in the filesystem backend failed: {0}")]
    FsBackendError(FsBackendError),
}

/// For implementors of custom executors: upgrades the owner of the given [Path] using the given [ProcessSpawner]
/// and [FsBackend], if the [VmmOwnershipModel] requires the upgrade (otherwise, no-ops).
pub async fn upgrade_owner(
    path: &Path,
    ownership_model: VmmOwnershipModel,
    process_spawner: &impl ProcessSpawner,
    fs_backend: &impl FsBackend,
) -> Result<(), ChangeOwnerError> {
    if ownership_model.is_upgrade() {
        change_owner(path, *PROCESS_UID, *PROCESS_GID, true, process_spawner, fs_backend).await
    } else {
        Ok(())
    }
}

/// For implementors of custom executors: downgrades the owner of the given [Path] using the given [ProcessSpawner]
/// and [FsBackend], if the [VmmOwnershipModel] requires the downgrade (otherwise, no-ops).
pub async fn downgrade_owner(
    path: &Path,
    ownership_model: VmmOwnershipModel,
    process_spawner: &impl ProcessSpawner,
    fs_backend: &impl FsBackend,
) -> Result<(), ChangeOwnerError> {
    if let Some((uid, gid)) = ownership_model.as_downgrade() {
        change_owner(path, uid, gid, false, process_spawner, fs_backend).await
    } else {
        Ok(())
    }
}

async fn change_owner(
    path: &Path,
    uid: Uid,
    gid: Gid,
    forced: bool,
    process_spawner: &impl ProcessSpawner,
    fs_backend: &impl FsBackend,
) -> Result<(), ChangeOwnerError> {
    // use "chown" process spawning for forced chowns since they require privilege acquiry that can't be done on the
    // control process
    // otherwise, use an in-process async implementation from the FS backend
    if forced {
        let mut child = process_spawner
            .spawn(
                &PathBuf::from("chown"),
                vec![
                    "-f".to_string(),
                    "-R".to_string(),
                    format!("{uid}:{gid}"),
                    path.to_string_lossy().into_owned(),
                ],
                false,
            )
            .await
            .map_err(ChangeOwnerError::ProcessSpawnFailed)?;
        let exit_status = child.wait().await.map_err(ChangeOwnerError::ProcessWaitFailed)?;

        // code 256 means that a concurrent chown is being called and the chown will still be applied, so this error can
        // "safely" be ignored, which is better than inducing the overhead of global locking on chown paths.
        if !exit_status.success() && exit_status.into_raw() != 256 {
            return Err(ChangeOwnerError::ProcessExitedWithWrongStatus(exit_status));
        }
    } else {
        fs_backend
            .chownr(path, uid, gid)
            .await
            .map_err(ChangeOwnerError::FsBackendError)?;
    }

    Ok(())
}
