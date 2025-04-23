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
        resource_system_main_task, OwnedResource, OwnedResourceState, ResourceData, ResourceSystemRequest,
        ResourceSystemResponse,
    },
    Resource, ResourceState, ResourceType,
};

/// A [ResourceSystem] represents a non-cloneable object connected to a background task running on a [Runtime]. This task
/// is a central task that responds to messages from the connected [ResourceSystem] and [Resource]s and spawns various
/// auxiliary tasks onto the same [Runtime] that perform asynchronous resource actions such as initialization and disposal.
///
/// The [ResourceSystem] allows the creation of new [Resource]s and global synchronization with the task. After being dropped,
/// the [ResourceSystem] will transmit a shutdown message that will end the task. The [ResourceSystem] requires not only a
/// [Runtime], but also a [ProcessSpawner] and a [VmmOwnershipModel] in order to perform its functionality, and these
/// objects will be used by VMs and VMM processes that internally embed a [ResourceSystem].
#[derive(Debug)]
pub struct ResourceSystem<S: ProcessSpawner, R: Runtime> {
    request_tx: mpsc::UnboundedSender<ResourceSystemRequest<R>>,
    response_rx: mpsc::UnboundedReceiver<ResourceSystemResponse>,
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
    /// Create a new [ResourceSystem] with empty buffers for storing resource objects, using the given
    /// [ProcessSpawner], [Runtime] and [VmmOwnershipModel].
    pub fn new(process_spawner: S, runtime: R, ownership_model: VmmOwnershipModel) -> Self {
        Self::new_inner(Vec::new(), Vec::new(), process_spawner, runtime, ownership_model)
    }

    /// Create a new [ResourceSystem] with pre-reserved buffers of a certain capacity for storing resource objects,
    /// using the given [ProcessSpawner], [Runtime] and [VmmOwnershipModel].
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
        let (request_tx, request_rx) = mpsc::unbounded();
        let (response_tx, response_rx) = mpsc::unbounded();

        runtime.clone().spawn_task(resource_system_main_task::<S, R>(
            request_rx,
            response_tx,
            owned_resources,
            process_spawner.clone(),
            runtime.clone(),
            ownership_model,
        ));

        Self {
            request_tx,
            response_rx,
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

    /// Get a shared slice into an internal buffer that contains all [Resource]s within this [ResourceSystem], not
    /// including any clones of given out [Resource]s. This slice can be cloned to produce a [Vec] if owned [Resource]
    /// instances are needed, but, by default, no cloning occurs when calling this function.
    pub fn get_resources(&self) -> &[Resource] {
        &self.resources
    }

    /// Create a [Resource] in this [ResourceSystem] from a given source path and a [ResourceType]. The data will
    /// immediately be transmitted into the [ResourceSystem]'s central task, and an extra [Resource] clone will be
    /// stored inside the buffer accessible via [get_resources](ResourceSystem::get_resources).
    pub fn create_resource<P: Into<PathBuf>>(
        &mut self,
        source_path: P,
        r#type: ResourceType,
    ) -> Result<Resource, ResourceSystemError> {
        let (request_tx, request_rx) = mpsc::unbounded();
        let (response_tx, response_rx) = async_broadcast::broadcast(RESOURCE_BROADCAST_CAPACITY);

        let owned_resource = OwnedResource {
            state: OwnedResourceState::Uninitialized,
            request_rx,
            response_tx,
            data: Arc::new(ResourceData {
                source_path: source_path.into(),
                r#type,
            }),
            init_data: None,
        };

        let data = owned_resource.data.clone();

        self.request_tx
            .unbounded_send(ResourceSystemRequest::AddResource(owned_resource))
            .map_err(|_| ResourceSystemError::ChannelDisconnected)?;

        let resource = Resource {
            request_tx,
            response_rx: Mutex::new(response_rx),
            data,
            init_data: OnceLock::new(),
            disposed: Arc::new(AtomicBool::new(false)),
        };

        self.resources.push(resource.clone());
        Ok(resource)
    }

    /// Performs manual synchronization with the underlying central task. This operation waits until all initialization,
    /// disposal or other scheduled tasks complete (regardless of whether they complete successfully or not). If you
    /// intend only to wait on a single operation such as a single resource's initialization, prefer using
    /// [Resource::initialize], [Resource::initialize_with_same_path] or [Resource::dispose] instead, as they will return
    /// an error if an operation fails, unlike this function.
    pub async fn synchronize(&mut self) -> Result<(), ResourceSystemError> {
        self.request_tx
            .unbounded_send(ResourceSystemRequest::Synchronize)
            .map_err(|_| ResourceSystemError::ChannelDisconnected)?;

        match self.response_rx.next().await {
            Some(ResourceSystemResponse::SynchronizationComplete) => Ok(()),
            None => Err(ResourceSystemError::ChannelDisconnected),
        }
    }
}

impl<S: ProcessSpawner, R: Runtime> Drop for ResourceSystem<S, R> {
    fn drop(&mut self) {
        let _ = self.request_tx.unbounded_send(ResourceSystemRequest::Shutdown);
    }
}

/// An error that can be emitted by a [ResourceSystem] or a standalone [Resource].
#[derive(Debug, Clone)]
pub enum ResourceSystemError {
    /// A [Resource]'s [ResourceState] did not permit the requested operation.
    IncorrectState(ResourceState),
    /// An internal channel connection was severed from the other side.
    ChannelDisconnected,
    /// A malformed response was transmitted over an internal channel connection.
    MalformedResponse,
    /// A [ChangeOwnerError] occurred when executing a scheduled action.
    ChangeOwnerError(Arc<ChangeOwnerError>),
    /// An I/O error occurred while interacting with the filesystem for a scheduled action.
    FilesystemError(Arc<std::io::Error>),
    /// A [Resource]'s source path was missing at the time of the execution of a scheduled action.
    SourcePathMissing,
}

impl std::fmt::Display for ResourceSystemError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ResourceSystemError::IncorrectState(state) => {
                write!(f, "Had incorrect {state} of resource, not permitted by operation")
            }
            ResourceSystemError::ChannelDisconnected => {
                write!(f, "An internal channel connection was severed from the other side")
            }
            ResourceSystemError::MalformedResponse => write!(
                f,
                "A malformed response was transmitted over an internal channel connection"
            ),
            ResourceSystemError::ChangeOwnerError(err) => write!(f, "An error occurred when changing ownership: {err}"),
            ResourceSystemError::FilesystemError(err) => write!(f, "A filesystem error occurred: {err}"),
            ResourceSystemError::SourcePathMissing => write!(f, "A resource's source path is missing"),
        }
    }
}

impl std::error::Error for ResourceSystemError {}
