use std::{
    ops::{Deref, DerefMut},
    path::PathBuf,
    sync::Arc,
};

use bus::{Bus, BusClient};
use internal::{InternalResourceData, InternalResourceInitData, ResourceRequest, ResourceResponse};
use system::ResourceSystemError;

mod internal;

pub mod bus;
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
pub struct MovedResource<B: Bus>(pub(super) Resource<B>);

impl<B: Bus> Deref for MovedResource<B> {
    type Target = Resource<B>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<B: Bus> DerefMut for MovedResource<B> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

#[derive(Clone)]
pub struct CreatedResource<B: Bus>(pub(super) Resource<B>);

impl<B: Bus> Deref for CreatedResource<B> {
    type Target = Resource<B>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<B: Bus> DerefMut for CreatedResource<B> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

#[derive(Clone)]
pub struct ProducedResource<B: Bus>(pub(super) Resource<B>);

impl<B: Bus> Deref for ProducedResource<B> {
    type Target = Resource<B>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<B: Bus> DerefMut for ProducedResource<B> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

#[derive(Clone)]
pub struct Resource<B: Bus> {
    pub(super) bus_client: B::Client<ResourceRequest, ResourceResponse>,
    pub(super) data: Arc<InternalResourceData>,
    pub(super) init: Option<(Arc<InternalResourceInitData>, Result<(), ResourceSystemError>)>,
    pub(super) dispose: Option<Result<(), ResourceSystemError>>,
}

impl<B: Bus> Resource<B> {
    #[inline]
    pub fn get_state(&mut self) -> ResourceState {
        self.poll();

        if self.dispose.is_some() {
            return ResourceState::Disposed;
        }

        match self.init {
            Some(_) => ResourceState::Initialized,
            None => ResourceState::Uninitialized,
        }
    }

    pub fn get_type(&mut self) -> ResourceType {
        self.data.r#type
    }

    pub fn get_source_path(&mut self) -> PathBuf {
        self.data.source_path.clone()
    }

    pub fn get_effective_path(&mut self) -> Option<PathBuf> {
        self.poll();
        self.init.as_ref().map(|(data, _)| data.effective_path.clone())
    }

    pub fn get_local_path(&mut self) -> Option<PathBuf> {
        self.poll();
        self.init.as_ref().and_then(|(data, _)| data.local_path.clone())
    }

    pub fn start_initialization(
        &mut self,
        effective_path: PathBuf,
        local_path: Option<PathBuf>,
    ) -> Result<(), ResourceSystemError> {
        self.assert_state(ResourceState::Uninitialized)?;

        match self
            .bus_client
            .send_request(ResourceRequest::Initialize(InternalResourceInitData {
                effective_path,
                local_path,
            })) {
            true => Ok(()),
            false => Err(ResourceSystemError::BusDisconnected),
        }
    }

    pub async fn initialize(
        &mut self,
        effective_path: PathBuf,
        local_path: Option<PathBuf>,
    ) -> Result<(), ResourceSystemError> {
        self.assert_state(ResourceState::Uninitialized)?;

        match self
            .bus_client
            .make_request(ResourceRequest::Initialize(InternalResourceInitData {
                effective_path,
                local_path,
            }))
            .await
        {
            Some(ResourceResponse::Initialized { result, init_data }) => {
                self.init = Some((init_data, result));
                result?;
                Ok(())
            }
            Some(_) => Err(ResourceSystemError::MalformedResponse),
            None => Err(ResourceSystemError::BusDisconnected),
        }
    }

    pub fn start_disposal(&mut self) -> Result<(), ResourceSystemError> {
        self.assert_state(ResourceState::Initialized)?;

        match self.bus_client.send_request(ResourceRequest::Dispose) {
            true => Ok(()),
            false => Err(ResourceSystemError::BusDisconnected),
        }
    }

    pub async fn dispose(&mut self) -> Result<(), ResourceSystemError> {
        self.assert_state(ResourceState::Initialized)?;

        match self.bus_client.make_request(ResourceRequest::Dispose).await {
            Some(ResourceResponse::Disposed(result)) => {
                self.dispose = Some(result);
                result?;
                Ok(())
            }
            Some(_) => Err(ResourceSystemError::MalformedResponse),
            None => Err(ResourceSystemError::BusDisconnected),
        }
    }

    #[inline(always)]
    fn poll(&mut self) {
        if let Some(response) = self.bus_client.try_get_response() {
            match response {
                ResourceResponse::Initialized { result, init_data } => {
                    self.init = Some((init_data, result));
                }
                ResourceResponse::Disposed(result) => {
                    self.dispose = Some(result);
                }
            }
        }
    }

    #[inline(always)]
    fn assert_state(&mut self, expected: ResourceState) -> Result<(), ResourceSystemError> {
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
