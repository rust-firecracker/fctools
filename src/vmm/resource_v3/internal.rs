use std::{future::poll_fn, path::PathBuf, sync::Arc, task::Poll};

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
    pub push_rx: mpsc::UnboundedReceiver<ResourcePush>,
    pub pull_tx: async_broadcast::Sender<ResourcePull>,
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

pub enum ResourcePush {
    Initialize(InternalResourceInitData),
    Dispose,
}

#[derive(Clone)]
pub enum ResourcePull {
    Initialized {
        result: Result<(), ResourceSystemError>,
        init_data: Arc<InternalResourceInitData>,
    },
    Disposed(Result<(), ResourceSystemError>),
}

pub enum ResourceSystemPush<R: Runtime> {
    AddResource(InternalResource<R>),
    Shutdown,
}

pub enum ResourceSystemPull {
    ShutdownFinished,
}

pub async fn resource_system_main_task<S: ProcessSpawner, R: Runtime>(
    mut push_rx: mpsc::UnboundedReceiver<ResourceSystemPush<R>>,
    pull_tx: mpsc::UnboundedSender<ResourceSystemPull>,
    mut internal_resources: Vec<InternalResource<R>>,
    process_spawner: S,
    runtime: R,
    ownership_model: VmmOwnershipModel,
) {
    enum Incoming<R: Runtime> {
        SystemPush(ResourceSystemPush<R>),
        ResourcePush(usize, ResourcePush),
    }

    loop {
        let incoming = poll_fn(|cx| {
            if let Poll::Ready(Some(push)) = push_rx.poll_next_unpin(cx) {
                return Poll::Ready(Incoming::SystemPush(push));
            }

            for (resource_index, resource) in internal_resources.iter_mut().enumerate() {
                if let Poll::Ready(Some(push)) = resource.push_rx.poll_next_unpin(cx) {
                    return Poll::Ready(Incoming::ResourcePush(resource_index, push));
                }
            }

            Poll::Pending
        })
        .await;

        match incoming {
            Incoming::SystemPush(push) => match push {
                ResourceSystemPush::AddResource(internal_resource) => {
                    internal_resources.push(internal_resource);
                }
                ResourceSystemPush::Shutdown => {
                    let _ = pull_tx.unbounded_send(ResourceSystemPull::ShutdownFinished);
                    return;
                }
            },
            Incoming::ResourcePush(idx, push) => {
                let resource = internal_resources.get_mut(idx).expect("idx is invalid. Iterator bug");

                match push {
                    ResourcePush::Initialize(init_data) => todo!(),
                    ResourcePush::Dispose => todo!(),
                }
            }
        }
    }
}
