use std::{
    future::Future,
    path::{Path, PathBuf},
};

use crate::{
    process_spawner::ProcessSpawner,
    runtime::Runtime,
    vmm::ownership::{downgrade_owner, upgrade_owner, VmmOwnershipModel},
};

use super::{
    moved::{MovedVmmResource, VmmResourceMoveMethod},
    VmmResourceError,
};

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
        runtime: R,
    ) -> impl Future<Output = Result<(), VmmResourceError>> + Send {
        let path = effective_path.clone();
        self.effective_path = Some(effective_path);

        async move {
            if let Some(parent_path) = path.parent() {
                runtime
                    .fs_create_dir_all(&parent_path)
                    .await
                    .map_err(VmmResourceError::FilesystemError)?;

                downgrade_owner(&parent_path, ownership_model).map_err(VmmResourceError::ChangeOwnerError)?;
            }

            Ok(())
        }
    }

    /// A shorthand to initialize the resource with the same effective path as its local path.
    pub fn initialize_with_same_path<R: Runtime>(
        &mut self,
        ownership_model: VmmOwnershipModel,
        runtime: R,
    ) -> impl Future<Output = Result<(), VmmResourceError>> + Send {
        self.initialize(self.local_path.clone(), ownership_model, runtime)
    }

    /// Dispose of the resource unless it has been unlinked, according to the given [VmmOwnershipModel]
    /// and [ProcessSpawner]. The returned future doesn't depend on &self and can be spawned on the
    /// [Runtime] to be persisted to the filesystem.
    pub fn dispose<S: ProcessSpawner, R: Runtime>(
        &self,
        ownership_model: VmmOwnershipModel,
        process_spawner: S,
        runtime: R,
    ) -> impl Future<Output = Result<(), VmmResourceError>> + Send {
        let linked = self.linked;
        let path = self.effective_path().to_owned();

        async move {
            if linked {
                upgrade_owner(&path, ownership_model, &process_spawner, &runtime)
                    .await
                    .map_err(VmmResourceError::ChangeOwnerError)?;

                runtime
                    .fs_remove_file(&path)
                    .await
                    .map_err(VmmResourceError::FilesystemError)?;
            }

            Ok(())
        }
    }

    /// Unlink and copy the resource to the given new effective path. The local path remains unchanged.
    pub async fn copy<R: Runtime>(
        &mut self,
        new_effective_path: impl Into<PathBuf>,
        runtime: &R,
    ) -> Result<(), std::io::Error> {
        let new_effective_path = new_effective_path.into();
        runtime.fs_copy(self.effective_path(), &new_effective_path).await?;
        self.effective_path = Some(new_effective_path);
        self.unlink();
        Ok(())
    }

    /// Unlink and move/rename the resource to the given new effective path. The local path remains unchanged.
    pub async fn rename<R: Runtime>(
        &mut self,
        new_effective_path: impl Into<PathBuf>,
        runtime: &R,
    ) -> Result<(), std::io::Error> {
        let new_effective_path = new_effective_path.into();
        runtime.fs_rename(self.effective_path(), &new_effective_path).await?;
        self.effective_path = Some(new_effective_path);
        self.unlink();
        Ok(())
    }

    /// Attempt to delete the resource, giving back ownership alongside the error if the operation fails.
    pub async fn delete<R: Runtime>(self, runtime: &R) -> Result<(), (std::io::Error, Self)> {
        if let Err(err) = runtime.fs_remove_file(self.effective_path()).await {
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
