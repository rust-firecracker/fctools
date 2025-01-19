use std::sync::{OnceLock, Weak};

use super::system::{ResourceBaseInfo, ResourceInitInfo};

pub struct ResourcePointer {
    action_tx: futures_channel::mpsc::UnboundedSender<()>,
    init_rx: async_broadcast::Receiver<Weak<ResourceInitInfo>>,
    base_info: Weak<ResourceBaseInfo>,
    init_info: OnceLock<Weak<ResourceInitInfo>>,
}

impl ResourcePointer {
    #[inline]
    pub fn poll(&self) -> ResourcePointerState {
        if self.base_info.strong_count() == 0 {
            return ResourcePointerState::Disposed;
        }

        if let Ok(init_info) = self.init_rx.clone().try_recv() {
            let _ = self.init_info.set(init_info);
        }

        if self.init_info.get().map_or(false, |ptr| ptr.strong_count() > 0) {
            ResourcePointerState::Initialized
        } else {
            ResourcePointerState::Uninitialized
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ResourcePointerState {
    Uninitialized,
    Initialized,
    Disposed,
}
