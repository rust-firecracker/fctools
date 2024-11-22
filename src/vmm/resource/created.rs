use std::path::{Path, PathBuf};

use nix::sys::stat::Mode;

use crate::{
    runtime::{Runtime, RuntimeFilesystem},
    vmm::ownership::{downgrade_owner, VmmOwnershipModel},
};

use super::VmmResourceError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreatedVmmResource {
    effective_path: Option<PathBuf>,
    local_path: PathBuf,
    r#type: VmmResourceType,
}

impl CreatedVmmResource {
    pub fn new(path: impl Into<PathBuf>, r#type: VmmResourceType) -> Self {
        Self {
            effective_path: None,
            local_path: path.into(),
            r#type,
        }
    }

    pub async fn apply<R: Runtime>(
        &mut self,
        effective_path: PathBuf,
        ownership_model: VmmOwnershipModel,
    ) -> Result<(), VmmResourceError> {
        if let Some(parent_path) = effective_path.parent() {
            R::Filesystem::create_dir_all(&parent_path)
                .await
                .map_err(VmmResourceError::FilesystemError)?;
        }

        match self.r#type {
            VmmResourceType::File => {
                R::Filesystem::create_file(&effective_path)
                    .await
                    .map_err(VmmResourceError::FilesystemError)?;
            }
            VmmResourceType::Fifo => {
                if nix::unistd::mkfifo(
                    &effective_path,
                    Mode::S_IROTH | Mode::S_IWOTH | Mode::S_IRUSR | Mode::S_IWUSR,
                )
                .is_err()
                {
                    return Err(VmmResourceError::MkfifoError(std::io::Error::last_os_error()));
                }
            }
        };

        downgrade_owner(&effective_path, ownership_model).map_err(VmmResourceError::ChangeOwnerError)?;

        self.effective_path = Some(effective_path);
        Ok(())
    }

    pub fn local_path(&self) -> &Path {
        self.local_path.as_path()
    }

    pub fn effective_path_checked(&self) -> Option<&Path> {
        self.effective_path.as_deref()
    }

    pub fn effective_path(&self) -> &Path {
        self.effective_path
            .as_deref()
            .expect("effective_path was None, use effective_path_checked instead")
    }

    pub fn r#type(&self) -> VmmResourceType {
        self.r#type
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VmmResourceType {
    File,
    Fifo,
}
