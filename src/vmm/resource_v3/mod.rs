use std::{path::PathBuf, sync::Arc};

use futures_channel::mpsc;
use system::ResourceError;

use crate::runtime::Runtime;

pub mod handle;
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

enum ResourceState<R: Runtime> {
    Uninitialized,
    Initializing(R::Task<ResourceError>),
    Initialized,
    Disposing(R::Task<ResourceError>),
    Disposed,
}

struct Resource<R: Runtime> {
    state: ResourceState<R>,
    request_rx: mpsc::UnboundedReceiver<ResourceRequest<R>>,
    response_tx: async_broadcast::Sender<ResourceResponse>,
    data: Arc<ResourceData>,
    init_data: Option<Arc<ResourceInitData>>,
}

struct ResourceData {
    source_path: PathBuf,
    r#type: ResourceType,
}

struct ResourceInitData {
    effective_path: PathBuf,
    local_path: Option<PathBuf>,
}

enum ResourceRequest<R: Runtime> {
    Initialize {
        effective_path: PathBuf,
        local_path: Option<PathBuf>,
        runtime: R,
    },
    Dispose {
        runtime: R,
    },
    Ping,
}

#[derive(Clone)]
enum ResourceResponse {
    Initialized {
        result: Result<(), ResourceError>,
        init_data: Arc<ResourceInitData>,
    },
    Disposed(Result<(), ResourceError>),
    Pong,
}
