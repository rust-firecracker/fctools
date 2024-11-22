use std::path::{Path, PathBuf};

use nix::sys::stat::Mode;

use crate::runtime::{Runtime, RuntimeFilesystem};

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

    pub async fn apply<R: Runtime>(&mut self, effective_path: PathBuf) -> Result<(), std::io::Error> {
        match self.r#type {
            VmmResourceType::File => {
                R::Filesystem::create_file(&effective_path).await?;
            }
            VmmResourceType::Fifo => {
                if nix::unistd::mkfifo(
                    &effective_path,
                    Mode::S_IROTH | Mode::S_IWOTH | Mode::S_IRUSR | Mode::S_IWUSR,
                )
                .is_err()
                {
                    return Err(std::io::Error::last_os_error());
                }
            }
        };

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
    ) -> Result<(), std::io::Error> {
        match self.move_method {
            VmmResourceMoveMethod::Copy => {
                R::Filesystem::copy(&self.source_path, &effective_path).await?;
            }
            VmmResourceMoveMethod::HardLink => {
                R::Filesystem::hard_link(&self.source_path, &effective_path).await?;
            }
            VmmResourceMoveMethod::CopyOrHardLink => {
                if R::Filesystem::copy(&self.source_path, &effective_path).await.is_err() {
                    R::Filesystem::hard_link(&self.source_path, &effective_path).await?;
                }
            }
            VmmResourceMoveMethod::HardLinkOrCopy => {
                if R::Filesystem::hard_link(&self.source_path, &effective_path)
                    .await
                    .is_err()
                {
                    R::Filesystem::copy(&self.source_path, &effective_path).await?;
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
pub enum VmmResourceType {
    File,
    Fifo,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VmmResourceMoveMethod {
    Copy,
    HardLink,
    CopyOrHardLink,
    HardLinkOrCopy,
}
