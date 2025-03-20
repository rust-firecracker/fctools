use std::path::PathBuf;

use crate::{process_spawner::ProcessSpawner, runtime::Runtime};

use super::{
    internal::{ResourceData, ResourceInitData},
    system::{ResourceSystem, ResourceSystemError},
    Resource, ResourceType,
};

#[derive(Debug, Clone)]
pub struct DetachedResource {
    pub(super) data: ResourceData,
    pub(super) init_data: Option<ResourceInitData>,
}

impl DetachedResource {
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

    pub fn attach<S: ProcessSpawner, R: Runtime>(
        self,
        resource_system: &mut ResourceSystem<S, R>,
    ) -> Result<Resource, (Self, ResourceSystemError)> {
        match resource_system.attach_resource(self.data.clone(), self.init_data.clone()) {
            Ok(resource) => Ok(resource),
            Err(err) => Err((self, err)),
        }
    }
}
