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

static PROCESS_UID: LazyLock<Uid> = LazyLock::new(|| nix::unistd::geteuid());
static PROCESS_GID: LazyLock<Gid> = LazyLock::new(|| nix::unistd::getegid());

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum VmmOwnershipModel {
    Shared,
    UpgradedPermanently,
    UpgradedTemporarily,
}

/// An error that can occur when changing the owner to accommodate for ownership upgrades and/or downgrades.
#[derive(Debug, thiserror::Error)]
pub enum ChangeOwnerError {
    #[error("Spawning a \"chown\" process failed: `{0}`")]
    ProcessSpawnFailed(std::io::Error),
    #[error("Waiting on the completion of the \"chown\" process failed: `{0}`")]
    ProcessWaitFailed(std::io::Error),
    #[error("The \"chown\" process exited with a non-zero exit status: `{0}`")]
    ProcessExitedWithWrongStatus(ExitStatus),
    #[error("An in-process recursive chown implementation in the filesystem backend failed: `{0}`")]
    FsBackendError(FsBackendError),
}

pub(crate) async fn change_owner(
    path: &Path,
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
                    format!("{}:{}", *PROCESS_UID, *PROCESS_GID),
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
            .chownr(path, *PROCESS_UID, *PROCESS_GID)
            .await
            .map_err(ChangeOwnerError::FsBackendError)?;
    }

    Ok(())
}
