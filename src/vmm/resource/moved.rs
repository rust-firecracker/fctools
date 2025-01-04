use std::{
    future::Future,
    path::{Path, PathBuf},
};

use crate::{
    process_spawner::ProcessSpawner,
    runtime::Runtime,
    vmm::ownership::{upgrade_owner, VmmOwnershipModel},
};

use super::VmmResourceError;

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
    pub fn initialize<S: ProcessSpawner, R: Runtime>(
        &mut self,
        effective_path: PathBuf,
        local_path: PathBuf,
        ownership_model: VmmOwnershipModel,
        process_spawner: S,
        runtime: R,
    ) -> impl Future<Output = Result<(), VmmResourceError>> + Send {
        self.effective_path = Some(effective_path.clone());
        self.local_path = Some(local_path);

        let source_path = self.source_path.clone();
        let move_method = self.move_method;

        async move {
            if effective_path == source_path {
                return Ok(());
            }

            upgrade_owner(&source_path, ownership_model, &process_spawner, &runtime)
                .await
                .map_err(VmmResourceError::ChangeOwnerError)?;

            if !runtime
                .fs_exists(&source_path)
                .await
                .map_err(VmmResourceError::FilesystemError)?
            {
                return Err(VmmResourceError::SourcePathMissing(source_path));
            }

            if let Some(parent_path) = effective_path.parent() {
                runtime
                    .fs_create_dir_all(parent_path)
                    .await
                    .map_err(VmmResourceError::FilesystemError)?;
            }

            match move_method {
                VmmResourceMoveMethod::Copy => {
                    runtime
                        .fs_copy(&source_path, &effective_path)
                        .await
                        .map_err(VmmResourceError::FilesystemError)?;
                }
                VmmResourceMoveMethod::HardLink => {
                    runtime
                        .fs_hard_link(&source_path, &effective_path)
                        .await
                        .map_err(VmmResourceError::FilesystemError)?;
                }
                VmmResourceMoveMethod::CopyOrHardLink => {
                    if runtime.fs_copy(&source_path, &effective_path).await.is_err() {
                        runtime
                            .fs_hard_link(&source_path, &effective_path)
                            .await
                            .map_err(VmmResourceError::FilesystemError)?;
                    }
                }
                VmmResourceMoveMethod::HardLinkOrCopy => {
                    if runtime.fs_hard_link(&source_path, &effective_path).await.is_err() {
                        runtime
                            .fs_copy(&source_path, &effective_path)
                            .await
                            .map_err(VmmResourceError::FilesystemError)?;
                    }
                }
                VmmResourceMoveMethod::Rename => {
                    runtime
                        .fs_rename(&source_path, &effective_path)
                        .await
                        .map_err(VmmResourceError::FilesystemError)?;
                }
            };

            Ok(())
        }
    }

    /// A shorthand to initialize the resource with local and effective paths being equal to the source path, i.e. with the
    /// underlying file being unmoved.
    pub fn initialize_with_same_path<S: ProcessSpawner, R: Runtime>(
        &mut self,
        ownership_model: VmmOwnershipModel,
        process_spawner: S,
        runtime: R,
    ) -> impl Future<Output = Result<(), VmmResourceError>> + Send {
        self.effective_path = Some(self.source_path.clone());
        self.local_path = Some(self.source_path.clone());

        let source_path = self.source_path.clone();
        async move {
            upgrade_owner(&source_path, ownership_model, &process_spawner, &runtime)
                .await
                .map_err(VmmResourceError::ChangeOwnerError)?;

            if !runtime
                .fs_exists(&source_path)
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
