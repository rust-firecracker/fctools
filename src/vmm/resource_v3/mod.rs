use std::{
    ops::{Deref, DerefMut},
    path::PathBuf,
    sync::{Arc, OnceLock},
};

use futures_channel::mpsc;
use internal::{InternalResourceData, InternalResourceInitData, ResourcePull, ResourcePush};
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

#[derive(Clone)]
pub struct Resource {
    pub(super) push_tx: mpsc::UnboundedSender<ResourcePush>,
    pub(super) pull_rx: async_broadcast::Receiver<ResourcePull>,
    pub(super) data: Arc<InternalResourceData>,
    pub(super) init: OnceLock<(Arc<InternalResourceInitData>, Result<(), ResourceSystemError>)>,
    pub(super) dispose: OnceLock<Result<(), ResourceSystemError>>,
}

impl Resource {
    #[inline]
    pub fn get_state(&self) -> ResourceState {
        self.poll();

        if self.dispose.get().is_some() {
            return ResourceState::Disposed;
        }

        match self.init.get() {
            Some(_) => ResourceState::Initialized,
            None => ResourceState::Uninitialized,
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
        self.init.get().map(|(data, _)| data.effective_path.clone())
    }

    pub fn get_local_path(&self) -> Option<PathBuf> {
        self.poll();
        self.init.get().and_then(|(data, _)| data.local_path.clone())
    }

    pub fn start_initialization(
        &self,
        effective_path: PathBuf,
        local_path: Option<PathBuf>,
    ) -> Result<(), ResourceSystemError> {
        self.assert_state(ResourceState::Uninitialized)?;

        self.push_tx
            .unbounded_send(ResourcePush::Initialize(InternalResourceInitData {
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
        if let Ok(pull) = self.pull_rx.new_receiver().try_recv() {
            match pull {
                ResourcePull::Initialized { result, init_data } => {
                    let _ = self.init.set((init_data, result));
                }
                ResourcePull::Disposed(result) => {
                    let _ = self.dispose.set(result);
                }
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
