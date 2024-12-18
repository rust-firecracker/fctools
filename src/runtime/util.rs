use std::{future::Future, path::Path};

use super::{Runtime, RuntimeTask};

#[cfg(feature = "vmm-process")]
use std::pin::Pin;

/// A utility set of multiple [RuntimeTask]s that are run concurrently which can be waited on to ensure all
/// contained [RuntimeTask]s complete.
#[derive(Default)]
pub struct RuntimeTaskSet<O: Send + 'static, R: Runtime> {
    tasks: Vec<R::Task<Result<(), O>>>,
    runtime: R,
}

impl<O: Send + 'static, R: Runtime> RuntimeTaskSet<O, R> {
    pub fn new(runtime: R) -> Self {
        Self {
            tasks: Vec::new(),
            runtime,
        }
    }

    pub fn spawn<F>(&mut self, future: F)
    where
        F: Future<Output = Result<(), O>> + Send + 'static,
    {
        self.tasks.push(self.runtime.spawn_task(future));
    }

    pub async fn wait(self) -> Option<Result<(), O>> {
        for task in self.tasks {
            match task.join().await {
                Some(result) => match result {
                    Ok(()) => {}
                    Err(err) => return Some(Err(err)),
                },
                None => return None,
            }
        }

        Some(Ok(()))
    }
}

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
/// internally.
#[cfg(feature = "vmm-process")]
#[cfg_attr(docsrs, doc(cfg(feature = "vmm-process")))]
#[derive(Clone)]
pub struct RuntimeHyperExecutor<R: Runtime>(pub R);

#[cfg(feature = "vmm-process")]
#[cfg_attr(docsrs, doc(cfg(feature = "vmm-process")))]
impl<R: Runtime> hyper::rt::Executor<Pin<Box<dyn Future<Output = ()> + Send>>> for RuntimeHyperExecutor<R> {
    fn execute(&self, future: Pin<Box<dyn Future<Output = ()> + Send>>) {
        self.0.spawn_task(future);
    }
}
