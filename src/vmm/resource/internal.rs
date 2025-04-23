use std::{future::poll_fn, path::PathBuf, pin::pin, sync::Arc, task::Poll};

use futures_channel::mpsc;
use futures_util::StreamExt;

use crate::{
    process_spawner::ProcessSpawner,
    runtime::{Runtime, RuntimeTask},
    vmm::ownership::{downgrade_owner, upgrade_owner, VmmOwnershipModel},
};

use super::{system::ResourceSystemError, CreatedResourceType, MovedResourceType, ResourceType};

pub enum OwnedResourceState<R: Runtime> {
    Uninitialized,
    Initializing(R::Task<Result<ResourceInitData, ResourceSystemError>>),
    Initialized,
    Disposing(R::Task<Result<(), ResourceSystemError>>),
    Disposed,
}

pub struct OwnedResource<R: Runtime> {
    pub state: OwnedResourceState<R>,
    pub request_rx: mpsc::UnboundedReceiver<ResourceRequest>,
    pub response_tx: async_broadcast::Sender<ResourceResponse>,
    pub data: Arc<ResourceData>,
    pub init_data: Option<Arc<ResourceInitData>>,
}

#[derive(Debug, Clone)]
pub struct ResourceData {
    pub source_path: PathBuf,
    pub r#type: ResourceType,
}

#[derive(Debug, Clone)]
pub struct ResourceInitData {
    pub effective_path: PathBuf,
    pub local_path: Option<PathBuf>,
}

pub enum ResourceRequest {
    Initialize(ResourceInitData),
    Dispose,
}

#[derive(Debug, Clone)]
pub enum ResourceResponse {
    Initialized(Result<Arc<ResourceInitData>, ResourceSystemError>),
    Disposed(Result<(), ResourceSystemError>),
}

pub enum ResourceSystemRequest<R: Runtime> {
    AddResource(OwnedResource<R>),
    Synchronize,
    Shutdown,
}

pub enum ResourceSystemResponse {
    SynchronizationComplete,
}

pub async fn resource_system_main_task<S: ProcessSpawner, R: Runtime>(
    mut request_rx: mpsc::UnboundedReceiver<ResourceSystemRequest<R>>,
    response_tx: mpsc::UnboundedSender<ResourceSystemResponse>,
    mut owned_resources: Vec<OwnedResource<R>>,
    process_spawner: S,
    runtime: R,
    ownership_model: VmmOwnershipModel,
) {
    enum Incoming<R: Runtime> {
        SystemRequest(ResourceSystemRequest<R>),
        ResourceRequest(usize, ResourceRequest),
        FinishedInitTask(usize, Result<ResourceInitData, ResourceSystemError>),
        FinishedDisposeTask(usize, Result<(), ResourceSystemError>),
        FinishedBroadcastTask(usize),
    }

    let mut synchronization_in_progress = false;
    let mut broadcast_tasks = Vec::<R::Task<()>>::new();

    loop {
        let incoming = poll_fn(|cx| {
            for (resource_index, resource) in owned_resources.iter_mut().enumerate() {
                if let Poll::Ready(Some(request)) = resource.request_rx.poll_next_unpin(cx) {
                    return Poll::Ready(Incoming::ResourceRequest(resource_index, request));
                }

                if let OwnedResourceState::Initializing(ref mut task) = resource.state {
                    if let Poll::Ready(Some(result)) = task.poll_join(cx) {
                        return Poll::Ready(Incoming::FinishedInitTask(resource_index, result));
                    }
                } else if let OwnedResourceState::Disposing(ref mut task) = resource.state {
                    if let Poll::Ready(Some(result)) = task.poll_join(cx) {
                        return Poll::Ready(Incoming::FinishedDisposeTask(resource_index, result));
                    }
                }
            }

            if let Poll::Ready(Some(request)) = request_rx.poll_next_unpin(cx) {
                return Poll::Ready(Incoming::SystemRequest(request));
            }

            for (index, task) in broadcast_tasks.iter_mut().enumerate() {
                if let Poll::Ready(Some(_)) = task.poll_join(cx) {
                    return Poll::Ready(Incoming::FinishedBroadcastTask(index));
                }
            }

            Poll::Pending
        })
        .await;

        match incoming {
            Incoming::SystemRequest(request) => match request {
                ResourceSystemRequest::AddResource(owned_resource) => {
                    owned_resources.push(owned_resource);
                }
                ResourceSystemRequest::Shutdown => {
                    return;
                }
                ResourceSystemRequest::Synchronize => {
                    synchronization_in_progress = true;
                }
            },
            Incoming::ResourceRequest(resource_index, request) => {
                let Some(resource) = owned_resources.get_mut(resource_index) else {
                    continue;
                };

                match request {
                    ResourceRequest::Initialize(init_data) => {
                        let init_task = runtime.spawn_task(resource_system_init_task(
                            resource.data.clone(),
                            init_data,
                            runtime.clone(),
                            process_spawner.clone(),
                            ownership_model,
                        ));

                        resource.state = OwnedResourceState::Initializing(init_task);
                    }
                    ResourceRequest::Dispose => {
                        let dispose_task = runtime.spawn_task(resource_system_dispose_task(
                            resource.init_data.clone().unwrap(),
                            runtime.clone(),
                            process_spawner.clone(),
                            ownership_model,
                        ));

                        resource.state = OwnedResourceState::Disposing(dispose_task);
                    }
                }
            }
            Incoming::FinishedInitTask(resource_index, result) => {
                let Some(resource) = owned_resources.get_mut(resource_index) else {
                    continue;
                };

                match result {
                    Ok(init_data) => {
                        let init_data = Arc::new(init_data);
                        resource.init_data = Some(init_data.clone());
                        resource.state = OwnedResourceState::Initialized;
                        broadcast_tasks.push(defer_resource_pull(
                            resource,
                            &runtime,
                            ResourceResponse::Initialized(Ok(init_data)),
                        ));
                    }
                    Err(err) => {
                        resource.state = OwnedResourceState::Uninitialized;
                        broadcast_tasks.push(defer_resource_pull(
                            resource,
                            &runtime,
                            ResourceResponse::Initialized(Err(err)),
                        ));
                    }
                }
            }
            Incoming::FinishedDisposeTask(resource_index, result) => {
                let Some(resource) = owned_resources.get_mut(resource_index) else {
                    continue;
                };

                match result {
                    Ok(_) => {
                        resource.state = OwnedResourceState::Disposed;
                        broadcast_tasks.push(defer_resource_pull(
                            resource,
                            &runtime,
                            ResourceResponse::Disposed(Ok(())),
                        ));
                    }
                    Err(err) => {
                        resource.state = OwnedResourceState::Initialized;
                        broadcast_tasks.push(defer_resource_pull(
                            resource,
                            &runtime,
                            ResourceResponse::Disposed(Err(err)),
                        ));
                    }
                }
            }
            Incoming::FinishedBroadcastTask(index) => {
                broadcast_tasks.remove(index);
            }
        };

        if synchronization_in_progress {
            let pending_tasks = owned_resources
                .iter()
                .filter(|resource| {
                    matches!(
                        resource.state,
                        OwnedResourceState::Initializing(_) | OwnedResourceState::Disposing(_)
                    )
                })
                .count()
                + broadcast_tasks.len();

            if pending_tasks == 0 {
                synchronization_in_progress = false;
                let _ = response_tx.unbounded_send(ResourceSystemResponse::SynchronizationComplete);
            }
        }
    }
}

#[inline]
fn defer_resource_pull<R: Runtime>(
    resource: &mut OwnedResource<R>,
    runtime: &R,
    pull: ResourceResponse,
) -> R::Task<()> {
    let tx = resource.response_tx.clone();
    runtime.spawn_task(async move {
        let _ = pin!(tx.broadcast_direct(pull)).await;
    })
}

async fn resource_system_init_task<S: ProcessSpawner, R: Runtime>(
    data: Arc<ResourceData>,
    init_data: ResourceInitData,
    runtime: R,
    process_spawner: S,
    ownership_model: VmmOwnershipModel,
) -> Result<ResourceInitData, ResourceSystemError> {
    match data.r#type {
        ResourceType::Moved(moved_resource_type) => {
            if data.source_path == init_data.effective_path {
                return Ok(init_data);
            }

            upgrade_owner(&data.source_path, ownership_model, &process_spawner, &runtime)
                .await
                .map_err(|err| ResourceSystemError::ChangeOwnerError(Arc::new(err)))?;

            if !runtime
                .fs_exists(&data.source_path)
                .await
                .map_err(|err| ResourceSystemError::FilesystemError(Arc::new(err)))?
            {
                return Err(ResourceSystemError::SourcePathMissing);
            }

            if let Some(parent_path) = init_data.effective_path.parent() {
                runtime
                    .fs_create_dir_all(parent_path)
                    .await
                    .map_err(|err| ResourceSystemError::FilesystemError(Arc::new(err)))?;
            }

            match moved_resource_type {
                MovedResourceType::Copied => {
                    runtime
                        .fs_copy(&data.source_path, &init_data.effective_path)
                        .await
                        .map_err(|err| ResourceSystemError::FilesystemError(Arc::new(err)))?;
                }
                MovedResourceType::HardLinked => {
                    runtime
                        .fs_hard_link(&data.source_path, &init_data.effective_path)
                        .await
                        .map_err(|err| ResourceSystemError::FilesystemError(Arc::new(err)))?;
                }
                MovedResourceType::CopiedOrHardLinked => {
                    if runtime
                        .fs_copy(&data.source_path, &init_data.effective_path)
                        .await
                        .is_err()
                    {
                        runtime
                            .fs_hard_link(&data.source_path, &init_data.effective_path)
                            .await
                            .map_err(|err| ResourceSystemError::FilesystemError(Arc::new(err)))?;
                    }
                }
                MovedResourceType::HardLinkedOrCopied => {
                    if runtime
                        .fs_hard_link(&data.source_path, &init_data.effective_path)
                        .await
                        .is_err()
                    {
                        runtime
                            .fs_copy(&data.source_path, &init_data.effective_path)
                            .await
                            .map_err(|err| ResourceSystemError::FilesystemError(Arc::new(err)))?;
                    }
                }
                MovedResourceType::Renamed => {
                    runtime
                        .fs_rename(&data.source_path, &init_data.effective_path)
                        .await
                        .map_err(|err| ResourceSystemError::FilesystemError(Arc::new(err)))?;
                }
            }
        }
        ResourceType::Created(created_resource_type) => {
            if let Some(parent_path) = init_data.effective_path.parent() {
                runtime
                    .fs_create_dir_all(&parent_path)
                    .await
                    .map_err(|err| ResourceSystemError::FilesystemError(Arc::new(err)))?;
            }

            match created_resource_type {
                CreatedResourceType::File => {
                    runtime
                        .fs_create_file(&init_data.effective_path)
                        .await
                        .map_err(|err| ResourceSystemError::FilesystemError(Arc::new(err)))?;
                }
                CreatedResourceType::Fifo => {
                    crate::syscall::mkfifo(&init_data.effective_path)
                        .map_err(|err| ResourceSystemError::FilesystemError(Arc::new(err)))?;
                }
            }

            downgrade_owner(&init_data.effective_path, ownership_model)
                .map_err(|err| ResourceSystemError::ChangeOwnerError(Arc::new(err)))?;
        }
        ResourceType::Produced => {
            if let Some(parent_path) = init_data.effective_path.parent() {
                runtime
                    .fs_create_dir_all(&parent_path)
                    .await
                    .map_err(|err| ResourceSystemError::FilesystemError(Arc::new(err)))?;

                downgrade_owner(&parent_path, ownership_model)
                    .map_err(|err| ResourceSystemError::ChangeOwnerError(Arc::new(err)))?;
            }
        }
    };

    Ok(init_data)
}

async fn resource_system_dispose_task<R: Runtime, S: ProcessSpawner>(
    init_data: Arc<ResourceInitData>,
    runtime: R,
    process_spawner: S,
    ownership_model: VmmOwnershipModel,
) -> Result<(), ResourceSystemError> {
    upgrade_owner(&init_data.effective_path, ownership_model, &process_spawner, &runtime)
        .await
        .map_err(|err| ResourceSystemError::ChangeOwnerError(Arc::new(err)))?;

    runtime
        .fs_remove_file(&init_data.effective_path)
        .await
        .map_err(|err| ResourceSystemError::FilesystemError(Arc::new(err)))
}
