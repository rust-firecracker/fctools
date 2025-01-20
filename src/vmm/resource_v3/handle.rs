use std::{
    ops::Deref,
    path::PathBuf,
    pin::pin,
    sync::{Arc, OnceLock},
};

use futures_channel::mpsc;

use super::{system::ResourceError, ResourceData, ResourceInitData, ResourceRequest, ResourceResponse, ResourceType};

#[derive(Clone)]
pub struct MovedResourceHandle(pub(super) ResourceHandle);

impl Deref for MovedResourceHandle {
    type Target = ResourceHandle;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[derive(Clone)]
pub struct CreatedResourceHandle(pub(super) ResourceHandle);

impl Deref for CreatedResourceHandle {
    type Target = ResourceHandle;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[derive(Clone)]
pub struct ProducedResourceHandle(pub(super) ResourceHandle);

impl Deref for ProducedResourceHandle {
    type Target = ResourceHandle;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[derive(Clone)]
pub struct ResourceHandle {
    pub(super) request_tx: mpsc::UnboundedSender<ResourceRequest>,
    pub(super) response_rx: async_broadcast::Receiver<ResourceResponse>,
    pub(super) data: Arc<ResourceData>,
    pub(super) init_lock: OnceLock<(Arc<ResourceInitData>, Result<(), ResourceError>)>,
    pub(super) dispose_lock: OnceLock<Result<(), ResourceError>>,
}

impl ResourceHandle {
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
    ) -> Result<(), ResourceError> {
        self.assert_state(ResourceHandleState::Initialized)?;

        self.request_tx
            .unbounded_send(ResourceRequest::Initialize {
                effective_path,
                local_path,
            })
            .map_err(|_| ResourceError::ChannelDisconnected)
    }

    pub async fn initialize(&self, effective_path: PathBuf, local_path: Option<PathBuf>) -> Result<(), ResourceError> {
        self.begin_initialization(effective_path, local_path)?;

        match pin!(self.response_rx.new_receiver().recv_direct())
            .await
            .map_err(|_| ResourceError::ChannelDisconnected)?
        {
            ResourceResponse::Initialized { result, init_data } => {
                let _ = self.init_lock.set((init_data, result));
                Ok(())
            }
            _ => Err(ResourceError::IncorrectResponseReceived),
        }
    }

    pub async fn ping(&self) -> Result<(), ResourceError> {
        self.request_tx
            .unbounded_send(ResourceRequest::Ping)
            .map_err(|_| ResourceError::ChannelDisconnected)?;

        let response = self
            .response_rx
            .new_receiver()
            .recv()
            .await
            .map_err(|_| ResourceError::ChannelDisconnected)?;

        match response {
            ResourceResponse::Pong => Ok(()),
            _ => Err(ResourceError::IncorrectResponseReceived),
        }
    }

    #[inline(always)]
    fn assert_state(&self, expected: ResourceHandleState) -> Result<(), ResourceError> {
        let actual = self.get_state();

        if actual != expected {
            Err(ResourceError::IncorrectHandleState { expected, actual })
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
