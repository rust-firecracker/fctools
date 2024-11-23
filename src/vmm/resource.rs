use std::{
    future::Future,
    path::{Path, PathBuf},
    sync::Arc,
};

use nix::sys::stat::Mode;

use crate::{
    process_spawner::ProcessSpawner,
    runtime::{Runtime, RuntimeFilesystem},
    vmm::ownership::{downgrade_owner, VmmOwnershipModel},
};

use super::ownership::{upgrade_owner, ChangeOwnerError};

#[derive(Debug)]
pub enum VmmResourceError {
    FilesystemError(std::io::Error),
    MkfifoError(std::io::Error),
    ChangeOwnerError(ChangeOwnerError),
    SourcePathMissing(PathBuf),
}

impl std::error::Error for VmmResourceError {}

impl std::fmt::Display for VmmResourceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VmmResourceError::FilesystemError(err) => {
                write!(f, "A filesystem operation backed by the runtime failed: {err}")
            }
            VmmResourceError::MkfifoError(err) => write!(f, "Creating a named pipe via mkfifo failed: {err}"),
            VmmResourceError::ChangeOwnerError(err) => write!(f, "An ownership change failed: {err}"),
            VmmResourceError::SourcePathMissing(path) => {
                write!(f, "The source path of a resource is missing: {}", path.display())
            }
        }
    }
}

pub struct VmmResourceReferences<'r> {
    pub moved_resources: Vec<&'r mut MovedVmmResource>,
    pub created_resources: Vec<&'r mut CreatedVmmResource>,
    pub produced_resources: Vec<&'r mut ProducedVmmResource>,
}

impl<'r> VmmResourceReferences<'r> {
    pub fn new() -> Self {
        Self {
            moved_resources: Vec::new(),
            created_resources: Vec::new(),
            produced_resources: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreatedVmmResource {
    effective_path: Option<PathBuf>,
    local_path: PathBuf,
    r#type: CreatedVmmResourceType,
    linked: bool,
}

impl CreatedVmmResource {
    pub fn new(path: impl Into<PathBuf>, r#type: CreatedVmmResourceType) -> Self {
        Self {
            effective_path: None,
            local_path: path.into(),
            r#type,
            linked: true,
        }
    }

    pub fn unlink(&mut self) {
        self.linked = false;
    }

    pub fn apply<R: Runtime>(
        &mut self,
        effective_path: PathBuf,
        ownership_model: VmmOwnershipModel,
    ) -> impl Future<Output = Result<(), VmmResourceError>> + Send {
        self.effective_path = Some(effective_path.clone());

        let r#type = self.r#type;
        async move {
            if let Some(parent_path) = effective_path.parent() {
                R::Filesystem::create_dir_all(&parent_path)
                    .await
                    .map_err(VmmResourceError::FilesystemError)?;
            }

            match r#type {
                CreatedVmmResourceType::File => {
                    R::Filesystem::create_file(&effective_path)
                        .await
                        .map_err(VmmResourceError::FilesystemError)?;
                }
                CreatedVmmResourceType::Fifo => {
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

            Ok(())
        }
    }

    pub fn apply_with_same_path<R: Runtime>(
        &mut self,
        ownership_model: VmmOwnershipModel,
    ) -> impl Future<Output = Result<(), VmmResourceError>> + Send {
        self.apply::<R>(self.local_path.clone(), ownership_model)
    }

    pub fn dispose<R: Runtime>(
        &self,
        ownership_model: VmmOwnershipModel,
        process_spawner: Arc<impl ProcessSpawner>,
    ) -> impl Future<Output = Result<(), VmmResourceError>> + Send {
        dispose_future::<R>(
            self.effective_path().to_owned(),
            self.linked,
            ownership_model,
            process_spawner,
        )
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

    pub fn r#type(&self) -> CreatedVmmResourceType {
        self.r#type
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CreatedVmmResourceType {
    File,
    Fifo,
}

#[cfg(feature = "vm")]
#[cfg_attr(docsrs, doc(cfg(feature = "vm")))]
impl serde::Serialize for CreatedVmmResource {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.local_path.serialize(serializer)
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

    pub fn apply<R: Runtime>(
        &mut self,
        effective_path: PathBuf,
        local_path: PathBuf,
        ownership_model: VmmOwnershipModel,
        process_spawner: Arc<impl ProcessSpawner>,
    ) -> impl Future<Output = Result<(), VmmResourceError>> + Send {
        self.effective_path = Some(effective_path.clone());
        self.local_path = Some(local_path);

        let source_path = self.source_path.clone();
        let move_method = self.move_method;

        async move {
            if effective_path == source_path {
                return Ok(());
            }

            upgrade_owner::<R>(&source_path, ownership_model, process_spawner.as_ref())
                .await
                .map_err(VmmResourceError::ChangeOwnerError)?;

            if !R::Filesystem::check_exists(&source_path)
                .await
                .map_err(VmmResourceError::FilesystemError)?
            {
                return Err(VmmResourceError::SourcePathMissing(source_path));
            }

            if let Some(parent_path) = effective_path.parent() {
                R::Filesystem::create_dir_all(parent_path)
                    .await
                    .map_err(VmmResourceError::FilesystemError)?;
            }

            match move_method {
                VmmResourceMoveMethod::Copy => {
                    R::Filesystem::copy(&source_path, &effective_path)
                        .await
                        .map_err(VmmResourceError::FilesystemError)?;
                }
                VmmResourceMoveMethod::HardLink => {
                    R::Filesystem::hard_link(&source_path, &effective_path)
                        .await
                        .map_err(VmmResourceError::FilesystemError)?;
                }
                VmmResourceMoveMethod::CopyOrHardLink => {
                    if R::Filesystem::copy(&source_path, &effective_path).await.is_err() {
                        R::Filesystem::hard_link(&source_path, &effective_path)
                            .await
                            .map_err(VmmResourceError::FilesystemError)?;
                    }
                }
                VmmResourceMoveMethod::HardLinkOrCopy => {
                    if R::Filesystem::hard_link(&source_path, &effective_path).await.is_err() {
                        R::Filesystem::copy(&source_path, &effective_path)
                            .await
                            .map_err(VmmResourceError::FilesystemError)?;
                    }
                }
                VmmResourceMoveMethod::Rename => {
                    R::Filesystem::rename_file(&source_path, &effective_path)
                        .await
                        .map_err(VmmResourceError::FilesystemError)?;
                }
            };

            Ok(())
        }
    }

    pub fn apply_with_same_path<R: Runtime>(
        &mut self,
        ownership_model: VmmOwnershipModel,
        process_spawner: Arc<impl ProcessSpawner>,
    ) -> impl Future<Output = Result<(), VmmResourceError>> + Send {
        self.effective_path = Some(self.source_path.clone());
        self.local_path = Some(self.source_path.clone());

        let source_path = self.source_path.clone();
        async move {
            upgrade_owner::<R>(&source_path, ownership_model, process_spawner.as_ref())
                .await
                .map_err(VmmResourceError::ChangeOwnerError)?;

            if !R::Filesystem::check_exists(&source_path)
                .await
                .map_err(VmmResourceError::FilesystemError)?
            {
                return Err(VmmResourceError::SourcePathMissing(source_path));
            }

            Ok(())
        }
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

#[cfg(feature = "vm")]
#[cfg_attr(docsrs, doc(cfg(feature = "vm")))]
impl serde::Serialize for MovedVmmResource {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.local_path().serialize(serializer)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VmmResourceMoveMethod {
    Copy,
    HardLink,
    CopyOrHardLink,
    HardLinkOrCopy,
    Rename,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProducedVmmResource {
    local_path: PathBuf,
    effective_path: Option<PathBuf>,
    linked: bool,
}

impl ProducedVmmResource {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            local_path: path.into(),
            effective_path: None,
            linked: true,
        }
    }

    pub fn unlink(&mut self) {
        self.linked = false;
    }

    pub fn apply(&mut self, effective_path: PathBuf) {
        self.effective_path = Some(effective_path);
    }

    pub fn apply_with_same_path(&mut self) {
        self.effective_path = Some(self.local_path.clone());
    }

    pub fn dispose<R: Runtime>(
        &self,
        ownership_model: VmmOwnershipModel,
        process_spawner: Arc<impl ProcessSpawner>,
    ) -> impl Future<Output = Result<(), VmmResourceError>> + Send {
        dispose_future::<R>(
            self.effective_path().to_owned(),
            self.linked,
            ownership_model,
            process_spawner,
        )
    }

    pub async fn copy<R: Runtime>(&mut self, new_effective_path: impl Into<PathBuf>) -> Result<(), std::io::Error> {
        let new_effective_path = new_effective_path.into();
        R::Filesystem::copy(self.effective_path(), &new_effective_path).await?;
        self.effective_path = Some(new_effective_path);
        Ok(())
    }

    pub async fn remove<R: Runtime>(self) -> Result<(), (std::io::Error, Self)> {
        if let Err(err) = R::Filesystem::remove_file(self.effective_path()).await {
            return Err((err, self));
        }

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
}

#[cfg(feature = "vm")]
#[cfg_attr(docsrs, doc(cfg(feature = "vm")))]
impl serde::Serialize for ProducedVmmResource {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.local_path.serialize(serializer)
    }
}

async fn dispose_future<R: Runtime>(
    path: PathBuf,
    linked: bool,
    ownership_model: VmmOwnershipModel,
    process_spawner: Arc<impl ProcessSpawner>,
) -> Result<(), VmmResourceError> {
    if linked {
        upgrade_owner::<R>(&path, ownership_model, process_spawner.as_ref())
            .await
            .map_err(VmmResourceError::ChangeOwnerError)?;

        R::Filesystem::remove_file(&path)
            .await
            .map_err(VmmResourceError::FilesystemError)?;
    }

    Ok(())
}
