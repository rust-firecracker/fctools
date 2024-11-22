//! Provides a wide variety of VMM-related APIs that cover VMM arguments, IDs, ownership, installations,
//! resources and processes, gated behind various features.

#[cfg(feature = "vmm-arguments")]
#[cfg_attr(docsrs, doc(cfg(feature = "vmm-arguments")))]
pub mod arguments;

#[cfg(feature = "vmm-arguments")]
#[cfg_attr(docsrs, doc(cfg(feature = "vmm-executor")))]
pub mod resource;

#[cfg(feature = "vmm-arguments")]
#[cfg_attr(docsrs, doc(cfg(feature = "vmm-arguments")))]
pub mod id;

#[cfg(feature = "vmm-executor")]
#[cfg_attr(docsrs, doc(cfg(feature = "vmm-executor")))]
pub mod executor;

#[cfg(feature = "vmm-executor")]
#[cfg_attr(docsrs, doc(cfg(feature = "vmm-executor")))]
pub mod ownership;

#[cfg(feature = "vmm-installation")]
#[cfg_attr(docsrs, doc(cfg(feature = "vmm-installation")))]
pub mod installation;

#[cfg(feature = "vmm-process")]
#[cfg_attr(docsrs, doc(cfg(feature = "vmm-process")))]
pub mod process;
