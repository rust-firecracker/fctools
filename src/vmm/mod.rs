//! Provides a wide variety of VMM-related APIs behind the following feature gates, in order of lower to higher level:
//! - `vmm-arguments`, full mappings to the CLI arguments of the "firecracker" and "jailer" binaries.
//! - `vmm-installation`, a simple struct containing the paths to relevant VMM toolchain binaries with the ability
//!   to verify the installation's validity at runtime.
//! - `vmm-executor`, a low-level executor abstraction that manages a VMM environment and invokes it.
//! - `vmm-process`, a higher-level (but lower than a VM) abstraction that manages the VMM process's full functionality.

#[cfg(feature = "vmm-arguments")]
#[cfg_attr(docsrs, doc(cfg(feature = "vmm-arguments")))]
pub mod arguments;

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
