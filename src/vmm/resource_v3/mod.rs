use std::{path::PathBuf, sync::Arc};

use futures_channel::mpsc;
use system::ResourceSystemError;

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
    Initializing(R::Task<ResourceSystemError>),
    Initialized,
    Disposing(R::Task<ResourceSystemError>),
    Disposed,
}

struct Resource<R: Runtime> {
    state: ResourceState<R>,
    action_rx: mpsc::UnboundedReceiver<ResourceAction<R>>,
    init_tx: async_broadcast::Sender<Arc<ResourceInitData>>,
    disposed_tx: async_broadcast::Sender<()>,
    data: Arc<ResourceData>,
    init_data: Option<Arc<ResourceInitData>>,
}

struct ResourceData {
    path: PathBuf,
    r#type: ResourceType,
}

struct ResourceInitData {
    effective_path: PathBuf,
    local_path: Option<PathBuf>,
}

enum ResourceAction<R: Runtime> {
    Initialize {
        effective_path: PathBuf,
        local_path: Option<PathBuf>,
        runtime: R,
    },
    Dispose {
        runtime: R,
    },
}
