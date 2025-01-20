use std::{future::poll_fn, task::Poll};

use futures_channel::{mpsc, oneshot};
use futures_util::{FutureExt, StreamExt};

use crate::{runtime::Runtime, vmm::resource_v3::ResourceRequest};

use super::Resource;

pub struct ResourceSystem<R: Runtime> {
    resource_tx: mpsc::UnboundedSender<Resource<R>>,
    shutdown_tx: Option<oneshot::Sender<()>>,
    shutdown_finished_rx: Option<oneshot::Receiver<Result<(), ResourceSystemError>>>,
}

impl<R: Runtime> ResourceSystem<R> {
    pub fn new(runtime: &R) -> Self {
        Self::new_inner(Vec::new(), runtime)
    }

    pub fn with_capacity(runtime: &R, capacity: usize) -> Self {
        Self::new_inner(Vec::with_capacity(capacity), runtime)
    }

    #[inline(always)]
    fn new_inner(resources: Vec<Resource<R>>, runtime: &R) -> Self {
        let (resource_tx, resource_rx) = mpsc::unbounded();
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let (shutdown_finished_tx, shutdown_finished_rx) = oneshot::channel();

        runtime.spawn_task(resource_system_main_task(
            resource_rx,
            shutdown_rx,
            shutdown_finished_tx,
            resources,
        ));

        Self {
            resource_tx,
            shutdown_tx: Some(shutdown_tx),
            shutdown_finished_rx: Some(shutdown_finished_rx),
        }
    }

    pub async fn shutdown(mut self) -> Result<(), ResourceSystemError> {
        self.shutdown_tx
            .take()
            .ok_or(ResourceSystemError::ChannelNotFound)?
            .send(())
            .map_err(|_| ResourceSystemError::ChannelBroken)?;

        self.shutdown_finished_rx
            .take()
            .ok_or(ResourceSystemError::ChannelNotFound)?
            .await
            .map_err(|_| ResourceSystemError::ChannelBroken)?
    }
}

impl<R: Runtime> Drop for ResourceSystem<R> {
    fn drop(&mut self) {
        if let Some(shutdown_tx) = self.shutdown_tx.take() {
            let _ = shutdown_tx.send(());
        }
    }
}

#[derive(Clone)]
pub enum ResourceSystemError {
    ChannelBroken,
    ChannelNotFound,
}

async fn resource_system_main_task<R: Runtime>(
    mut resource_rx: mpsc::UnboundedReceiver<Resource<R>>,
    mut shutdown_rx: oneshot::Receiver<()>,
    shutdown_finished_tx: oneshot::Sender<Result<(), ResourceSystemError>>,
    mut resources: Vec<Resource<R>>,
) {
    enum Incoming<R: Runtime> {
        Shutdown,
        Resource(Resource<R>),
        ResourceRequest(usize, ResourceRequest<R>),
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
