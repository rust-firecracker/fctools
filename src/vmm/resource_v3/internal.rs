use std::{future::poll_fn, path::PathBuf, pin::pin, sync::Arc, task::Poll};

use futures_channel::mpsc;
use futures_util::StreamExt;

use crate::{process_spawner::ProcessSpawner, runtime::Runtime, vmm::ownership::VmmOwnershipModel};

use super::{system::ResourceSystemError, ResourceType};

pub enum InternalResourceState<R: Runtime> {
    Uninitialized,
    Initializing(R::Task<ResourceSystemError>),
    Initialized,
    Disposing(R::Task<ResourceSystemError>),
    Disposed,
}

pub struct InternalResource<R: Runtime> {
    pub state: InternalResourceState<R>,
    pub request_rx: mpsc::UnboundedReceiver<ResourceRequest>,
    pub response_tx: async_broadcast::Sender<ResourceResponse>,
    pub data: Arc<InternalResourceData>,
    pub init_data: Option<Arc<InternalResourceInitData>>,
}

pub struct InternalResourceData {
    pub source_path: PathBuf,
    pub r#type: ResourceType,
}

pub struct InternalResourceInitData {
    pub effective_path: PathBuf,
    pub local_path: Option<PathBuf>,
}

pub enum ResourceRequest {
    Initialize(InternalResourceInitData),
    Dispose,
    Ping,
}

#[derive(Clone)]
pub enum ResourceResponse {
    Initialized {
        result: Result<(), ResourceSystemError>,
        init_data: Arc<InternalResourceInitData>,
    },
    Disposed(Result<(), ResourceSystemError>),
    Pong,
}

pub enum ResourceSystemRequest<R: Runtime> {
    AddResource(InternalResource<R>),
    Shutdown,
}

pub enum ResourceSystemResponse {
    ShutdownFinished,
}

pub async fn resource_system_main_task<S: ProcessSpawner, R: Runtime>(
    mut request_rx: mpsc::UnboundedReceiver<ResourceSystemRequest<R>>,
    response_tx: mpsc::UnboundedSender<ResourceSystemResponse>,
    mut internal_resources: Vec<InternalResource<R>>,
    process_spawner: S,
    runtime: R,
    ownership_model: VmmOwnershipModel,
) {
    enum Incoming<R: Runtime> {
        SystemRequest(ResourceSystemRequest<R>),
        ResourceRequest(usize, ResourceRequest),
    }

    loop {
        let incoming = poll_fn(|cx| {
            if let Poll::Ready(Some(request)) = request_rx.poll_next_unpin(cx) {
                return Poll::Ready(Incoming::SystemRequest(request));
            }

            for (resource_index, resource) in internal_resources.iter_mut().enumerate() {
                if let Poll::Ready(Some(resource_action)) = resource.request_rx.poll_next_unpin(cx) {
                    return Poll::Ready(Incoming::ResourceRequest(resource_index, resource_action));
                }
            }

            Poll::Pending
        })
        .await;

        match incoming {
            Incoming::SystemRequest(request) => match request {
                ResourceSystemRequest::AddResource(internal_resource) => {
                    internal_resources.push(internal_resource);
                }
                ResourceSystemRequest::Shutdown => {
                    let _ = response_tx.unbounded_send(ResourceSystemResponse::ShutdownFinished);
                    return;
                }
            },
            Incoming::ResourceRequest(resource_index, resource_action) => {
                let resource = internal_resources
                    .get_mut(resource_index)
                    .expect("resource_index is invalid. Internal library bug");

                match resource_action {
                    ResourceRequest::Ping => {
                        let resource_request_tx = resource.response_tx.clone();
                        runtime.spawn_task(async move {
                            let _ = pin!(resource_request_tx.broadcast_direct(ResourceResponse::Pong)).await;
                        });
                    }
                    ResourceRequest::Initialize(init_data) => {
                        resource.init_data = Some(Arc::new(init_data));
                    }
                    _ => {}
                }
            }
        }
    }
}
