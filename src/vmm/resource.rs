use std::path::{Path, PathBuf};

use nix::sys::stat::Mode;

use crate::runtime::{Runtime, RuntimeFilesystem};

pub struct CreatedVmmResource {
    effective_path: Option<PathBuf>,
    local_path: PathBuf,
    creation_strategy: VmmResourceCreationStrategy,
}

impl CreatedVmmResource {
    pub fn new(path: impl Into<PathBuf>, creation_strategy: VmmResourceCreationStrategy) -> Self {
        Self {
            effective_path: None,
            local_path: path.into(),
            creation_strategy,
        }
    }

    pub async fn apply<R: Runtime>(&mut self, effective_path: PathBuf) -> Result<(), std::io::Error> {
        match self.creation_strategy {
            VmmResourceCreationStrategy::File => {
                R::Filesystem::create_file(&effective_path).await?;
            }
            VmmResourceCreationStrategy::Fifo => {
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
        self.effective_path.as_deref().unwrap()
    }

    pub fn creation_strategy(&self) -> VmmResourceCreationStrategy {
        self.creation_strategy
    }
}

pub struct ExistingVmmResource {
    source_path: PathBuf,
    effective_path: Option<PathBuf>,
    local_path: Option<PathBuf>,
}

impl ExistingVmmResource {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            source_path: path.into(),
            effective_path: None,
            local_path: None,
        }
    }

    pub fn apply(&mut self, effective_path: PathBuf, local_path: PathBuf) {
        self.effective_path = Some(effective_path);
        self.local_path = Some(local_path);
    }

    pub fn source_path(&self) -> &Path {
        self.source_path.as_path()
    }

    pub fn effective_path_checked(&self) -> Option<&Path> {
        self.effective_path.as_deref()
    }

    pub fn effective_path(&self) -> &Path {
        self.effective_path.as_deref().unwrap()
    }

    pub fn local_path_checked(&self) -> Option<&Path> {
        self.local_path.as_deref()
    }

    pub fn local_path(&self) -> &Path {
        self.local_path.as_deref().unwrap()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VmmResourceCreationStrategy {
    File,
    Fifo,
}
