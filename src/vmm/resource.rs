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

/// An error that can be produced by an operation on a VMM resource.
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

/// A set of mutable references to VMM resources of all three types. Through these references, the resources
/// should be initialized by a VMM executor.
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

/// A VMM resource that is created by the control process for the VMM to use, for example a log or metrics file.
/// The type of file that the resource should be is defined by the [CreatedVmmResourceType].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreatedVmmResource {
    effective_path: Option<PathBuf>,
    local_path: PathBuf,
    r#type: CreatedVmmResourceType,
}

impl CreatedVmmResource {
    /// Construct an uninitialized created resource of the given local path and [CreatedVmmResourceType].
    pub fn new(path: impl Into<PathBuf>, r#type: CreatedVmmResourceType) -> Self {
        Self {
            effective_path: None,
            local_path: path.into(),
            r#type,
        }
    }

    /// Initialize the created resource to the given effective path using the specified [VmmOwnershipModel]. The
    /// mutation will be performed immediately, and the returned future can be spawned onto a [Runtime] to apply
    /// the changes to the filesystem.
    pub fn initialize<R: Runtime>(
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

    /// A shorthand to initialize to an effective path that is the local path.
    pub fn initialize_with_same_path<R: Runtime>(
        &mut self,
        ownership_model: VmmOwnershipModel,
    ) -> impl Future<Output = Result<(), VmmResourceError>> + Send {
        self.initialize::<R>(self.local_path.clone(), ownership_model)
    }

    /// Dispose of the resource by deleting it according to ownership constraints via [VmmOwnershipModel] and
    /// [ProcessSpawner].
    pub fn dispose<R: Runtime>(
        &self,
        ownership_model: VmmOwnershipModel,
        process_spawner: Arc<impl ProcessSpawner>,
    ) -> impl Future<Output = Result<(), VmmResourceError>> + Send {
        let path = self.effective_path().to_owned();

        async move {
            upgrade_owner::<R>(&path, ownership_model, process_spawner.as_ref())
                .await
                .map_err(VmmResourceError::ChangeOwnerError)?;

            R::Filesystem::remove_file(&path)
                .await
                .map_err(VmmResourceError::FilesystemError)
        }
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

/// The type of file that a [CreatedVmmResource] is backed by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CreatedVmmResourceType {
    /// A plain-text file created normally.
    File,
    /// A FIFO named pipe created using mkfifo.
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

/// A VMM resource that represents an already-existing file accessible by the control process that is
/// moved for use by the VMM. The filesystem method to move the file is defined by the
/// [VmmResourceMoveMethod]. A kernel image, an initrd and a block device for a VM are all examples of
/// moved resources.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MovedVmmResource {
    source_path: PathBuf,
    effective_path: Option<PathBuf>,
    local_path: Option<PathBuf>,
    move_method: VmmResourceMoveMethod,
}

impl MovedVmmResource {
    /// Construct an uninitialized moved resource with the given source path and [VmmResourceMoveMethod].
    pub fn new(path: impl Into<PathBuf>, move_method: VmmResourceMoveMethod) -> Self {
        Self {
            source_path: path.into(),
            effective_path: None,
            local_path: None,
            move_method,
        }
    }

    /// Initialize the resource at the given effective and local paths according to the ownership constraints
    /// defined by [VmmOwnershipModel] and [ProcessSpawner]. The mutation will be performed immediately, and
    /// the returned futrue can be spawned onto the [Runtime] to apply the changes to the filesystem.
    pub fn initialize<R: Runtime>(
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

    /// A shorthand to initialize the resource with local and effective paths being equal to the source path, i.e. with the
    /// underlying file being unmoved.
    pub fn initialize_with_same_path<R: Runtime>(
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

/// A set of methods of moving [MovedVmmResource] paths.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VmmResourceMoveMethod {
    /// Copy, costly.
    Copy,
    /// Hard link, doesn't work with cross-device links.
    HardLink,
    /// Copy or hard link if copying failed.
    CopyOrHardLink,
    /// Hard link or copy if hard linking failed.
    HardLinkOrCopy,
    /// Fully move by renaming.
    Rename,
}

/// A produced VMM resource represents a file created by the VMM process that is to be used by the
/// control process, for example a snapshot of a VM. By default, a produced resource is linked, meaning
/// that it will be disposed of by the executor upon VMM cleanup. In cases when the resource can be used
/// after its VMM process has exited (a snapshot), the resource can be unlinked and further used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProducedVmmResource {
    local_path: PathBuf,
    effective_path: Option<PathBuf>,
    linked: bool,
}

impl ProducedVmmResource {
    /// Construct an uninitialized produced resource with the given local path.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            local_path: path.into(),
            effective_path: None,
            linked: true,
        }
    }

    /// Unlink the resource for it to be used beyond the scope of its original VMM.
    pub fn unlink(&mut self) {
        self.linked = false;
    }

    /// Initialize the produced resource with the given effective path.
    pub fn initialize<R: Runtime>(
        &mut self,
        effective_path: PathBuf,
        ownership_model: VmmOwnershipModel,
    ) -> impl Future<Output = Result<(), VmmResourceError>> + Send {
        let path = effective_path.clone();
        self.effective_path = Some(effective_path);

        async move {
            if let Some(parent_path) = path.parent() {
                R::Filesystem::create_dir_all(&parent_path)
                    .await
                    .map_err(VmmResourceError::FilesystemError)?;

                downgrade_owner(&parent_path, ownership_model).map_err(VmmResourceError::ChangeOwnerError)?;
            }

            Ok(())
        }
    }

    /// A shorthand to initialize the resource with the same effective path as its local path.
    pub fn initialize_with_same_path(&mut self) {
        self.effective_path = Some(self.local_path.clone());
    }

    /// Dispose of the resource unless it has been unlinked, according to the given [VmmOwnershipModel]
    /// and [ProcessSpawner]. The returned future doesn't depend on &self and can be spawned on the
    /// [Runtime] to be persisted to the filesystem.
    pub fn dispose<R: Runtime>(
        &self,
        ownership_model: VmmOwnershipModel,
        process_spawner: Arc<impl ProcessSpawner>,
    ) -> impl Future<Output = Result<(), VmmResourceError>> + Send {
        let linked = self.linked;
        let path = self.effective_path().to_owned();

        async move {
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
    }

    /// Unlink and copy the resource to the given new effective path. The local path remains unchanged.
    pub async fn copy<R: Runtime>(&mut self, new_effective_path: impl Into<PathBuf>) -> Result<(), std::io::Error> {
        let new_effective_path = new_effective_path.into();
        R::Filesystem::copy(self.effective_path(), &new_effective_path).await?;
        self.effective_path = Some(new_effective_path);
        self.unlink();
        Ok(())
    }

    /// Unlink and move/rename the resource to the given new effective path. The local path remains unchanged.
    pub async fn rename<R: Runtime>(&mut self, new_effective_path: impl Into<PathBuf>) -> Result<(), std::io::Error> {
        let new_effective_path = new_effective_path.into();
        R::Filesystem::rename_file(self.effective_path(), &new_effective_path).await?;
        self.effective_path = Some(new_effective_path);
        self.unlink();
        Ok(())
    }

    /// Attempt to delete the resource, giving back ownership alongside the error if the operation fails.
    pub async fn delete<R: Runtime>(self) -> Result<(), (std::io::Error, Self)> {
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

    pub fn linked(&self) -> bool {
        self.linked
    }

    /// Convert this initialized [ProducedVmmResource] into an uninitialized [MovedVmmResource] with the given
    /// [VmmResourceMoveMethod]. For example: a produced snapshot resource from VM 1 becomes a moved snapshot resource
    /// for VM 2 to use within its configuration.
    pub fn into_moved(self, move_method: VmmResourceMoveMethod) -> MovedVmmResource {
        MovedVmmResource::new(
            self.effective_path
                .expect("effective_path is None when calling into_moved"),
            move_method,
        )
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
