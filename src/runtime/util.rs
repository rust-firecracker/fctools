//! Extra utilities for runtime implementors.

use std::{future::Future, path::Path};

use super::Runtime;

/// A simple utility that performs recursive chown syscalls on the given directory's [Path] to
/// the given UID and GID. This operation is implemented via the blocking [std::fs::read_dir]
/// operation, meaning it should never be called in an async context, or should be delegated to
/// a blocking thread.
///
/// This is used with blocking threads by the Tokio and Smol runtime implementations to implement
/// [Runtime::fs_chown_all], and is public for usage by third-party runtimes too.
pub fn chown_all_blocking(path: &Path, uid: u32, gid: u32) -> Result<(), std::io::Error> {
    if path.is_dir() {
        for entry in std::fs::read_dir(path)? {
            let entry = entry?;
            chown_all_blocking(entry.path().as_path(), uid, gid)?;
        }
    }

    crate::syscall::chown(path, uid, gid)
}

/// A [hyper::rt::Executor] implementation that is agnostic over any [Runtime] by simply using [Runtime::spawn_task]
/// internally. Any static [Send] future that returns a static [Send] type upon completion is supported, mirroring
/// the definition of [Runtime::spawn_task] itself.
#[cfg(feature = "vmm-process")]
#[cfg_attr(docsrs, doc(cfg(feature = "vmm-process")))]
#[derive(Clone)]
pub struct RuntimeHyperExecutor<R: Runtime>(pub R);

#[cfg(feature = "vmm-process")]
#[cfg_attr(docsrs, doc(cfg(feature = "vmm-process")))]
impl<R, F> hyper::rt::Executor<F> for RuntimeHyperExecutor<R>
where
    R: Runtime,
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    fn execute(&self, future: F) {
        self.0.spawn_task(future);
    }
}
