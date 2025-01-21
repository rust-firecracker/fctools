use std::{
    marker::PhantomData,
    path::PathBuf,
    sync::{Arc, OnceLock},
};

use futures_channel::{mpsc, oneshot};
use futures_util::StreamExt;

use crate::{process_spawner::ProcessSpawner, runtime::Runtime, vmm::ownership::VmmOwnershipModel};

use super::{
    internal::{
        resource_system_main_task, InternalResource, InternalResourceData, InternalResourceState,
        ResourceSystemRequest, ResourceSystemResponse,
    },
    CreatedResource, CreatedResourceType, MovedResource, MovedResourceType, ProducedResource, Resource,
    ResourceHandleState, ResourceType,
};

pub struct ResourceSystem<S: ProcessSpawner, R: Runtime> {
    request_tx: mpsc::UnboundedSender<ResourceSystemRequest<R>>,
    response_rx: mpsc::UnboundedReceiver<ResourceSystemResponse>,
    marker: PhantomData<S>,
}

const RESPONSE_BROADCAST_CAPACITY: usize = 10;

impl<S: ProcessSpawner, R: Runtime> ResourceSystem<S, R> {
    pub fn new(process_spawner: S, runtime: R, ownership_model: VmmOwnershipModel) -> Self {
        Self::new_inner(Vec::new(), process_spawner, runtime, ownership_model)
    }

    pub fn with_capacity(process_spawner: S, runtime: R, ownership_model: VmmOwnershipModel, capacity: usize) -> Self {
        Self::new_inner(Vec::with_capacity(capacity), process_spawner, runtime, ownership_model)
    }

    #[inline(always)]
    fn new_inner(
        internal_resources: Vec<InternalResource<R>>,
        process_spawner: S,
        runtime: R,
        ownership_model: VmmOwnershipModel,
    ) -> Self {
        let (request_tx, request_rx) = mpsc::unbounded();
        let (response_tx, response_rx) = mpsc::unbounded();

        runtime.clone().spawn_task(resource_system_main_task(
            request_rx,
            response_tx,
            internal_resources,
            process_spawner,
            runtime,
            ownership_model,
        ));

        Self {
            request_tx,
            response_rx,
            marker: PhantomData,
        }
    }

    pub async fn shutdown(mut self) -> Result<(), ResourceSystemError> {
        self.request_tx
            .unbounded_send(ResourceSystemRequest::Shutdown)
            .map_err(|_| ResourceSystemError::ChannelDisconnected)?;

        if let Some(ResourceSystemResponse::ShutdownFinished) = self.response_rx.next().await {
            Ok(())
        } else {
            Err(ResourceSystemError::IncorrectResponseReceived)
        }
    }

    pub fn new_moved_resource(
        &self,
        source_path: PathBuf,
        r#type: MovedResourceType,
    ) -> Result<MovedResource, ResourceSystemError> {
        self.new_resource(source_path, ResourceType::Moved(r#type))
            .map(MovedResource)
    }

    pub fn new_created_resource(
        &self,
        source_path: PathBuf,
        r#type: CreatedResourceType,
    ) -> Result<CreatedResource, ResourceSystemError> {
        self.new_resource(source_path, ResourceType::Created(r#type))
            .map(CreatedResource)
    }

    pub fn new_produced_resource(&self, source_path: PathBuf) -> Result<ProducedResource, ResourceSystemError> {
        self.new_resource(source_path, ResourceType::Produced)
            .map(ProducedResource)
    }

    #[inline(always)]
    fn new_resource(&self, source_path: PathBuf, r#type: ResourceType) -> Result<Resource, ResourceSystemError> {
        let (request_tx, request_rx) = mpsc::unbounded();
        let (response_tx, response_rx) = async_broadcast::broadcast(RESPONSE_BROADCAST_CAPACITY);
        let internal_resource: InternalResource<R> = InternalResource {
            state: InternalResourceState::Uninitialized,
            request_rx,
            response_tx,
            data: Arc::new(InternalResourceData { source_path, r#type }),
            init_data: None,
        };

        let data = internal_resource.data.clone();

        self.request_tx
            .unbounded_send(ResourceSystemRequest::AddResource(internal_resource))
            .map_err(|_| ResourceSystemError::ChannelDisconnected)?;

        Ok(Resource {
            request_tx,
            response_rx,
            data,
            init_lock: OnceLock::new(),
            dispose_lock: OnceLock::new(),
        })
    }
}

impl<S: ProcessSpawner, R: Runtime> Drop for ResourceSystem<S, R> {
    fn drop(&mut self) {
        let _ = self.request_tx.unbounded_send(ResourceSystemRequest::Shutdown);
    }
}

#[derive(Debug, Clone, Copy)]
pub enum ResourceSystemError {
    IncorrectHandleState {
        expected: ResourceHandleState,
        actual: ResourceHandleState,
    },
    ChannelDisconnected,
    ChannelNotFound,
    IncorrectResponseReceived,
}
