use std::{
    path::PathBuf,
    pin::pin,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, OnceLock,
    },
};

use async_broadcast::TryRecvError;
use futures_channel::mpsc;
use internal::{ResourceData, ResourceInitData, ResourcePull, ResourcePush};
use system::ResourceSystemError;

mod internal;

pub mod system;

/// A type that categorizes a [Resource] based on its relation to a Firecracker microVM environment:
/// created, moved or produced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceType {
    /// A created resource is a text file or a named (FIFO) pipe created by the fctools-utilizing application
    /// directly inside Firecracker's environment. For example, a Firecracker log or metrics file. The nature
    /// of the file is determined by the inner [CreatedResourceType].
    Created(CreatedResourceType),
    /// A moved resource is a pre-existing file, such as a rootfs or a kernel, which is moved according to the
    /// inner [MovedResourceType] into Firecracker's environment.
    Moved(MovedResourceType),
    /// A produced resource is a file that is created by Firecracker in order to be used by the fctools-utilizing
    /// application. For example, a snapshot state or memory file.
    Produced,
}

/// A [CreatedResourceType] determines whether a created resource is a plain-text file or a named pipe. In cases
/// such as a metrics file, both are allowed by Firecracker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CreatedResourceType {
    /// A plain-text file.
    File,
    /// A FIFO named pipe.
    Fifo,
}

/// A [MovedResourceType] determines what filesystem operation should be used in order to move the pre-existing
/// file into the Firecracker environment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MovedResourceType {
    /// Fully copy from source to destination (potentially slow).
    Copied,
    /// Hard link from source to destination (doesn't work in cross-device contexts).
    HardLinked,
    /// Try to first copy and then fall back to hard linking if copying fails.
    CopiedOrHardLinked,
    /// Try to first hard link and then fall back to copying if hard linking fails.
    HardLinkedOrCopied,
    /// Move/rename the source to the destination. This doesn't preserve the source at all, meaning it will be removed
    /// alongside the Firecracker environment after usage.
    Renamed,
}

/// The underlying state of a [Resource].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceState {
    /// The [Resource] is not initialized yet, meaning it has no effective or local path yet.
    Uninitialized,
    /// The [Resource] has been initialized, meaning its effective path (and its local path if it has one) now exists.
    Initialized,
    /// The [Resource] has been initialized and is now disposed, meaning its effective path likely no longer exists.
    Disposed,
}

impl std::fmt::Display for ResourceState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ResourceState::Uninitialized => write!(f, "Uninitialized"),
            ResourceState::Initialized => write!(f, "Initialized"),
            ResourceState::Disposed => write!(f, "Disposed"),
        }
    }
}

/// A [Resource] is resource pointer that communicates with an actual resource object owned by a resource system.
/// This pointer can be cloned moderately cheaply, and efficient synchronization primitives are internally employed
/// to make all operations accessible with only a shared reference. Two [Resource]s are always equal if they point
/// to the same object in the same resource system.
///
/// The [Resource]'s state includes its [ResourceType], [ResourceState] and three paths that characterize the file:
/// the source path, available in all [ResourceState]s, an effective path available after initialization and,
/// optionally, a local path also available post-initialization. The source path is the original path of the file
/// on disk, created by the fctools-utilizing application or by Firecracker, the effective path is the final absolute
/// path of the file that it ends up in after initialization, and the local path (when filled out) is the relative path
/// of the file after initialization, as which it is seen by Firecracker in a jailed environment, or the same absolute
/// path if no jailing takes place.
///
/// Some operations on a [Resource] are instant, while others may need to "poll" the [Resource] in order to ensure the
/// internally cached values are up-to-date. The performance cost of polling is as low as feasible: a mutex lock and
/// unlock that always doesn't need to wait and an atomic store if an update is needed.
///
/// The scheduling of resource initialization and disposal is the explicit responsibility of a VMM executor. The
/// appropriate [Resource] functions that perform scheduling manually should only be used if the VMM executor layer is
/// disabled or if you intend to create your own executor.
///
/// When the VM layer is enabled, a [Resource] implements serde's Serialize trait by serializing either its local path
/// for moved resources or its source path, and panics if either is inaccessible, so it is not safe to serialize an
/// uninitialized [Resource].
#[derive(Debug, Clone)]
pub struct Resource {
    pull_rx: async_broadcast::Receiver<ResourcePull>,
    shared: Arc<ResourceShared>,
}

#[derive(Debug)]
struct ResourceShared {
    push_tx: mpsc::UnboundedSender<ResourcePush>,
    data: Arc<ResourceData>,
    init_data: OnceLock<Arc<ResourceInitData>>,
    disposed: AtomicBool,
}

impl PartialEq for Resource {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.shared, &other.shared)
    }
}

impl Eq for Resource {}

impl Resource {
    /// Gets the current [ResourceState] of this [Resource]. This function may poll.
    #[inline]
    pub fn get_state(&mut self) -> ResourceState {
        if self.shared.disposed.load(Ordering::Acquire) {
            return ResourceState::Disposed;
        }

        match self.shared.init_data.get() {
            Some(_) => ResourceState::Initialized,
            None => {
                self.poll();
                match self.shared.init_data.get() {
                    Some(_) => ResourceState::Initialized,
                    None => ResourceState::Uninitialized,
                }
            }
        }
    }

    /// Get the [ResourceType] of this [Resource]. This function never polls.
    pub fn get_type(&self) -> ResourceType {
        self.shared.data.r#type
    }

    /// Get the source path as an owned [PathBuf] from this [Resource]. This function never polls.
    pub fn get_source_path(&self) -> PathBuf {
        self.shared.data.source_path.clone()
    }

    /// Get the effective path as an owned [PathBuf] from this [Resource], or [None] if the [Resource]
    /// is not yet initialized. This function always polls.
    pub fn get_effective_path(&mut self) -> Option<PathBuf> {
        self.poll();
        self.shared.init_data.get().map(|data| data.effective_path.clone())
    }

    /// Get the local path as an owned [PathBuf] from this [Resource], or [None] if the [Resource] is not
    /// yet initialized or hasn't been assigned a local path during its initialization. This function always
    /// polls.
    pub fn get_local_path(&mut self) -> Option<PathBuf> {
        self.poll();
        self.shared.init_data.get().and_then(|data| data.local_path.clone())
    }

    /// Schedule this [Resource] to be initialized by its system to the given effective and local paths.
    /// This operation doesn't actually wait for the initialization to occur. This function may poll.
    pub fn start_initialization(
        &mut self,
        effective_path: PathBuf,
        local_path: Option<PathBuf>,
    ) -> Result<(), ResourceSystemError> {
        self.assert_state(ResourceState::Uninitialized)?;

        self.shared
            .push_tx
            .unbounded_send(ResourcePush::Initialize(ResourceInitData {
                effective_path,
                local_path,
            }))
            .map_err(|_| ResourceSystemError::ChannelDisconnected)
    }

    /// Schedule this [Resource] to be initialized by its system to the same effective (and local, if the
    /// resource is moved) path as its source path. This operation doesn't actually wait for the initialization
    /// to occur. This function may poll.
    pub fn start_initialization_with_same_path(&mut self) -> Result<(), ResourceSystemError> {
        let local_path = match self.get_type() {
            ResourceType::Moved(_) => Some(self.get_source_path()),
            _ => None,
        };

        self.start_initialization(self.get_source_path(), local_path)
    }

    /// Schedule this [Resource] to be disposed by its system. This function may poll.
    pub fn start_disposal(&mut self) -> Result<(), ResourceSystemError> {
        self.assert_state(ResourceState::Initialized)?;
        let _ = self.shared.push_tx.unbounded_send(ResourcePush::Dispose);
        Ok(())
    }

    /// Schedule and await the initialization of this [Resource] to the given effective and local paths. This
    /// function may poll.
    pub async fn initialize(
        &mut self,
        effective_path: PathBuf,
        local_path: Option<PathBuf>,
    ) -> Result<(), ResourceSystemError> {
        self.start_initialization(effective_path, local_path)?;

        match pin!(self.pull_rx.recv_direct()).await {
            Ok(ResourcePull::Initialized(Ok(init_data))) => {
                let _ = self.shared.init_data.set(init_data);
                Ok(())
            }
            Ok(ResourcePull::Initialized(Err(err))) => Err(err),
            Ok(_) => Err(ResourceSystemError::MalformedResponse),
            Err(_) => Err(ResourceSystemError::ChannelDisconnected),
        }
    }

    /// Schedule and await the initialization of this [Resource] to the same effective (and local, if the
    /// resource is moved) path as its source path. This function may poll.
    pub async fn initialize_with_same_path(&mut self) -> Result<(), ResourceSystemError> {
        let local_path = match self.get_type() {
            ResourceType::Moved(_) => Some(self.get_source_path()),
            _ => None,
        };

        self.initialize(self.get_source_path(), local_path).await
    }

    /// Schedule and await the disposal of this [Resource]. This function may poll.
    pub async fn dispose(&mut self) -> Result<(), ResourceSystemError> {
        self.start_disposal()?;

        match pin!(self.pull_rx.recv_direct()).await {
            Ok(ResourcePull::Disposed(Ok(_))) => {
                self.shared.disposed.store(true, Ordering::Release);
                Ok(())
            }
            Ok(ResourcePull::Disposed(Err(err))) => Err(err),
            Ok(_) => Err(ResourceSystemError::MalformedResponse),
            Err(_) => Err(ResourceSystemError::ChannelDisconnected),
        }
    }

    #[inline(always)]
    fn poll(&mut self) {
        match self.pull_rx.try_recv() {
            Ok(ResourcePull::Disposed(Ok(_))) => {
                self.shared.disposed.store(true, Ordering::Release);
            }
            Err(TryRecvError::Closed) => {
                self.shared.disposed.store(true, Ordering::Release);
            }
            Ok(ResourcePull::Initialized(Ok(init_data))) => {
                let _ = self.shared.init_data.set(init_data);
            }
            _ => {}
        }
    }

    #[inline(always)]
    fn assert_state(&mut self, expected: ResourceState) -> Result<(), ResourceSystemError> {
        let actual = self.get_state();

        if actual != expected {
            Err(ResourceSystemError::IncorrectState(actual))
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
        match self.shared.data.r#type {
            ResourceType::Moved(_) => {
                self.clone().poll();
                self.shared
                    .init_data
                    .get()
                    .expect("called serialize on uninitialized resource")
                    .local_path
                    .as_ref()
                    .expect("no local_path for moved resource")
                    .serialize(serializer)
            }
            _ => self.shared.data.source_path.serialize(serializer),
        }
    }
}

#[derive(Debug)]
pub struct ResourceIterator(std::vec::IntoIter<Resource>);

impl ResourceIterator {
    #[inline]
    pub fn new(source: &[Resource]) -> Self {
        Self(source.to_vec().into_iter())
    }
}

impl From<&[Resource]> for ResourceIterator {
    fn from(value: &[Resource]) -> Self {
        Self::new(value)
    }
}

impl Iterator for ResourceIterator {
    type Item = Resource;

    fn next(&mut self) -> Option<Self::Item> {
        self.0.next()
    }
}
