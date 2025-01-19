//! Provides a wide variety of VMM-related APIs that concern themselves with the lower level of a
//! "firecracker"/"jailer" process instead of the underlying VM itself.
//!
//! With only the `vmm-core` feature enabled, these are available:
//! - VMM arguments (for "firecracker" and "jailer" binaries).
//! - VMM IDs.
//! - VMM installations (including the possibility to verify them at runtime).
//! - VMM resource management (moved, created and produced resources).
//! - VMM ownership models and implementation helpers.
//!
//! With the `vmm-executor` feature, a VMM executor trait is additionally available that abstracts
//! away the details of possibly jailing or not jailing a VMM, as well as other details of a VMM's lifecycle.
//!
//! The `unrestricted-vmm-executor`, `jailed-vmm-executor` and `either-vmm-executor` features enable the
//! respective default implementations of VMM executors.
//!
//! With the `vmm-process` feature, a VMM process abstraction that works on top of a VMM executor
//! and provides additional useful functionality like an HTTP connection pool is additionally available.

pub mod arguments;

pub mod resource;

pub mod resource_v3;

pub mod id;

pub mod installation;

pub mod ownership;

#[cfg(feature = "vmm-executor")]
#[cfg_attr(docsrs, doc(cfg(feature = "vmm-executor")))]
pub mod executor;

#[cfg(feature = "vmm-process")]
#[cfg_attr(docsrs, doc(cfg(feature = "vmm-process")))]
pub mod process;
