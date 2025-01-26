use std::{
    ops::Deref,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, OnceLock,
    },
};

use async_broadcast::TryRecvError;
use futures_channel::mpsc;
use internal::{ResourceData, ResourceInitData, ResourcePull, ResourcePush};
use system::ResourceSystemError;

mod internal;

pub mod system;

#[derive(Debug, Clone, Copy)]
pub enum ResourceType {
    Created(CreatedResourceType),
    Moved(MovedResourceType),
    Produced,
}

#[derive(Debug, Clone, Copy)]
pub enum CreatedResourceType {
    File,
    Fifo,
}

#[derive(Debug, Clone, Copy)]
pub enum MovedResourceType {
    Copied,
    HardLinked,
    CopiedOrHardLinked,
    HardLinkedOrCopied,
    Renamed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "vm", derive(serde::Serialize))]
pub struct MovedResource(Resource);

impl Deref for MovedResource {
    type Target = Resource;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl MovedResource {
    pub fn into_inner(self) -> Resource {
        self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "vm", derive(serde::Serialize))]
pub struct CreatedResource(Resource);

impl Deref for CreatedResource {
    type Target = Resource;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl CreatedResource {
    pub fn into_inner(self) -> Resource {
        self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "vm", derive(serde::Serialize))]
pub struct ProducedResource(Resource);

impl Deref for ProducedResource {
    type Target = Resource;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl ProducedResource {
    pub fn into_inner(self) -> Resource {
        self.0
    }
}

#[derive(Debug)]
pub struct Resource {
    push_tx: mpsc::UnboundedSender<ResourcePush>,
    pull_rx: Mutex<async_broadcast::Receiver<ResourcePull>>,
    data: Arc<ResourceData>,
    init_data: OnceLock<Arc<ResourceInitData>>,
    disposed: Arc<AtomicBool>,
}

impl PartialEq for Resource {
    fn eq(&self, other: &Self) -> bool {
        self.push_tx.same_receiver(&other.push_tx)
    }
}

impl Eq for Resource {}

impl Clone for Resource {
    fn clone(&self) -> Self {
        Self {
            push_tx: self.push_tx.clone(),
            pull_rx: Mutex::new(self.pull_rx.lock().expect("pull_rx mutex was poisoned").clone()),
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

        if !self.data.attached.load(Ordering::Acquire) {
            return ResourceState::Detached;
        }

        if self.disposed.load(Ordering::Acquire) {
            return ResourceState::Disposed;
        }

        match self.init_data.get() {
            Some(_) => ResourceState::Initialized,
            None => ResourceState::Uninitialized,
        }
    }

    pub fn is_attached(&self) -> bool {
        self.data.attached.load(Ordering::Acquire)
    }

    pub fn detach(&self) {
        self.data.attached.store(false, Ordering::Release);
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

    pub fn start_initialization_with_same_path(&self) -> Result<(), ResourceSystemError> {
        let local_path = match self.get_type() {
            ResourceType::Moved(_) => Some(self.get_source_path()),
            _ => None,
        };

        self.start_initialization(self.get_source_path(), local_path)
    }

    pub fn start_disposal(&self) -> Result<(), ResourceSystemError> {
        self.assert_state(ResourceState::Initialized)?;

        self.push_tx
            .unbounded_send(ResourcePush::Dispose)
            .map_err(|_| ResourceSystemError::ChannelDisconnected)
    }

    #[inline(always)]
    fn poll(&self) {
        match self.pull_rx.lock().expect("pull_rx mutex was poisoned").try_recv() {
            Ok(ResourcePull::Disposed(Ok(_))) | Err(TryRecvError::Closed) => {
                self.disposed.store(true, Ordering::Release);
            }
            Ok(ResourcePull::Initialized(Ok(init_data))) => {
                let _ = self.init_data.set(init_data);
            }
            _ => {}
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

#[cfg(feature = "vm")]
#[cfg_attr(docsrs, doc(cfg(feature = "vm")))]
impl serde::Serialize for Resource {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.init_data
            .get()
            .expect("called serialize on uninitialized resource")
            .effective_path
            .serialize(serializer)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceState {
    Uninitialized,
    Initialized,
    Disposed,
    Detached,
}

impl std::fmt::Display for ResourceState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ResourceState::Uninitialized => write!(f, "Uninitialized"),
            ResourceState::Initialized => write!(f, "Initialized"),
            ResourceState::Disposed => write!(f, "Disposed"),
            ResourceState::Detached => write!(f, "Detached"),
        }
    }
}
