use std::sync::Arc;

use futures_channel::mpsc;

use crate::runtime::Runtime;

use super::{ResourceAction, ResourceData, ResourceInitData};

#[derive(Clone)]
pub struct ResourceHandle<R: Runtime> {
    action_tx: mpsc::UnboundedSender<ResourceAction<R>>,
    init_rx: async_broadcast::Receiver<Arc<ResourceInitData>>,
    disposed_rx: async_broadcast::Receiver<()>,
    data: Arc<ResourceData>,
    init_data: Option<Arc<ResourceInitData>>,
    disposed: bool,
}

impl<R: Runtime> ResourceHandle<R> {
    #[inline]
    pub fn poll(&mut self) -> ResourceHandleState {
        if self.disposed {
            return ResourceHandleState::Disposed;
        }

        if let Ok(_) = self.disposed_rx.try_recv() {
            self.disposed = true;
            return ResourceHandleState::Disposed;
        }

        self.poll_init_data();

        match self.init_data {
            Some(_) => ResourceHandleState::Initialized,
            None => ResourceHandleState::Uninitialized,
        }
    }

    #[inline(always)]
    fn poll_init_data(&mut self) {
        if self.init_data.is_none() {
            if let Ok(init_data) = self.init_rx.try_recv() {
                self.init_data = Some(init_data);
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
