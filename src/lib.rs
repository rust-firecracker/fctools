//! `fctools` is a highly modular and exhaustive SDK for developing high-performance asynchronous production applications that
//! leverage the capabilities of Firecracker microVMs.
//!
//! By default, only the [runtime] module that provides traits for supporting any asynchronous runtime is enabled. Binary
//! crates using `fctools` should usually pull in either a built-in implementation of a [Runtime](runtime::Runtime) via either
//! the `tokio-runtime` or `smol-runtime` features, or install a third-party crate with its own implementation.
//!
//! Either the "syscall-nix" or "syscall-rustix" feature must be enabled to provide syscalls for the runtime facilities, otherwise
//! the crate won't compile. These features use their respective crates internally to provide these syscalls. "nix" uses C FFI to
//! call into libc and perform syscalls, while "rustix" invokes syscalls directly without any FFI.
//!
//! The rest of the crate that provides actual functionality is structured in a vertical fashion, with each layer introducing more
//! useful and high-level features than the one preceding it. There are 6 such layers, from lowest to highest level of abstraction:
//!
//! 1. The process spawner layer, enabled via the `process-spawner` feature. It provides functionality for invoking the microVM process.
//! 2. The VMM core layer, enabled via the `vmm-core` feature. It provides basic facilities for managing a VMM.
//! 3. The VMM executor layer, enabled via the `vmm-executor` feature. It provides an executor trait that handles a VMM's lifecycle, as
//!    well as introducing handling of VMM ownership models.
//! 4. The VMM process layer, enabled via the `vmm-process` feature. It provides a VMM process abstraction over an underlying executor,
//!    introducing various useful features like making requests to the VMM's HTTP API server.
//! 5. The VM layer, enabled via the `vm` feature. It provides a wide range of high-level and opinionated facilities that build on top of
//!    a VMM process. These address concerns such as: high-level API server bindings, making snapshots, initializing VMs, shutting them
//!    down in a graceful and controlled fashion with timeouts and so on.
//! 6. The extension layer, enabled via various features ending with `-extension`. These small extensions, each typically spanning under
//!    500 lines of code, provide various real-world utilities useful for a microVM-based application.
//!
//! Each higher layer is more opinionated and high-level than its predecessor, while offering more useful features. Depending on the needs
//! of your application or library, you should decide which layers make sense for your use-case. Enabling the VM layer and all necessary
//! extensions is usually a good start.

#![cfg_attr(docsrs, feature(doc_cfg))]

#[cfg(feature = "vmm-core")]
#[cfg_attr(docsrs, doc(cfg(feature = "vmm-core")))]
pub mod vmm;

pub mod extension;

pub mod runtime;

#[cfg(feature = "process-spawner")]
#[cfg_attr(docsrs, doc(cfg(feature = "process-spawner")))]
pub mod process_spawner;

#[cfg(feature = "vm")]
#[cfg_attr(docsrs, doc(cfg(feature = "vm")))]
pub mod vm;

#[cfg(not(any(feature = "syscall-nix", feature = "syscall-rustix")))]
compile_error!("Either \"syscall-nix\" or \"syscall-rustix\" must be enabled to provide syscalls");

#[cfg(any(feature = "syscall-nix", feature = "syscall-rustix"))]
pub(crate) mod syscall;
