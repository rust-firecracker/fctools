use std::{
    future::poll_fn,
    marker::PhantomData,
    path::PathBuf,
    pin::pin,
    sync::{Arc, OnceLock},
    task::Poll,
};

use futures_channel::{mpsc, oneshot};
use futures_util::{FutureExt, StreamExt};

use crate::{
    process_spawner::ProcessSpawner,
    runtime::Runtime,
    vmm::{
        ownership::VmmOwnershipModel,
        resource_v3::internal::{ResourceRequest, ResourceResponse},
    },
};

use super::{
    internal::{OwnedResource, OwnedResourceState, ResourceData},
    MovedResource, MovedResourceType, Resource, ResourceHandleState, ResourceType,
};

pub struct ResourceSystem<S: ProcessSpawner, R: Runtime> {
    resource_tx: mpsc::UnboundedSender<OwnedResource<R>>,
    shutdown_tx: Option<oneshot::Sender<()>>,
    shutdown_finished_rx: Option<oneshot::Receiver<Result<(), ResourceSystemError>>>,
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
        resources: Vec<OwnedResource<R>>,
        process_spawner: S,
        runtime: R,
        ownership_model: VmmOwnershipModel,
    ) -> Self {
        let (resource_tx, resource_rx) = mpsc::unbounded();
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let (shutdown_finished_tx, shutdown_finished_rx) = oneshot::channel();

        runtime.clone().spawn_task(resource_system_main_task(
            resource_rx,
            shutdown_rx,
            shutdown_finished_tx,
            resources,
            process_spawner,
            runtime,
            ownership_model,
        ));

        Self {
            resource_tx,
            shutdown_tx: Some(shutdown_tx),
            shutdown_finished_rx: Some(shutdown_finished_rx),
            marker: PhantomData,
        }
    }

    pub async fn shutdown(mut self) -> Result<(), ResourceSystemError> {
        self.shutdown_tx
            .take()
            .ok_or(ResourceSystemError::ChannelNotFound)?
            .send(())
            .map_err(|_| ResourceSystemError::ChannelDisconnected)?;

        self.shutdown_finished_rx
            .take()
            .ok_or(ResourceSystemError::ChannelNotFound)?
            .await
            .map_err(|_| ResourceSystemError::ChannelDisconnected)?
    }

    pub fn create_moved_resource(
        &self,
        source_path: PathBuf,
        r#type: MovedResourceType,
    ) -> Result<MovedResource, ResourceSystemError> {
        self.create_resource(source_path, ResourceType::Moved(r#type))
            .map(|inner| MovedResource(inner))
    }

    #[inline(always)]
    fn create_resource(&self, source_path: PathBuf, r#type: ResourceType) -> Result<Resource, ResourceSystemError> {
        let (request_tx, request_rx) = mpsc::unbounded();
        let (response_tx, response_rx) = async_broadcast::broadcast(RESPONSE_BROADCAST_CAPACITY);
        let resource: OwnedResource<R> = OwnedResource {
            state: OwnedResourceState::Uninitialized,
            request_rx,
            response_tx,
            data: Arc::new(ResourceData { source_path, r#type }),
            init_data: None,
        };

        let data = resource.data.clone();

        self.resource_tx
            .unbounded_send(resource)
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
        if let Some(shutdown_tx) = self.shutdown_tx.take() {
            let _ = shutdown_tx.send(());
        }
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

async fn resource_system_main_task<S: ProcessSpawner, R: Runtime>(
    mut resource_rx: mpsc::UnboundedReceiver<OwnedResource<R>>,
    mut shutdown_rx: oneshot::Receiver<()>,
    shutdown_finished_tx: oneshot::Sender<Result<(), ResourceSystemError>>,
    mut resources: Vec<OwnedResource<R>>,
    process_spawner: S,
    runtime: R,
    ownership_model: VmmOwnershipModel,
) {
    enum Incoming<R: Runtime> {
        Shutdown,
        Resource(OwnedResource<R>),
        ResourceRequest(usize, ResourceRequest),
    }

    loop {
        let incoming = poll_fn(|cx| {
            if let Poll::Ready(Some(resource)) = resource_rx.poll_next_unpin(cx) {
                return Poll::Ready(Incoming::Resource(resource));
            }

            if let Poll::Ready(Ok(_)) = shutdown_rx.poll_unpin(cx) {
                return Poll::Ready(Incoming::Shutdown);
            }

            for (resource_index, resource) in resources.iter_mut().enumerate() {
                if let Poll::Ready(Some(resource_action)) = resource.request_rx.poll_next_unpin(cx) {
                    return Poll::Ready(Incoming::ResourceRequest(resource_index, resource_action));
                }
            }

            Poll::Pending
        })
        .await;

        match incoming {
            Incoming::Shutdown => {
                let _ = shutdown_finished_tx.send(Ok(()));
                return;
            }
            Incoming::Resource(resource) => {
                resources.push(resource);
            }
            Incoming::ResourceRequest(resource_index, resource_action) => {
                let resource = resources
                    .get_mut(resource_index)
                    .expect("resource_index is invalid. Internal library bug");

                match resource_action {
                    ResourceRequest::Ping => {
                        schedule_response(&runtime, resource, ResourceResponse::Pong);
                    }
                    _ => {}
                }
            }
        }
    }
}

#[inline(always)]
fn schedule_response<R: Runtime>(runtime: &R, resource: &mut OwnedResource<R>, response: ResourceResponse) {
    let sender = resource.response_tx.clone();
    runtime.spawn_task(async move {
        let _ = pin!(sender.broadcast_direct(response)).await;
    });
}
