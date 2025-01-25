use std::{
    marker::PhantomData,
    path::PathBuf,
    sync::{atomic::AtomicBool, Arc, Mutex, OnceLock},
};

use futures_channel::mpsc;
use futures_util::StreamExt;

use crate::{
    process_spawner::ProcessSpawner,
    runtime::Runtime,
    vmm::ownership::{ChangeOwnerError, VmmOwnershipModel},
};

use super::{
    internal::{
        resource_system_main_task, OwnedResource, OwnedResourceState, ResourceData, ResourceSystemPull,
        ResourceSystemPush,
    },
    CreatedResource, CreatedResourceType, MovedResource, MovedResourceType, ProducedResource, Resource, ResourceState,
    ResourceType,
};

#[derive(Debug)]
pub struct ResourceSystem<S: ProcessSpawner, R: Runtime> {
    push_tx: mpsc::UnboundedSender<ResourceSystemPush<R>>,
    pull_rx: mpsc::UnboundedReceiver<ResourceSystemPull>,
    marker: PhantomData<S>,
}

const RESOURCE_BROADCAST_CAPACITY: usize = 5;

impl<S: ProcessSpawner, R: Runtime> ResourceSystem<S, R> {
    pub fn new(process_spawner: S, runtime: R, ownership_model: VmmOwnershipModel) -> Self {
        Self::new_inner(Vec::new(), process_spawner, runtime, ownership_model)
    }

    pub fn with_capacity(process_spawner: S, runtime: R, ownership_model: VmmOwnershipModel, capacity: usize) -> Self {
        Self::new_inner(Vec::with_capacity(capacity), process_spawner, runtime, ownership_model)
    }

    #[inline(always)]
    fn new_inner(
        owned_resources: Vec<OwnedResource<R>>,
        process_spawner: S,
        runtime: R,
        ownership_model: VmmOwnershipModel,
    ) -> Self {
        let (push_tx, push_rx) = mpsc::unbounded();
        let (pull_tx, pull_rx) = mpsc::unbounded();

        runtime.clone().spawn_task(resource_system_main_task::<S, R>(
            push_rx,
            pull_tx,
            owned_resources,
            process_spawner,
            runtime,
            ownership_model,
        ));

        Self {
            push_tx,
            pull_rx,
            marker: PhantomData,
        }
    }

    pub async fn shutdown(mut self) -> Result<(), ResourceSystemError> {
        self.push_tx
            .unbounded_send(ResourceSystemPush::Shutdown)
            .map_err(|_| ResourceSystemError::ChannelDisconnected)?;

        match self.pull_rx.next().await {
            Some(ResourceSystemPull::ShutdownFinished(result)) => result,
            Some(_) => Err(ResourceSystemError::MalformedResponse),
            None => Err(ResourceSystemError::ChannelDisconnected),
        }
    }

    pub async fn wait_for_pending_tasks(&mut self) -> Result<(), ResourceSystemError> {
        self.push_tx
            .unbounded_send(ResourceSystemPush::AwaitPendingTasks)
            .map_err(|_| ResourceSystemError::ChannelDisconnected)?;

        match self.pull_rx.next().await {
            Some(ResourceSystemPull::PendingTasksComplete) => Ok(()),
            Some(_) => Err(ResourceSystemError::MalformedResponse),
            None => Err(ResourceSystemError::ChannelDisconnected),
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
        let (push_tx, push_rx) = mpsc::unbounded();
        let (pull_tx, pull_rx) = async_broadcast::broadcast(RESOURCE_BROADCAST_CAPACITY);

        let owned_resource = OwnedResource {
            state: OwnedResourceState::Uninitialized,
            push_rx,
            pull_tx,
            data: Arc::new(ResourceData {
                source_path,
                r#type,
                linked: AtomicBool::new(true),
            }),
        };

        let data = owned_resource.data.clone();

        self.push_tx
            .unbounded_send(ResourceSystemPush::AddResource(owned_resource))
            .map_err(|_| ResourceSystemError::ChannelDisconnected)?;

        Ok(Resource {
            push_tx,
            pull_rx: Mutex::new(pull_rx),
            data,
            init_data: OnceLock::new(),
            disposed: Arc::new(AtomicBool::new(false)),
        })
    }
}

impl<S: ProcessSpawner, R: Runtime> Drop for ResourceSystem<S, R> {
    fn drop(&mut self) {
        let _ = self.push_tx.unbounded_send(ResourceSystemPush::Shutdown);
    }
}

#[derive(Debug, Clone)]
pub enum ResourceSystemError {
    IncorrectState {
        expected: ResourceState,
        actual: ResourceState,
    },
    ChannelDisconnected,
    MalformedResponse,
    ChangeOwnerError(Arc<ChangeOwnerError>),
    FilesystemError(Arc<std::io::Error>),
    SourcePathMissing,
    TaskJoinFailed,
}
