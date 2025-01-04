use std::{
    future::Future,
    path::{Path, PathBuf},
};

use crate::{
    process_spawner::ProcessSpawner,
    runtime::Runtime,
    vmm::ownership::{downgrade_owner, upgrade_owner, VmmOwnershipModel},
};

use super::VmmResourceError;

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
        runtime: R,
    ) -> impl Future<Output = Result<(), VmmResourceError>> + Send {
        self.effective_path = Some(effective_path.clone());

        let r#type = self.r#type;
        async move {
            if let Some(parent_path) = effective_path.parent() {
                runtime
                    .fs_create_dir_all(&parent_path)
                    .await
                    .map_err(VmmResourceError::FilesystemError)?;
            }

            match r#type {
                CreatedVmmResourceType::File => {
                    runtime
                        .fs_create_file(&effective_path)
                        .await
                        .map_err(VmmResourceError::FilesystemError)?;
                }
                CreatedVmmResourceType::Fifo => {
                    crate::syscall::mkfifo(&effective_path).map_err(VmmResourceError::MkfifoError)?;
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
        runtime: R,
    ) -> impl Future<Output = Result<(), VmmResourceError>> + Send {
        self.initialize(self.local_path.clone(), ownership_model, runtime)
    }

    /// Dispose of the resource by deleting it according to ownership constraints via [VmmOwnershipModel] and
    /// [ProcessSpawner].
    pub fn dispose<S: ProcessSpawner, R: Runtime>(
        &self,
        ownership_model: VmmOwnershipModel,
        process_spawner: S,
        runtime: R,
    ) -> impl Future<Output = Result<(), VmmResourceError>> + Send {
        let path = self.effective_path().to_owned();

        async move {
            upgrade_owner(&path, ownership_model, &process_spawner, &runtime)
                .await
                .map_err(VmmResourceError::ChangeOwnerError)?;

            runtime
                .fs_remove_file(&path)
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
