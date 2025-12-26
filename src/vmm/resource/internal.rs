use std::{
    future::poll_fn,
    path::PathBuf,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    task::Poll,
};

use futures_channel::mpsc::{UnboundedReceiver, UnboundedSender};
use futures_util::StreamExt;

use super::{CreatedResourceType, MovedResourceType, ResourceType, system::ResourceSystemError};
use crate::{
    process_spawner::ProcessSpawner,
    runtime::{Runtime, RuntimeTask},
    vmm::ownership::{VmmOwnershipModel, downgrade_owner, upgrade_owner},
};

#[derive(Debug)]
pub struct ResourceInfo {
    pub request_tx: UnboundedSender<ResourceRequest>,
    pub initial_path: PathBuf,
    pub r#type: ResourceType,
    pub init_info: OnceLock<Arc<ResourceInitInfo>>,
    pub disposed: AtomicBool,
}

#[derive(Debug, Clone)]
pub struct ResourceInitInfo {
    pub effective_path: PathBuf,
    pub virtual_path: Option<PathBuf>,
}

pub struct OwnedResource<R: Runtime> {
    pub init_task: Option<R::Task<Result<ResourceInitInfo, ResourceSystemError>>>,
    pub dispose_task: Option<R::Task<Result<(), ResourceSystemError>>>,
    pub request_rx: UnboundedReceiver<ResourceRequest>,
    pub info: Arc<ResourceInfo>,
}

pub enum ResourceRequest {
    Initialize(ResourceInitInfo),
    Dispose,
}

pub enum ResourceSystemRequest<R: Runtime> {
    AddResource(OwnedResource<R>),
    Synchronize,
    Shutdown,
}

pub enum ResourceSystemResponse {
    SynchronizationComplete(Result<(), ResourceSystemError>),
}

pub async fn resource_system_main_task<S: ProcessSpawner, R: Runtime>(
    mut request_rx: UnboundedReceiver<ResourceSystemRequest<R>>,
    response_tx: UnboundedSender<ResourceSystemResponse>,
    mut owned_resources: Vec<OwnedResource<R>>,
    process_spawner: S,
    runtime: R,
    ownership_model: VmmOwnershipModel,
) {
    enum Incoming<R: Runtime> {
        SystemRequest(ResourceSystemRequest<R>),
        ResourceRequest(usize, ResourceRequest),
        InitTaskCompletion(usize, Result<ResourceInitInfo, ResourceSystemError>),
        DisposeTaskCompletion(usize, Result<(), ResourceSystemError>),
    }

    let mut synchronization_in_progress = false;
    let mut synchronization_errors = Vec::new();

    loop {
        let incoming = poll_fn(|cx| {
            for (resource_index, resource) in owned_resources.iter_mut().enumerate() {
                if let Poll::Ready(Some(request)) = resource.request_rx.poll_next_unpin(cx) {
                    return Poll::Ready(Incoming::ResourceRequest(resource_index, request));
                }

                if let Some(ref mut task) = resource.init_task {
                    if let Poll::Ready(Some(result)) = task.poll_join(cx) {
                        resource.init_task = None;
                        return Poll::Ready(Incoming::InitTaskCompletion(resource_index, result));
                    }
                } else if let Some(ref mut task) = resource.dispose_task {
                    if let Poll::Ready(Some(result)) = task.poll_join(cx) {
                        resource.dispose_task = None;
                        return Poll::Ready(Incoming::DisposeTaskCompletion(resource_index, result));
                    }
                }
            }

            if let Poll::Ready(Some(request)) = request_rx.poll_next_unpin(cx) {
                return Poll::Ready(Incoming::SystemRequest(request));
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
                    ResourceRequest::Initialize(init_info) => {
                        let init_task = runtime.spawn_task(resource_system_init_task(
                            resource.info.clone(),
                            init_info,
                            runtime.clone(),
                            process_spawner.clone(),
                            ownership_model,
                        ));

                        resource.init_task = Some(init_task);
                    }
                    ResourceRequest::Dispose => {
                        let dispose_task = runtime.spawn_task(resource_system_dispose_task(
                            resource.info.init_info.get().unwrap().clone(),
                            runtime.clone(),
                            process_spawner.clone(),
                            ownership_model,
                        ));

                        resource.dispose_task = Some(dispose_task);
                    }
                }
            }
            Incoming::InitTaskCompletion(resource_index, result) => {
                let Some(resource) = owned_resources.get_mut(resource_index) else {
                    continue;
                };

                match result {
                    Ok(init_info) => {
                        let _ = resource.info.init_info.set(Arc::new(init_info));
                    }
                    Err(err) => {
                        if synchronization_in_progress {
                            synchronization_errors.push(err);
                        }
                    }
                }
            }
            Incoming::DisposeTaskCompletion(resource_index, result) => {
                let Some(resource) = owned_resources.get_mut(resource_index) else {
                    continue;
                };

                match result {
                    Ok(_) => {
                        resource.info.disposed.store(true, Ordering::Release);
                    }
                    Err(err) => {
                        if synchronization_in_progress {
                            synchronization_errors.push(err);
                        }
                    }
                }
            }
        };

        if synchronization_in_progress {
            let no_pending_tasks = owned_resources
                .iter()
                .filter(|resource| resource.init_task.is_some() || resource.dispose_task.is_some())
                .next()
                .is_none();

            if no_pending_tasks {
                synchronization_in_progress = false;

                let result = match synchronization_errors.len() {
                    0 => Ok(()),
                    1 => Err(synchronization_errors
                        .pop()
                        .expect("synchronization_errors had length 1, but could not pop")),
                    _ => Err(ResourceSystemError::ErrorChain(
                        synchronization_errors.drain(..).collect(),
                    )),
                };

                let _ = response_tx.unbounded_send(ResourceSystemResponse::SynchronizationComplete(result));
            }
        }
    }
}

async fn resource_system_init_task<S: ProcessSpawner, R: Runtime>(
    info: Arc<ResourceInfo>,
    init_info: ResourceInitInfo,
    runtime: R,
    process_spawner: S,
    ownership_model: VmmOwnershipModel,
) -> Result<ResourceInitInfo, ResourceSystemError> {
    match info.r#type {
        ResourceType::Moved(moved_resource_type) => {
            if info.initial_path == init_info.effective_path {
                return Ok(init_info);
            }

            upgrade_owner(&info.initial_path, ownership_model, &process_spawner, &runtime)
                .await
                .map_err(ResourceSystemError::ChangeOwnerError)?;

            if !runtime
                .fs_exists(&info.initial_path)
                .await
                .map_err(ResourceSystemError::FilesystemError)?
            {
                return Err(ResourceSystemError::InitialPathMissing);
            }

            if let Some(parent_path) = init_info.effective_path.parent() {
                runtime
                    .fs_create_dir_all(parent_path)
                    .await
                    .map_err(ResourceSystemError::FilesystemError)?;
            }

            match moved_resource_type {
                MovedResourceType::Copied => {
                    runtime
                        .fs_copy(&info.initial_path, &init_info.effective_path)
                        .await
                        .map_err(ResourceSystemError::FilesystemError)?;
                }
                MovedResourceType::HardLinked => {
                    runtime
                        .fs_hard_link(&info.initial_path, &init_info.effective_path)
                        .await
                        .map_err(ResourceSystemError::FilesystemError)?;
                }
                MovedResourceType::CopiedOrHardLinked => {
                    if runtime
                        .fs_copy(&info.initial_path, &init_info.effective_path)
                        .await
                        .is_err()
                    {
                        runtime
                            .fs_hard_link(&info.initial_path, &init_info.effective_path)
                            .await
                            .map_err(ResourceSystemError::FilesystemError)?;
                    }
                }
                MovedResourceType::HardLinkedOrCopied => {
                    if runtime
                        .fs_hard_link(&info.initial_path, &init_info.effective_path)
                        .await
                        .is_err()
                    {
                        runtime
                            .fs_copy(&info.initial_path, &init_info.effective_path)
                            .await
                            .map_err(ResourceSystemError::FilesystemError)?;
                    }
                }
                MovedResourceType::Renamed => {
                    runtime
                        .fs_rename(&info.initial_path, &init_info.effective_path)
                        .await
                        .map_err(ResourceSystemError::FilesystemError)?;
                }
            }
        }
        ResourceType::Created(created_resource_type) => {
            if let Some(parent_path) = init_info.effective_path.parent() {
                runtime
                    .fs_create_dir_all(&parent_path)
                    .await
                    .map_err(ResourceSystemError::FilesystemError)?;
            }

            match created_resource_type {
                CreatedResourceType::File => {
                    runtime
                        .fs_create_file(&init_info.effective_path)
                        .await
                        .map_err(ResourceSystemError::FilesystemError)?;
                }
                CreatedResourceType::Fifo => {
                    crate::syscall::mkfifo(&init_info.effective_path).map_err(ResourceSystemError::FilesystemError)?;
                }
            }

            downgrade_owner(&init_info.effective_path, ownership_model)
                .map_err(ResourceSystemError::ChangeOwnerError)?;
        }
        ResourceType::Produced => {
            if let Some(parent_path) = init_info.effective_path.parent() {
                runtime
                    .fs_create_dir_all(&parent_path)
                    .await
                    .map_err(ResourceSystemError::FilesystemError)?;

                downgrade_owner(&parent_path, ownership_model).map_err(ResourceSystemError::ChangeOwnerError)?;
            }
        }
    };

    Ok(init_info)
}

async fn resource_system_dispose_task<R: Runtime, S: ProcessSpawner>(
    init_info: Arc<ResourceInitInfo>,
    runtime: R,
    process_spawner: S,
    ownership_model: VmmOwnershipModel,
) -> Result<(), ResourceSystemError> {
    upgrade_owner(&init_info.effective_path, ownership_model, &process_spawner, &runtime)
        .await
        .map_err(ResourceSystemError::ChangeOwnerError)?;

    runtime
        .fs_remove_file(&init_info.effective_path)
        .await
        .map_err(ResourceSystemError::FilesystemError)
}
