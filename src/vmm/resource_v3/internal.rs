use std::{future::poll_fn, path::PathBuf, sync::Arc, task::Poll};

use crate::{
    process_spawner::ProcessSpawner,
    runtime::Runtime,
    vmm::{ownership::VmmOwnershipModel, resource_v3::bus::BusServer},
};

use super::{
    bus::{Bus, BusOutgoing},
    system::ResourceSystemError,
    ResourceType,
};

pub enum InternalResourceState<R: Runtime> {
    Uninitialized,
    Initializing(R::Task<ResourceSystemError>),
    Initialized,
    Disposing(R::Task<ResourceSystemError>),
    Disposed,
}

pub struct InternalResource<R: Runtime, B: Bus> {
    pub state: InternalResourceState<R>,
    pub bus_server: B::Server<ResourceRequest, ResourceResponse>,
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
}

#[derive(Clone)]
pub enum ResourceResponse {
    Initialized {
        result: Result<(), ResourceSystemError>,
        init_data: Arc<InternalResourceInitData>,
    },
    Disposed(Result<(), ResourceSystemError>),
}

pub enum ResourceSystemRequest<R: Runtime, B: Bus> {
    AddResource(InternalResource<R, B>),
    Shutdown,
}

#[derive(Clone, Copy)]
pub enum ResourceSystemResponse {
    ShutdownFinished,
}

pub async fn resource_system_main_task<S: ProcessSpawner, R: Runtime, B: Bus>(
    mut bus_server: B::Server<ResourceSystemRequest<R, B>, ResourceSystemResponse>,
    mut internal_resources: Vec<InternalResource<R, B>>,
    process_spawner: S,
    runtime: R,
    ownership_model: VmmOwnershipModel,
) {
    loop {
        let (system, resource) = poll_fn(|cx| {
            if let Poll::Ready(Some(tuple)) = bus_server.poll(cx) {
                return Poll::Ready((Some(tuple), None));
            }

            for (resource_index, resource) in internal_resources.iter_mut().enumerate() {
                if let Poll::Ready(Some(tuple)) = resource.bus_server.poll(cx) {
                    return Poll::Ready((None, Some((resource_index, tuple.0, tuple.1))));
                }
            }

            Poll::Pending
        })
        .await;

        if let Some((request, outgoing)) = system {
            match request {
                ResourceSystemRequest::Shutdown => {
                    runtime.spawn_task(outgoing.write(ResourceSystemResponse::ShutdownFinished));
                }
                ResourceSystemRequest::AddResource(internal_resource) => {
                    internal_resources.push(internal_resource);
                }
            }
        } else if let Some((resource_idx, request, outgoing)) = resource {
            let resource = internal_resources
                .get_mut(resource_idx)
                .expect("resource_idx is incorrect");

            match request {
                ResourceRequest::Initialize(init_data) => todo!(),
                ResourceRequest::Dispose => todo!(),
            }
        }
    }
}
