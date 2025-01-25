use std::{
    ops::{Deref, DerefMut},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, OnceLock,
    },
};

use futures_channel::mpsc;
use internal::{ResourceData, ResourceInitData, ResourcePull, ResourcePush};
use system::ResourceSystemError;

mod internal;

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

impl DerefMut for MovedResource {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
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

impl DerefMut for CreatedResource {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
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

impl DerefMut for ProducedResource {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

pub struct Resource {
    pub(super) push_tx: mpsc::UnboundedSender<ResourcePush>,
    pub(super) pull_rx: Mutex<async_broadcast::Receiver<ResourcePull>>,
    pub(super) data: Arc<ResourceData>,
    pub(super) init_data: OnceLock<Arc<ResourceInitData>>,
    pub(super) disposed: Arc<AtomicBool>,
}

impl Clone for Resource {
    fn clone(&self) -> Self {
        Self {
            push_tx: self.push_tx.clone(),
            pull_rx: Mutex::new(self.pull_rx.lock().unwrap().clone()),
            data: self.data.clone(),
            init_data: self.init_data.clone(),
            disposed: self.disposed.clone(),
        }
    }
}

impl Resource {
    #[inline]
    pub fn get_state(&self) -> ResourceState {
        self.poll();

        if self.disposed.load(Ordering::Acquire) {
            return ResourceState::Disposed;
        }

        match self.init_data.get() {
            Some(_) => ResourceState::Initialized,
            None => ResourceState::Uninitialized,
        }
    }

    pub fn is_linked(&self) -> bool {
        self.data.linked.load(Ordering::Acquire)
    }

    pub fn unlink(&self) {
        self.data.linked.store(false, Ordering::Release);
    }

    pub fn get_type(&self) -> ResourceType {
        self.data.r#type
    }

    pub fn get_source_path(&self) -> PathBuf {
        self.data.source_path.clone()
    }

    pub fn get_effective_path(&self) -> Option<PathBuf> {
        self.poll();
        self.init_data.get().map(|data| data.effective_path.clone())
    }

    pub fn get_local_path(&self) -> Option<PathBuf> {
        self.poll();
        self.init_data.get().and_then(|data| data.local_path.clone())
    }

    pub fn start_initialization(
        &self,
        effective_path: PathBuf,
        local_path: Option<PathBuf>,
    ) -> Result<(), ResourceSystemError> {
        self.assert_state(ResourceState::Uninitialized)?;

        self.push_tx
            .unbounded_send(ResourcePush::Initialize(ResourceInitData {
                effective_path,
                local_path,
            }))
            .map_err(|_| ResourceSystemError::ChannelDisconnected)
    }

    pub fn start_disposal(&self) -> Result<(), ResourceSystemError> {
        self.assert_state(ResourceState::Initialized)?;

        self.push_tx
            .unbounded_send(ResourcePush::Dispose)
            .map_err(|_| ResourceSystemError::ChannelDisconnected)
    }

    #[inline(always)]
    fn poll(&self) {
        if let Ok(pull) = self.pull_rx.lock().unwrap().try_recv() {
            match pull {
                ResourcePull::Initialized(Ok(init_data)) => {
                    let _ = self.init_data.set(init_data);
                }
                ResourcePull::Disposed(Ok(_)) => {
                    self.disposed.store(true, Ordering::Release);
                }
                _ => {}
            }
        }
    }

    #[inline(always)]
    fn assert_state(&self, expected: ResourceState) -> Result<(), ResourceSystemError> {
        let actual = self.get_state();

        if actual != expected {
            Err(ResourceSystemError::IncorrectState { expected, actual })
        } else {
            Ok(())
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceState {
    Uninitialized,
    Initialized,
    Disposed,
}
