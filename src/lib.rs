use std::{fmt::Debug, ops::Deref, sync::Arc};

#[cfg(feature = "vmm-executor")]
pub mod vmm;

pub mod extension;

#[cfg(feature = "fs-backend")]
pub mod fs_backend;

#[cfg(feature = "process-spawner")]
pub mod process_spawner;

#[cfg(feature = "vm")]
pub mod vm;

pub(crate) enum MaybeArced<T: Debug> {
    Owned(T),
    Arced(Arc<T>),
}

impl<T: Debug> Debug for MaybeArced<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Owned(inner) => inner.fmt(f),
            Self::Arced(inner) => inner.fmt(f),
        }
    }
}

impl<T: Debug> Deref for MaybeArced<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        match self {
            MaybeArced::Owned(inner) => inner,
            MaybeArced::Arced(inner) => inner.as_ref(),
        }
    }
}
