use std::{path::PathBuf, sync::Arc};

use crate::runtime::Runtime;

use super::{
    internal::{ResourceData, ResourceInitData},
    system::ResourceSystemError,
    ResourceState, ResourceType,
};

#[derive(Debug, Clone)]
pub struct ResourceTemplate {
    pub(super) data: ResourceData,
    pub(super) init_data: Option<ResourceInitData>,
}

impl ResourceTemplate {
    pub fn get_type(&self) -> ResourceType {
        self.data.r#type
    }

    pub fn get_source_path(&self) -> PathBuf {
        self.data.source_path.clone()
    }

    pub fn get_effective_path(&self) -> Option<PathBuf> {
        self.init_data
            .as_ref()
            .map(|init_data| init_data.effective_path.clone())
    }

    pub fn get_local_path(&self) -> Option<PathBuf> {
        self.init_data
            .as_ref()
            .and_then(|init_data| init_data.local_path.clone())
    }

    pub fn deinitialize(&mut self, r#type: ResourceType) -> bool {
        let Some(init_data) = self.init_data.take() else {
            return false;
        };

        self.data = ResourceData {
            source_path: init_data.effective_path,
            r#type,
        };
        self.init_data = None;

        return true;
    }

    pub async fn copy<P: Into<PathBuf>, R: Runtime>(
        &mut self,
        runtime: &R,
        new_effective_path: P,
    ) -> Result<(), ResourceSystemError> {
        let Some(effective_path) = self.get_effective_path() else {
            return Err(ResourceSystemError::IncorrectState(ResourceState::Uninitialized));
        };

        let new_effective_path = new_effective_path.into();
        runtime
            .fs_copy(&effective_path, &new_effective_path)
            .await
            .map_err(Arc::new)
            .map_err(ResourceSystemError::FilesystemError)?;
        self.init_data
            .as_mut()
            .expect("Breached static invariant of DetachedResource")
            .effective_path = new_effective_path;
        Ok(())
    }

    pub async fn remove<R: Runtime>(self, runtime: &R) -> Result<(), (Self, ResourceSystemError)> {
        let Some(effective_path) = self.get_effective_path() else {
            return Err((self, ResourceSystemError::IncorrectState(ResourceState::Uninitialized)));
        };

        match runtime.fs_remove_file(&effective_path).await {
            Ok(()) => Ok(()),
            Err(err) => Err((self, ResourceSystemError::FilesystemError(Arc::new(err)))),
        }
    }
}
