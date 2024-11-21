//! Provides a wide variety of VMM-related APIs that cover VMM arguments, IDs, ownership, installations
//! and processes, gated behind various features.

use std::{ffi::CString, os::unix::ffi::OsStrExt, path::Path};

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

#[inline]
pub(crate) fn with_c_path_ptr<R>(path: &Path, function: impl FnOnce(*const i8) -> R) -> R {
    let c_string = CString::new(path.as_os_str().as_bytes()).expect("Encountered nul byte in path");
    let result = function(c_string.as_ptr());
    drop(c_string);
    result
}
