use std::{marker::PhantomData, path::PathBuf, sync::Arc};

use crate::{process_spawner::ProcessSpawner, runtime::Runtime, vmm::ownership::VmmOwnershipModel};

use super::{
    bus::{Bus, BusClient},
    internal::{
        resource_system_main_task, InternalResource, InternalResourceData, InternalResourceState,
        ResourceSystemRequest, ResourceSystemResponse,
    },
    CreatedResource, CreatedResourceType, MovedResource, MovedResourceType, ProducedResource, Resource, ResourceState,
    ResourceType,
};

pub struct ResourceSystem<S: ProcessSpawner, R: Runtime, B: Bus> {
    bus_client: B::Client<ResourceSystemRequest<R, B>, ResourceSystemResponse>,
    marker: PhantomData<(S, B)>,
}

impl<S: ProcessSpawner, R: Runtime, B: Bus> ResourceSystem<S, R, B> {
    pub fn new(process_spawner: S, runtime: R, ownership_model: VmmOwnershipModel) -> Self {
        Self::new_inner(Vec::new(), process_spawner, runtime, ownership_model)
    }

    pub fn with_capacity(process_spawner: S, runtime: R, ownership_model: VmmOwnershipModel, capacity: usize) -> Self {
        Self::new_inner(Vec::with_capacity(capacity), process_spawner, runtime, ownership_model)
    }

    #[inline(always)]
    fn new_inner(
        internal_resources: Vec<InternalResource<R, B>>,
        process_spawner: S,
        runtime: R,
        ownership_model: VmmOwnershipModel,
    ) -> Self {
        let (bus_client, bus_server) = B::new();

        runtime.clone().spawn_task(resource_system_main_task::<S, R, B>(
            bus_server,
            internal_resources,
            process_spawner,
            runtime,
            ownership_model,
        ));

        Self {
            bus_client,
            marker: PhantomData,
        }
    }

    pub async fn shutdown(mut self) -> Result<(), ResourceSystemError> {
        match self.bus_client.make_request(ResourceSystemRequest::Shutdown).await {
            Some(ResourceSystemResponse::ShutdownFinished) => Ok(()),
            _ => Err(ResourceSystemError::BusDisconnected),
        }
    }

    pub fn new_moved_resource(
        &mut self,
        source_path: PathBuf,
        r#type: MovedResourceType,
    ) -> Result<MovedResource<B>, ResourceSystemError> {
        self.new_resource(source_path, ResourceType::Moved(r#type))
            .map(MovedResource)
    }

    pub fn new_created_resource(
        &mut self,
        source_path: PathBuf,
        r#type: CreatedResourceType,
    ) -> Result<CreatedResource<B>, ResourceSystemError> {
        self.new_resource(source_path, ResourceType::Created(r#type))
            .map(CreatedResource)
    }

    pub fn new_produced_resource(&mut self, source_path: PathBuf) -> Result<ProducedResource<B>, ResourceSystemError> {
        self.new_resource(source_path, ResourceType::Produced)
            .map(ProducedResource)
    }

    #[inline(always)]
    fn new_resource(&mut self, source_path: PathBuf, r#type: ResourceType) -> Result<Resource<B>, ResourceSystemError> {
        let (bus_client, bus_server) = B::new();
        let internal_resource = InternalResource {
            state: InternalResourceState::Uninitialized,
            bus_server,
            data: Arc::new(InternalResourceData { source_path, r#type }),
            init_data: None,
        };

        let data = internal_resource.data.clone();

        if !self
            .bus_client
            .send_request(ResourceSystemRequest::AddResource(internal_resource))
        {
            return Err(ResourceSystemError::BusDisconnected);
        }

        Ok(Resource {
            bus_client,
            data,
            init: None,
            dispose: None,
        })
    }
}

impl<S: ProcessSpawner, R: Runtime, B: Bus> Drop for ResourceSystem<S, R, B> {
    fn drop(&mut self) {
        self.bus_client.send_request(ResourceSystemRequest::Shutdown);
    }
}

#[derive(Debug, Clone, Copy)]
pub enum ResourceSystemError {
    IncorrectState {
        expected: ResourceState,
        actual: ResourceState,
    },
    BusDisconnected,
    MalformedResponse,
}
