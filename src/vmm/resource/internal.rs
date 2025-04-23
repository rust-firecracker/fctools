use std::{
    future::poll_fn,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, OnceLock,
    },
    task::Poll,
};

use futures_channel::mpsc;
use futures_util::StreamExt;

use crate::{
    process_spawner::ProcessSpawner,
    runtime::{Runtime, RuntimeTask},
    vmm::ownership::{downgrade_owner, upgrade_owner, VmmOwnershipModel},
};

use super::{system::ResourceSystemError, CreatedResourceType, MovedResourceType, ResourceType};

#[derive(Debug)]
pub struct ResourceInfo {
    pub request_tx: mpsc::UnboundedSender<ResourceRequest>,
    pub source_path: PathBuf,
    pub r#type: ResourceType,
    pub init_info: OnceLock<Arc<ResourceInitInfo>>,
    pub disposed: AtomicBool,
}

#[derive(Debug, Clone)]
pub struct ResourceInitInfo {
    pub effective_path: PathBuf,
    pub local_path: Option<PathBuf>,
}

pub struct OwnedResource<R: Runtime> {
    pub init_task: Option<R::Task<Result<ResourceInitInfo, ResourceSystemError>>>,
    pub dispose_task: Option<R::Task<Result<(), ResourceSystemError>>>,
    pub request_rx: mpsc::UnboundedReceiver<ResourceRequest>,
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
        InitTaskCompletion(usize, Result<ResourceInitInfo, ResourceSystemError>),
        DisposeTaskCompletion(usize, Result<(), ResourceSystemError>),
    }

    let mut synchronization_in_progress = false;

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
                        // TODO handle error
                        drop(err);
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
                        // TODO handle error
                        drop(err);
                    }
                }
            }
        };

        if synchronization_in_progress {
            let pending_tasks = owned_resources
                .iter()
                .filter(|resource| resource.init_task.is_some() || resource.dispose_task.is_some())
                .count();

            if pending_tasks == 0 {
                synchronization_in_progress = false;
                let _ = response_tx.unbounded_send(ResourceSystemResponse::SynchronizationComplete);
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
            if info.source_path == init_info.effective_path {
                return Ok(init_info);
            }

            upgrade_owner(&info.source_path, ownership_model, &process_spawner, &runtime)
                .await
                .map_err(|err| ResourceSystemError::ChangeOwnerError(Arc::new(err)))?;

            if !runtime
                .fs_exists(&info.source_path)
                .await
                .map_err(|err| ResourceSystemError::FilesystemError(Arc::new(err)))?
            {
                return Err(ResourceSystemError::SourcePathMissing);
            }

            if let Some(parent_path) = init_info.effective_path.parent() {
                runtime
                    .fs_create_dir_all(parent_path)
                    .await
                    .map_err(|err| ResourceSystemError::FilesystemError(Arc::new(err)))?;
            }

            match moved_resource_type {
                MovedResourceType::Copied => {
                    runtime
                        .fs_copy(&info.source_path, &init_info.effective_path)
                        .await
                        .map_err(|err| ResourceSystemError::FilesystemError(Arc::new(err)))?;
                }
                MovedResourceType::HardLinked => {
                    runtime
                        .fs_hard_link(&info.source_path, &init_info.effective_path)
                        .await
                        .map_err(|err| ResourceSystemError::FilesystemError(Arc::new(err)))?;
                }
                MovedResourceType::CopiedOrHardLinked => {
                    if runtime
                        .fs_copy(&info.source_path, &init_info.effective_path)
                        .await
                        .is_err()
                    {
                        runtime
                            .fs_hard_link(&info.source_path, &init_info.effective_path)
                            .await
                            .map_err(|err| ResourceSystemError::FilesystemError(Arc::new(err)))?;
                    }
                }
                MovedResourceType::HardLinkedOrCopied => {
                    if runtime
                        .fs_hard_link(&info.source_path, &init_info.effective_path)
                        .await
                        .is_err()
                    {
                        runtime
                            .fs_copy(&info.source_path, &init_info.effective_path)
                            .await
                            .map_err(|err| ResourceSystemError::FilesystemError(Arc::new(err)))?;
                    }
                }
                MovedResourceType::Renamed => {
                    runtime
                        .fs_rename(&info.source_path, &init_info.effective_path)
                        .await
                        .map_err(|err| ResourceSystemError::FilesystemError(Arc::new(err)))?;
                }
            }
        }
        ResourceType::Created(created_resource_type) => {
            if let Some(parent_path) = init_info.effective_path.parent() {
                runtime
                    .fs_create_dir_all(&parent_path)
                    .await
                    .map_err(|err| ResourceSystemError::FilesystemError(Arc::new(err)))?;
            }

            match created_resource_type {
                CreatedResourceType::File => {
                    runtime
                        .fs_create_file(&init_info.effective_path)
                        .await
                        .map_err(|err| ResourceSystemError::FilesystemError(Arc::new(err)))?;
                }
                CreatedResourceType::Fifo => {
                    crate::syscall::mkfifo(&init_info.effective_path)
                        .map_err(|err| ResourceSystemError::FilesystemError(Arc::new(err)))?;
                }
            }

            downgrade_owner(&init_info.effective_path, ownership_model)
                .map_err(|err| ResourceSystemError::ChangeOwnerError(Arc::new(err)))?;
        }
        ResourceType::Produced => {
            if let Some(parent_path) = init_info.effective_path.parent() {
                runtime
                    .fs_create_dir_all(&parent_path)
                    .await
                    .map_err(|err| ResourceSystemError::FilesystemError(Arc::new(err)))?;

                downgrade_owner(&parent_path, ownership_model)
                    .map_err(|err| ResourceSystemError::ChangeOwnerError(Arc::new(err)))?;
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
        .map_err(|err| ResourceSystemError::ChangeOwnerError(Arc::new(err)))?;

    runtime
        .fs_remove_file(&init_info.effective_path)
        .await
        .map_err(|err| ResourceSystemError::FilesystemError(Arc::new(err)))
}
