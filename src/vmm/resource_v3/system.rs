use std::{
    path::PathBuf,
    sync::{Arc, Weak},
};

use futures_channel::mpsc;

use crate::runtime::Runtime;

use super::ResourceType;

pub struct ResourceSystem<R: Runtime> {
    resources: Vec<Resource<R>>,
}

impl<R: Runtime> ResourceSystem<R> {
    pub fn add_created_resource() {}
}

pub enum ResourceSystemError {}

pub(super) enum ResourceState<R: Runtime> {
    Uninitialized,
    Initializing(R::Task<ResourceSystemError>),
    Initialized,
    Disposing(R::Task<ResourceSystemError>),
    Disposed,
}

pub(super) struct Resource<R: Runtime> {
    status: ResourceState<R>,
    action_rx: mpsc::UnboundedReceiver<()>,
    init_tx: async_broadcast::Sender<Weak<ResourceInitInfo>>,
    base_info: Arc<ResourceBaseInfo>,
    init_info: Option<Arc<ResourceInitInfo>>,
}

pub(super) struct ResourceBaseInfo {
    path: PathBuf,
    r#type: ResourceType,
}

pub(super) struct ResourceInitInfo {
    effective_path: PathBuf,
    local_path: Option<PathBuf>,
}
