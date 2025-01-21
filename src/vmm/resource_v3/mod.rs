use std::{
    ops::Deref,
    path::PathBuf,
    pin::pin,
    sync::{Arc, OnceLock},
};

use futures_channel::mpsc;
use internal::{InternalResourceData, InternalResourceInitData, ResourceRequest, ResourceResponse};
use system::ResourceSystemError;

pub mod internal;
pub mod system;

#[derive(Clone, Copy)]
pub enum ResourceType {
    Created(CreatedResourceType),
    Moved(MovedResourceType),
    Produced,
}

#[derive(Clone, Copy)]
pub enum CreatedResourceType {
    File,
    Fifo,
}

#[derive(Clone, Copy)]
pub enum MovedResourceType {
    Copied,
    HardLinked,
    CopiedOrHardLinked,
    HardLinkedOrCopied,
    Renamed,
}

#[derive(Clone)]
pub struct MovedResource(pub(super) Resource);

impl Deref for MovedResource {
    type Target = Resource;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[derive(Clone)]
pub struct CreatedResource(pub(super) Resource);

impl Deref for CreatedResource {
    type Target = Resource;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[derive(Clone)]
pub struct ProducedResource(pub(super) Resource);

impl Deref for ProducedResource {
    type Target = Resource;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[derive(Clone)]
pub struct Resource {
    pub(super) request_tx: mpsc::UnboundedSender<ResourceRequest>,
    pub(super) response_rx: async_broadcast::Receiver<ResourceResponse>,
    pub(super) data: Arc<InternalResourceData>,
    pub(super) init_lock: OnceLock<(Arc<InternalResourceInitData>, Result<(), ResourceSystemError>)>,
    pub(super) dispose_lock: OnceLock<Result<(), ResourceSystemError>>,
}

impl Resource {
    #[inline]
    pub fn get_state(&self) -> ResourceHandleState {
        self.poll();

        if self.dispose_lock.get().is_some() {
            return ResourceHandleState::Disposed;
        }

        match self.init_lock.get() {
            Some(_) => ResourceHandleState::Initialized,
            None => ResourceHandleState::Uninitialized,
        }
    }

    pub fn get_type(&self) -> ResourceType {
        self.data.r#type
    }

    pub fn get_source_path(&self) -> PathBuf {
        self.data.source_path.clone()
    }

    pub fn get_effective_path(&self) -> Option<PathBuf> {
        self.poll();
        self.init_lock.get().map(|(data, _)| data.effective_path.clone())
    }

    pub fn get_local_path(&self) -> Option<PathBuf> {
        self.poll();
        self.init_lock.get().and_then(|(data, _)| data.local_path.clone())
    }

    pub fn begin_initialization(
        &self,
        effective_path: PathBuf,
        local_path: Option<PathBuf>,
    ) -> Result<(), ResourceSystemError> {
        self.assert_state(ResourceHandleState::Uninitialized)?;

        self.request_tx
            .unbounded_send(ResourceRequest::Initialize(InternalResourceInitData {
                effective_path,
                local_path,
            }))
            .map_err(|_| ResourceSystemError::ChannelDisconnected)
    }

    pub async fn initialize(
        &self,
        effective_path: PathBuf,
        local_path: Option<PathBuf>,
    ) -> Result<(), ResourceSystemError> {
        self.begin_initialization(effective_path, local_path)?;

        match pin!(self.response_rx.new_receiver().recv_direct())
            .await
            .map_err(|_| ResourceSystemError::ChannelDisconnected)?
        {
            ResourceResponse::Initialized { result, init_data } => {
                let _ = self.init_lock.set((init_data, result));
                result?;
                Ok(())
            }
            _ => Err(ResourceSystemError::IncorrectResponseReceived),
        }
    }

    pub fn begin_disposal(&self) -> Result<(), ResourceSystemError> {
        self.assert_state(ResourceHandleState::Initialized)?;

        self.request_tx
            .unbounded_send(ResourceRequest::Dispose)
            .map_err(|_| ResourceSystemError::ChannelDisconnected)
    }

    pub async fn dispose(&self) -> Result<(), ResourceSystemError> {
        self.begin_disposal()?;

        match pin!(self.response_rx.new_receiver().recv_direct())
            .await
            .map_err(|_| ResourceSystemError::ChannelDisconnected)?
        {
            ResourceResponse::Disposed(result) => {
                let _ = self.dispose_lock.set(result);
                result?;
                Ok(())
            }
            _ => Err(ResourceSystemError::IncorrectResponseReceived),
        }
    }

    pub async fn ping(&self) -> Result<(), ResourceSystemError> {
        self.request_tx
            .unbounded_send(ResourceRequest::Ping)
            .map_err(|_| ResourceSystemError::ChannelDisconnected)?;

        let response = self
            .response_rx
            .new_receiver()
            .recv()
            .await
            .map_err(|_| ResourceSystemError::ChannelDisconnected)?;

        match response {
            ResourceResponse::Pong => Ok(()),
            _ => Err(ResourceSystemError::IncorrectResponseReceived),
        }
    }

    #[inline(always)]
    fn assert_state(&self, expected: ResourceHandleState) -> Result<(), ResourceSystemError> {
        let actual = self.get_state();

        if actual != expected {
            Err(ResourceSystemError::IncorrectHandleState { expected, actual })
        } else {
            Ok(())
        }
    }

    #[inline(always)]
    fn poll(&self) {
        if let Ok(response) = self.response_rx.new_receiver().try_recv() {
            match response {
                ResourceResponse::Initialized { result, init_data } => {
                    let _ = self.init_lock.set((init_data, result));
                }
                ResourceResponse::Disposed(result) => {
                    let _ = self.dispose_lock.set(result);
                }
                _ => {}
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceHandleState {
    Uninitialized,
    Initialized,
    Disposed,
}
