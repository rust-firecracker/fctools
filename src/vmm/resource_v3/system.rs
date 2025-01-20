use std::{future::poll_fn, marker::PhantomData, task::Poll};

use futures_channel::{mpsc, oneshot};
use futures_util::{FutureExt, StreamExt};

use crate::{
    process_spawner::ProcessSpawner,
    runtime::Runtime,
    vmm::{ownership::VmmOwnershipModel, resource_v3::ResourceRequest},
};

use super::Resource;

pub struct ResourceSystem<S: ProcessSpawner, R: Runtime> {
    resource_tx: mpsc::UnboundedSender<Resource<R>>,
    shutdown_tx: Option<oneshot::Sender<()>>,
    shutdown_finished_rx: Option<oneshot::Receiver<Result<(), ResourceError>>>,
    marker: PhantomData<S>,
}

impl<S: ProcessSpawner, R: Runtime> ResourceSystem<S, R> {
    pub fn new(process_spawner: S, runtime: R, ownership_model: VmmOwnershipModel) -> Self {
        Self::new_inner(Vec::new(), process_spawner, runtime, ownership_model)
    }

    pub fn with_capacity(process_spawner: S, runtime: R, ownership_model: VmmOwnershipModel, capacity: usize) -> Self {
        Self::new_inner(Vec::with_capacity(capacity), process_spawner, runtime, ownership_model)
    }

    #[inline(always)]
    fn new_inner(
        resources: Vec<Resource<R>>,
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

    pub async fn shutdown(mut self) -> Result<(), ResourceError> {
        self.shutdown_tx
            .take()
            .ok_or(ResourceError::ChannelNotFound)?
            .send(())
            .map_err(|_| ResourceError::ChannelBroken)?;

        self.shutdown_finished_rx
            .take()
            .ok_or(ResourceError::ChannelNotFound)?
            .await
            .map_err(|_| ResourceError::ChannelBroken)?
    }
}

impl<S: ProcessSpawner, R: Runtime> Drop for ResourceSystem<S, R> {
    fn drop(&mut self) {
        if let Some(shutdown_tx) = self.shutdown_tx.take() {
            let _ = shutdown_tx.send(());
        }
    }
}

#[derive(Clone)]
pub enum ResourceError {
    ChannelBroken,
    ChannelNotFound,
}

async fn resource_system_main_task<S: ProcessSpawner, R: Runtime>(
    mut resource_rx: mpsc::UnboundedReceiver<Resource<R>>,
    mut shutdown_rx: oneshot::Receiver<()>,
    shutdown_finished_tx: oneshot::Sender<Result<(), ResourceError>>,
    mut resources: Vec<Resource<R>>,
    process_spawner: S,
    runtime: R,
    ownership_model: VmmOwnershipModel,
) {
    enum Incoming<R: Runtime> {
        Shutdown,
        Resource(Resource<R>),
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
            Incoming::Shutdown => todo!(),
            Incoming::Resource(resource) => todo!(),
            Incoming::ResourceRequest(_, resource_action) => todo!(),
        }
    }
}
