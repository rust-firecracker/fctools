use std::{
    ops::Deref,
    path::PathBuf,
    sync::{Arc, OnceLock},
};

use futures_channel::mpsc;

use crate::runtime::Runtime;

use super::{system::ResourceError, ResourceData, ResourceInitData, ResourceRequest, ResourceResponse, ResourceType};

pub struct MovedResourceHandle<R: Runtime>(ResourceHandle<R>);

impl<R: Runtime> Deref for MovedResourceHandle<R> {
    type Target = ResourceHandle<R>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

pub struct CreatedResourceHandle<R: Runtime>(ResourceHandle<R>);

impl<R: Runtime> Deref for CreatedResourceHandle<R> {
    type Target = ResourceHandle<R>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

pub struct ProducedResourceHandle<R: Runtime>(ResourceHandle<R>);

impl<R: Runtime> Deref for ProducedResourceHandle<R> {
    type Target = ResourceHandle<R>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[derive(Clone)]
pub struct ResourceHandle<R: Runtime> {
    request_tx: mpsc::UnboundedSender<ResourceRequest<R>>,
    response_rx: async_broadcast::Receiver<ResourceResponse>,
    data: Arc<ResourceData>,
    init_lock: OnceLock<(Arc<ResourceInitData>, Result<(), ResourceError>)>,
    dispose_lock: OnceLock<Result<(), ResourceError>>,
}

impl<R: Runtime> ResourceHandle<R> {
    #[inline]
    pub fn get_state(&self) -> ResourceHandleState {
        self.poll_responses();

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
        self.poll_responses();
        self.init_lock.get().map(|(data, _)| data.effective_path.clone())
    }

    pub fn get_local_path(&self) -> Option<PathBuf> {
        self.poll_responses();
        self.init_lock.get().and_then(|(data, _)| data.local_path.clone())
    }

    pub async fn initialize(
        &self,
        runtime: R,
        effective_path: PathBuf,
        local_path: Option<PathBuf>,
    ) -> Result<(), ResourceError> {
        Ok(())
    }

    pub async fn ping(&self) -> bool {
        if self.request_tx.unbounded_send(ResourceRequest::Ping).is_err() {
            return false;
        }

        let response = self.response_rx.new_receiver().recv().await;
        matches!(response, Ok(ResourceResponse::Pong))
    }

    #[inline(always)]
    fn poll_responses(&self) {
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

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ResourceHandleState {
    Uninitialized,
    Initialized,
    Disposed,
}
