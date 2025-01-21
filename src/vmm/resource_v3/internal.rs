use std::{future::poll_fn, path::PathBuf, pin::Pin, sync::Arc, task::Poll};

use crate::{
    process_spawner::ProcessSpawner,
    runtime::Runtime,
    vmm::{ownership::VmmOwnershipModel, resource_v3::bus::BusServer},
};

use super::{bus::Bus, system::ResourceSystemError, ResourceType};

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
    enum Incoming<R: Runtime, B: Bus> {
        SystemRequest(ResourceSystemRequest<R, B>),
        ResourceRequest(usize, ResourceRequest),
    }

    loop {
        let incoming = poll_fn(|cx| {
            if let Poll::Ready(Some(request)) = Pin::new(&mut bus_server).poll(cx) {
                return Poll::Ready(Incoming::SystemRequest(request));
            }

            for (resource_index, resource) in internal_resources.iter_mut().enumerate() {
                if let Poll::Ready(Some(request)) = Pin::new(&mut resource.bus_server).poll(cx) {
                    return Poll::Ready(Incoming::ResourceRequest(resource_index, request));
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
                    let _ = bus_server.respond(ResourceSystemResponse::ShutdownFinished);
                    return;
                }
            },
            Incoming::ResourceRequest(resource_index, request) => {
                let resource = internal_resources
                    .get_mut(resource_index)
                    .expect("resource_index is invalid. Internal library bug");

                match request {
                    ResourceRequest::Ping => {
                        resource.bus_server.respond(ResourceResponse::Pong).await;
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
