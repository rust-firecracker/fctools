#[cfg(not(feature = "vmm-process"))]
use std::marker::PhantomData;
use std::{
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
    template::ResourceTemplate,
    Resource, ResourceState, ResourceType,
};

#[derive(Debug)]
pub struct ResourceSystem<S: ProcessSpawner, R: Runtime> {
    push_tx: mpsc::UnboundedSender<ResourceSystemPush<R>>,
    pull_rx: mpsc::UnboundedReceiver<ResourceSystemPull>,
    #[cfg(not(feature = "vmm-process"))]
    marker: PhantomData<S>,
    resources: Vec<Resource>,
    #[cfg(feature = "vmm-process")]
    pub(crate) process_spawner: S,
    #[cfg(feature = "vmm-process")]
    pub(crate) runtime: R,
    #[cfg(feature = "vmm-process")]
    pub(crate) ownership_model: VmmOwnershipModel,
}

const RESOURCE_BROADCAST_CAPACITY: usize = 10;

impl<S: ProcessSpawner, R: Runtime> ResourceSystem<S, R> {
    pub fn new(process_spawner: S, runtime: R, ownership_model: VmmOwnershipModel) -> Self {
        Self::new_inner(Vec::new(), Vec::new(), process_spawner, runtime, ownership_model)
    }

    pub fn with_capacity(process_spawner: S, runtime: R, ownership_model: VmmOwnershipModel, capacity: usize) -> Self {
        Self::new_inner(
            Vec::with_capacity(capacity),
            Vec::with_capacity(capacity),
            process_spawner,
            runtime,
            ownership_model,
        )
    }

    fn new_inner(
        owned_resources: Vec<OwnedResource<R>>,
        resources: Vec<Resource>,
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
            process_spawner.clone(),
            runtime.clone(),
            ownership_model,
        ));

        Self {
            push_tx,
            pull_rx,
            #[cfg(not(feature = "vmm-process"))]
            marker: PhantomData,
            resources,
            #[cfg(feature = "vmm-process")]
            process_spawner,
            #[cfg(feature = "vmm-process")]
            runtime,
            #[cfg(feature = "vmm-process")]
            ownership_model,
        }
    }

    pub fn get_resources(&self) -> &[Resource] {
        &self.resources
    }

    pub fn create_resource_from_template(
        &mut self,
        template: ResourceTemplate,
    ) -> Result<Resource, ResourceSystemError> {
        let (push_tx, push_rx) = mpsc::unbounded();
        let (pull_tx, pull_rx) = async_broadcast::broadcast(RESOURCE_BROADCAST_CAPACITY);

        let owned_resource = OwnedResource {
            state: match template.init_data.is_some() {
                true => OwnedResourceState::Initialized,
                false => OwnedResourceState::Uninitialized,
            },
            push_rx,
            pull_tx,
            data: Arc::new(template.data),
            init_data: template.init_data.map(Arc::new),
        };

        let data = owned_resource.data.clone();
        self.push_tx
            .unbounded_send(ResourceSystemPush::AddResource(owned_resource))
            .map_err(|_| ResourceSystemError::ChannelDisconnected)?;

        let resource = Resource {
            push_tx,
            pull_rx: Mutex::new(pull_rx),
            data,
            init_data: OnceLock::new(),
            disposed: Arc::new(AtomicBool::new(false)),
        };

        self.resources.push(resource.clone());
        Ok(resource)
    }

    pub fn create_resource<P: Into<PathBuf>>(
        &mut self,
        source_path: P,
        r#type: ResourceType,
    ) -> Result<Resource, ResourceSystemError> {
        let (push_tx, push_rx) = mpsc::unbounded();
        let (pull_tx, pull_rx) = async_broadcast::broadcast(RESOURCE_BROADCAST_CAPACITY);

        let owned_resource = OwnedResource {
            state: OwnedResourceState::Uninitialized,
            push_rx,
            pull_tx,
            data: Arc::new(ResourceData {
                source_path: source_path.into(),
                r#type,
            }),
            init_data: None,
        };

        let data = owned_resource.data.clone();

        self.push_tx
            .unbounded_send(ResourceSystemPush::AddResource(owned_resource))
            .map_err(|_| ResourceSystemError::ChannelDisconnected)?;

        let resource = Resource {
            push_tx,
            pull_rx: Mutex::new(pull_rx),
            data,
            init_data: OnceLock::new(),
            disposed: Arc::new(AtomicBool::new(false)),
        };

        self.resources.push(resource.clone());
        Ok(resource)
    }

    pub async fn synchronize(&mut self) -> Result<(), ResourceSystemError> {
        self.push_tx
            .unbounded_send(ResourceSystemPush::Synchronize)
            .map_err(|_| ResourceSystemError::ChannelDisconnected)?;

        match self.pull_rx.next().await {
            Some(ResourceSystemPull::SynchronizationComplete) => Ok(()),
            None => Err(ResourceSystemError::ChannelDisconnected),
        }
    }
}

impl<S: ProcessSpawner, R: Runtime> Drop for ResourceSystem<S, R> {
    fn drop(&mut self) {
        let _ = self.push_tx.unbounded_send(ResourceSystemPush::Shutdown);
    }
}

#[derive(Debug, Clone)]
pub enum ResourceSystemError {
    IncorrectState(ResourceState),
    ChannelDisconnected,
    MalformedResponse,
    ChangeOwnerError(Arc<ChangeOwnerError>),
    FilesystemError(Arc<std::io::Error>),
    SourcePathMissing,
    TaskJoinFailed,
}

impl std::fmt::Display for ResourceSystemError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ResourceSystemError::IncorrectState(state) => {
                write!(f, "Had incorrect {state} of resource, not permitted by operation")
            }
            ResourceSystemError::ChannelDisconnected => write!(f, "An internal channel connection broke"),
            ResourceSystemError::MalformedResponse => write!(
                f,
                "A malformed response was transmitted over an internal channel connection"
            ),
            ResourceSystemError::ChangeOwnerError(err) => write!(f, "An error occurred when changing ownership: {err}"),
            ResourceSystemError::FilesystemError(err) => write!(f, "A filesystem error occurred: {err}"),
            ResourceSystemError::SourcePathMissing => write!(f, "A moved resource's source path was missing"),
            ResourceSystemError::TaskJoinFailed => write!(f, "Joining on a set of runtime tasks failed"),
        }
    }
}

impl std::error::Error for ResourceSystemError {}
