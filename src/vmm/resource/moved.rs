use std::path::{Path, PathBuf};

use crate::{
    process_spawner::ProcessSpawner,
    runtime::{Runtime, RuntimeFilesystem},
    vmm::ownership::{upgrade_owner, VmmOwnershipModel},
};

use super::VmmResourceError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MovedVmmResource {
    source_path: PathBuf,
    effective_path: Option<PathBuf>,
    local_path: Option<PathBuf>,
    move_method: VmmResourceMoveMethod,
}

impl MovedVmmResource {
    pub fn new(path: impl Into<PathBuf>, move_method: VmmResourceMoveMethod) -> Self {
        Self {
            source_path: path.into(),
            effective_path: None,
            local_path: None,
            move_method,
        }
    }

    pub async fn apply<R: Runtime>(
        &mut self,
        effective_path: PathBuf,
        local_path: PathBuf,
        ownership_model: VmmOwnershipModel,
        process_spawner: &impl ProcessSpawner,
    ) -> Result<(), VmmResourceError> {
        if effective_path == self.source_path {
            return Ok(());
        }

        upgrade_owner::<R>(&self.source_path, ownership_model, process_spawner)
            .await
            .map_err(VmmResourceError::ChangeOwnerError)?;

        if !R::Filesystem::check_exists(&self.source_path)
            .await
            .map_err(VmmResourceError::FilesystemError)?
        {
            return Err(VmmResourceError::SourcePathMissing(self.source_path.clone()));
        }

        if let Some(parent_path) = effective_path.parent() {
            R::Filesystem::create_dir_all(parent_path)
                .await
                .map_err(VmmResourceError::FilesystemError)?;
        }

        match self.move_method {
            VmmResourceMoveMethod::Copy => {
                R::Filesystem::copy(&self.source_path, &effective_path)
                    .await
                    .map_err(VmmResourceError::FilesystemError)?;
            }
            VmmResourceMoveMethod::HardLink => {
                R::Filesystem::hard_link(&self.source_path, &effective_path)
                    .await
                    .map_err(VmmResourceError::FilesystemError)?;
            }
            VmmResourceMoveMethod::CopyOrHardLink => {
                if R::Filesystem::copy(&self.source_path, &effective_path).await.is_err() {
                    R::Filesystem::hard_link(&self.source_path, &effective_path)
                        .await
                        .map_err(VmmResourceError::FilesystemError)?;
                }
            }
            VmmResourceMoveMethod::HardLinkOrCopy => {
                if R::Filesystem::hard_link(&self.source_path, &effective_path)
                    .await
                    .is_err()
                {
                    R::Filesystem::copy(&self.source_path, &effective_path)
                        .await
                        .map_err(VmmResourceError::FilesystemError)?;
                }
            }
        };

        self.effective_path = Some(effective_path);
        self.local_path = Some(local_path);
        Ok(())
    }

    pub fn source_path(&self) -> &Path {
        self.source_path.as_path()
    }

    pub fn effective_path_checked(&self) -> Option<&Path> {
        self.effective_path.as_deref()
    }

    pub fn effective_path(&self) -> &Path {
        self.effective_path
            .as_deref()
            .expect("effective_path was None, use effective_path_checked instead")
    }

    pub fn local_path_checked(&self) -> Option<&Path> {
        self.local_path.as_deref()
    }

    pub fn local_path(&self) -> &Path {
        self.local_path
            .as_deref()
            .expect("local_path was None, use local_path_checked instead")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VmmResourceMoveMethod {
    Copy,
    HardLink,
    CopyOrHardLink,
    HardLinkOrCopy,
}
