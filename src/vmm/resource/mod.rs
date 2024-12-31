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

pub trait VmmResourceManager: Send {
    fn moved_resources(&mut self) -> impl Iterator<Item = &mut MovedVmmResource> + Send;

    fn created_resources(&mut self) -> impl Iterator<Item = &mut CreatedVmmResource> + Send;

    fn produced_resources(&mut self) -> impl Iterator<Item = &mut ProducedVmmResource> + Send;
}
