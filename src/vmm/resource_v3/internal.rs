use std::{path::PathBuf, sync::Arc};

use futures_channel::mpsc;

use crate::runtime::Runtime;

use super::{system::ResourceSystemError, ResourceType};

pub enum OwnedResourceState<R: Runtime> {
    Uninitialized,
    Initializing(R::Task<ResourceSystemError>),
    Initialized,
    Disposing(R::Task<ResourceSystemError>),
    Disposed,
}

pub struct OwnedResource<R: Runtime> {
    pub state: OwnedResourceState<R>,
    pub request_rx: mpsc::UnboundedReceiver<ResourceRequest>,
    pub response_tx: async_broadcast::Sender<ResourceResponse>,
    pub data: Arc<ResourceData>,
    pub init_data: Option<Arc<ResourceInitData>>,
}

pub struct ResourceData {
    pub source_path: PathBuf,
    pub r#type: ResourceType,
}

pub struct ResourceInitData {
    pub effective_path: PathBuf,
    pub local_path: Option<PathBuf>,
}

pub enum ResourceRequest {
    Initialize {
        effective_path: PathBuf,
        local_path: Option<PathBuf>,
    },
    Dispose,
    Ping,
}

#[derive(Clone)]
pub enum ResourceResponse {
    Initialized {
        result: Result<(), ResourceSystemError>,
        init_data: Arc<ResourceInitData>,
    },
    Disposed(Result<(), ResourceSystemError>),
    Pong,
}
