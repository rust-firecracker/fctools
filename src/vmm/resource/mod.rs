use std::{
    path::{Path, PathBuf},
    sync::{Arc, atomic::Ordering},
};

use internal::{ResourceInfo, ResourceInitInfo, ResourceRequest};
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
    /// The [Resource] is not initialized yet, meaning it has no effective or virtual path yet.
    Uninitialized,
    /// The [Resource] has been initialized, meaning its effective path and virtual path now exist.
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
/// This pointer can be cloned cheaply with the same performance overhead as that of cloning a single [Arc], meaning
/// that cloning a [Resource] is essentially instant. Two [Resource]s are always equal if they point to the same object
/// in the same resource system.
///
/// The [Resource]'s state includes its [ResourceType], [ResourceState] and three paths that characterize the file:
/// the initial path, available in all [ResourceState]s, an effective and a virtual path available after initialization.
/// The initial path is the original path of the file on disk, created by the fctools-utilizing application or by
/// Firecracker, the effective path is the final path of the file on disk after initialization, and the virtual path
/// is the path of the file as seen by Firecracker. In the case of jailing, for instance, the effective and virtual
/// paths differ due to the virtual path being a relative path from inside the jail and the effective path being an
/// absolute path. But, when no jailing occurs, the effective and virtual paths are usually equivalent.
///
/// The scheduling of resource initialization and disposal is the explicit responsibility of a VMM executor. The
/// appropriate [Resource] functions that perform scheduling manually should only be used if the VMM executor layer is
/// disabled or if you intend to create your own executor.
///
/// When the VM layer is enabled, a [Resource] implements serde's Serialize trait by serializing either its virtual path
/// for moved resources or its initial path, and panics if either is inaccessible, so it is not safe to serialize an
/// uninitialized [Resource].
#[derive(Debug, Clone)]
pub struct Resource(Arc<ResourceInfo>);

impl PartialEq for Resource {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for Resource {}

impl Resource {
    /// Gets the current [ResourceState] of this [Resource].
    #[inline]
    pub fn get_state(&self) -> ResourceState {
        if self.0.disposed.load(Ordering::Acquire) {
            return ResourceState::Disposed;
        }

        match self.0.init_info.get() {
            Some(_) => ResourceState::Initialized,
            None => ResourceState::Uninitialized,
        }
    }

    /// Get the [ResourceType] of this [Resource].
    pub fn get_type(&self) -> ResourceType {
        self.0.r#type
    }

    /// Get the initial path as a borrowed [Path] from this [Resource].
    pub fn get_initial_path(&self) -> &Path {
        self.0.initial_path.as_path()
    }

    /// Get the effective path as a borrowed [PathBuf] from this [Resource], or [None] if the [Resource]
    /// has not yet been initialized.
    pub fn get_effective_path(&self) -> Option<&Path> {
        self.0.init_info.get().map(|data| data.effective_path.as_path())
    }

    /// Get the virtual path as a borrowed [Path] from this [Resource], or [None] if the [Resource] has not
    /// yet been initialized.
    pub fn get_virtual_path(&self) -> Option<&Path> {
        self.0
            .init_info
            .get()
            .map(|data| data.virtual_path.as_deref().unwrap_or_else(|| self.get_initial_path()))
    }

    /// Schedule this [Resource] to be initialized by its system to the given effective and virtual paths.
    /// If the virtual path is [None], it is assumed to be the same as the effective path. This operation
    /// doesn't actually wait for the initialization to occur.
    pub fn start_initialization(
        &self,
        effective_path: PathBuf,
        virtual_path: Option<PathBuf>,
    ) -> Result<(), ResourceSystemError> {
        self.assert_state(ResourceState::Uninitialized)?;

        self.0
            .request_tx
            .unbounded_send(ResourceRequest::Initialize(ResourceInitInfo {
                effective_path,
                virtual_path,
            }))
            .map_err(|_| ResourceSystemError::ChannelDisconnected)
    }

    /// Schedule this [Resource] to be initialized by its resource system to the same effective and
    /// virtual paths as its initial path. This doesn't wait for the initialization to occur.
    pub fn start_initialization_with_same_path(&self) -> Result<(), ResourceSystemError> {
        self.start_initialization(self.get_initial_path().to_owned(), None)
    }

    /// Schedule this [Resource] to be disposed by its resource system. This doesn't wait for the
    /// disposal to occur.
    pub fn start_disposal(&self) -> Result<(), ResourceSystemError> {
        self.assert_state(ResourceState::Initialized)?;
        let _ = self.0.request_tx.unbounded_send(ResourceRequest::Dispose);
        Ok(())
    }

    #[inline(always)]
    fn assert_state(&self, expected: ResourceState) -> Result<(), ResourceSystemError> {
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
        match self.0.r#type {
            ResourceType::Moved(_) => self
                .get_virtual_path()
                .expect("called serialize on uninitialized resource")
                .serialize(serializer),
            _ => self.get_initial_path().serialize(serializer),
        }
    }
}
