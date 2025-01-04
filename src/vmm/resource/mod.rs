use std::path::PathBuf;

use created::CreatedVmmResource;
use moved::MovedVmmResource;
use produced::ProducedVmmResource;

use super::ownership::ChangeOwnerError;

pub mod created;
pub mod moved;
pub mod produced;

/// An error that can be produced by an operation on a VMM resource.
#[derive(Debug)]
pub enum VmmResourceError {
    FilesystemError(std::io::Error),
    MkfifoError(std::io::Error),
    ChangeOwnerError(ChangeOwnerError),
    SourcePathMissing(PathBuf),
}

impl std::error::Error for VmmResourceError {}

impl std::fmt::Display for VmmResourceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VmmResourceError::FilesystemError(err) => {
                write!(f, "A filesystem operation backed by the runtime failed: {err}")
            }
            VmmResourceError::MkfifoError(err) => write!(f, "Creating a named pipe via mkfifo failed: {err}"),
            VmmResourceError::ChangeOwnerError(err) => write!(f, "An ownership change failed: {err}"),
            VmmResourceError::SourcePathMissing(path) => {
                write!(f, "The source path of a resource is missing: {}", path.display())
            }
        }
    }
}

/// A [VmmResourceManager] represents a storage of all resources associated with a VMM. This trait is used by VMM
/// executors in order to initialize, as well as dispose of all resources.
pub trait VmmResourceManager: Send {
    /// Get an iterator of mutable references to the [MovedVmmResource]s from this manager.
    fn moved_resources(&mut self) -> impl Iterator<Item = &mut MovedVmmResource> + Send;

    /// Get an iterator of mutable references to the [CreatedVmmResource]s from this manager.
    fn created_resources(&mut self) -> impl Iterator<Item = &mut CreatedVmmResource> + Send;

    /// Get an iterator of mutable references to the [ProducedVmmResource]s from this manager.
    fn produced_resources(&mut self) -> impl Iterator<Item = &mut ProducedVmmResource> + Send;
}

/// A simple [VmmResourceManager] that operates over three [Vec]s for storing resources internally.
#[derive(Debug, Clone, Default)]
pub struct SimpleVmmResourceManager {
    pub moved_resources: Vec<MovedVmmResource>,
    pub created_resources: Vec<CreatedVmmResource>,
    pub produced_resources: Vec<ProducedVmmResource>,
}

impl VmmResourceManager for SimpleVmmResourceManager {
    fn moved_resources(&mut self) -> impl Iterator<Item = &mut MovedVmmResource> + Send {
        self.moved_resources.iter_mut()
    }

    fn created_resources(&mut self) -> impl Iterator<Item = &mut CreatedVmmResource> + Send {
        self.created_resources.iter_mut()
    }

    fn produced_resources(&mut self) -> impl Iterator<Item = &mut ProducedVmmResource> + Send {
        self.produced_resources.iter_mut()
    }
}
