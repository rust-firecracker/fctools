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
    pub push_rx: mpsc::UnboundedReceiver<ResourcePush>,
    pub pull_tx: async_broadcast::Sender<ResourcePull>,
    pub data: Arc<ResourceData>,
    pub init_data: Option<Arc<ResourceInitData>>,
    pub linked: bool,
}

pub struct ResourceData {
    pub source_path: PathBuf,
    pub r#type: ResourceType,
}

pub struct ResourceInitData {
    pub effective_path: PathBuf,
    pub local_path: Option<PathBuf>,
}

pub enum ResourcePush {
    Initialize(ResourceInitData),
    Dispose,
    Unlink,
}

#[derive(Clone)]
pub enum ResourcePull {
    Initialized(Result<Arc<ResourceInitData>, ResourceSystemError>),
    Disposed(Result<(), ResourceSystemError>),
}

pub enum ResourceSystemPush<R: Runtime> {
    AddResource(OwnedResource<R>),
    Shutdown,
}

pub enum ResourceSystemPull {
    ShutdownFinished,
}

pub async fn resource_system_main_task<S: ProcessSpawner, R: Runtime>(
    mut push_rx: mpsc::UnboundedReceiver<ResourceSystemPush<R>>,
    pull_tx: mpsc::UnboundedSender<ResourceSystemPull>,
    mut owned_resources: Vec<OwnedResource<R>>,
    process_spawner: S,
    runtime: R,
    ownership_model: VmmOwnershipModel,
) {
    enum Incoming<R: Runtime> {
        SystemPush(ResourceSystemPush<R>),
        ResourcePush(usize, ResourcePush),
        FinishedInitTask(usize, Result<ResourceInitData, ResourceSystemError>),
        FinishedDisposeTask(usize, Result<(), ResourceSystemError>),
    }

    loop {
        let incoming = poll_fn(|cx| {
            if let Poll::Ready(Some(push)) = push_rx.poll_next_unpin(cx) {
                return Poll::Ready(Incoming::SystemPush(push));
            }

            for (resource_index, resource) in owned_resources.iter_mut().enumerate() {
                if let Poll::Ready(Some(push)) = resource.push_rx.poll_next_unpin(cx) {
                    return Poll::Ready(Incoming::ResourcePush(resource_index, push));
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

            Poll::Pending
        })
        .await;

        match incoming {
            Incoming::SystemPush(push) => match push {
                ResourceSystemPush::AddResource(internal_resource) => {
                    owned_resources.push(internal_resource);
                }
                ResourceSystemPush::Shutdown => {
                    let _ = pull_tx.unbounded_send(ResourceSystemPull::ShutdownFinished);
                    return;
                }
            },
            Incoming::ResourcePush(resource_index, push) => {
                let Some(resource) = owned_resources.get_mut(resource_index) else {
                    continue;
                };

                match push {
                    ResourcePush::Initialize(init_data) => {
                        let init_task = runtime.spawn_task(resource_system_init_task(
                            resource.data.clone(),
                            init_data,
                            runtime.clone(),
                            process_spawner.clone(),
                            ownership_model,
                        ));

                        resource.state = OwnedResourceState::Initializing(init_task);
                    }
                    ResourcePush::Dispose => {
                        let dispose_task = runtime.spawn_task(resource_system_dispose_task(
                            resource.data.clone(),
                            resource.init_data.clone().unwrap(),
                            runtime.clone(),
                            process_spawner.clone(),
                            ownership_model,
                        ));

                        resource.state = OwnedResourceState::Disposing(dispose_task);
                    }
                    ResourcePush::Unlink => {
                        resource.linked = false;
                    }
                }
            }
            Incoming::FinishedInitTask(resource_index, result) => {
                let Some(resource) = owned_resources.get_mut(resource_index) else {
                    continue;
                };

                match result {
                    Ok(init_data) => {
                        resource.state = OwnedResourceState::Initialized;
                        let _ = pin!(resource
                            .pull_tx
                            .broadcast_direct(ResourcePull::Initialized(Ok(Arc::new(init_data)))))
                        .await;
                    }
                    Err(err) => {
                        resource.state = OwnedResourceState::Uninitialized;
                        let _ = pin!(resource.pull_tx.broadcast_direct(ResourcePull::Initialized(Err(err)))).await;
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
                        let _ = pin!(resource.pull_tx.broadcast_direct(ResourcePull::Disposed(Ok(())))).await;
                    }
                    Err(err) => {
                        resource.state = OwnedResourceState::Initialized;
                        let _ = pin!(resource.pull_tx.broadcast_direct(ResourcePull::Disposed(Err(err)))).await;
                    }
                }
            }
        }
    }
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
    data: Arc<ResourceData>,
    init_data: Arc<ResourceInitData>,
    runtime: R,
    process_spawner: S,
    ownership_model: VmmOwnershipModel,
) -> Result<(), ResourceSystemError> {
    Ok(())
}
