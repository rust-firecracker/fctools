use std::{future::Future, path::Path};

#[cfg(any(feature = "direct-process-spawner", feature = "elevation-process-spawners"))]
use std::ffi::OsString;

#[cfg(feature = "elevation-process-spawners")]
use futures_util::AsyncWriteExt;

#[cfg(feature = "elevation-process-spawners")]
use std::{
    path::PathBuf,
    sync::{Arc, LazyLock},
};

#[cfg(feature = "elevation-process-spawners")]
use crate::runtime::RuntimeChild;

use crate::runtime::Runtime;

/// A [ProcessSpawner] concerns itself with spawning a rootful or rootless process from the given binary path and arguments.
/// The command delegated to the spawner is either a "firecracker", "jailer" or "snapshot-editor" invocation for starting
/// the respective processes, or an elevated "chown"/"mkdir" invocation from the VMM executors.
///
/// Implementations of a [ProcessSpawner] are cloned highly frequently by fctools, so the [Clone] implementation must be fast
/// and cheap. If some inner state is stored, storing an [Arc] of it internally is recommended to avoid expensive copying
/// operations.
pub trait ProcessSpawner: Clone + Send + Sync + 'static {
    /// Spawn the process with the given binary path and arguments, optionally nulling as many of its pipes as feasible.
    fn spawn<R: Runtime>(
        &self,
        binary_path: &Path,
        arguments: Vec<String>,
        pipes_to_null: bool,
        runtime: &R,
    ) -> impl Future<Output = Result<R::Child, std::io::Error>> + Send;
}

/// A [ProcessSpawner] that directly invokes the underlying process.
#[derive(Debug, Clone)]
#[cfg(feature = "direct-process-spawner")]
#[cfg_attr(docsrs, doc(cfg(feature = "direct-process-spawner")))]
pub struct DirectProcessSpawner;

#[cfg(feature = "direct-process-spawner")]
#[cfg_attr(docsrs, doc(cfg(feature = "direct-process-spawner")))]
impl ProcessSpawner for DirectProcessSpawner {
    fn spawn<R: Runtime>(
        &self,
        binary_path: &Path,
        arguments: Vec<String>,
        pipes_to_null: bool,
        runtime: &R,
    ) -> impl Future<Output = Result<R::Child, std::io::Error>> + Send {
        std::future::ready(runtime.spawn_process(
            binary_path.as_os_str(),
            arguments.into_iter().map(OsString::from).collect(),
            !pipes_to_null,
            !pipes_to_null,
            !pipes_to_null,
        ))
    }
}

/// A [ProcessSpawner] that elevates the permissions of the process via the "su" CLI utility.
#[cfg(feature = "elevation-process-spawners")]
#[cfg_attr(docsrs, doc(cfg(feature = "elevation-process-spawners")))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SuProcessSpawner(Arc<SuProcessSpawnerInner>);

#[cfg(feature = "elevation-process-spawners")]
#[derive(Debug, PartialEq, Eq)]
struct SuProcessSpawnerInner {
    su_path: Option<PathBuf>,
    password: String,
}

#[cfg(feature = "elevation-process-spawners")]
#[cfg_attr(docsrs, doc(cfg(feature = "elevation-process-spawners")))]
impl SuProcessSpawner {
    /// Create a new [SuProcessSpawner] from a [String] password to use for authentication
    /// and, optionally, a [PathBuf] pointing to the "su" binary to invoke.
    pub fn new(password: String, su_path: Option<PathBuf>) -> Self {
        Self(Arc::new(SuProcessSpawnerInner { su_path, password }))
    }
}

#[cfg(feature = "elevation-process-spawners")]
static DEFAULT_SU_PROGRAM: LazyLock<OsString> = LazyLock::new(|| OsString::from("su"));

#[cfg(feature = "elevation-process-spawners")]
#[cfg_attr(docsrs, doc(cfg(feature = "elevation-process-spawners")))]
impl ProcessSpawner for SuProcessSpawner {
    async fn spawn<R: Runtime>(
        &self,
        path: &Path,
        arguments: Vec<String>,
        pipes_to_null: bool,
        runtime: &R,
    ) -> Result<R::Child, std::io::Error> {
        let program = match self.0.su_path {
            Some(ref path) => path.as_os_str(),
            None => DEFAULT_SU_PROGRAM.as_os_str(),
        };

        let mut process = runtime.spawn_process(program, Vec::new(), !pipes_to_null, !pipes_to_null, true)?;

        let stdin = process
            .get_stdin()
            .as_mut()
            .ok_or_else(|| std::io::Error::other("Stdin not received"))?;
        stdin.write_all(format!("{}\n", self.0.password).as_bytes()).await?;
        stdin
            .write_all(format!("{path:?} {} ; exit\n", arguments.join(" ")).as_bytes())
            .await?;

        if pipes_to_null {
            drop(process.take_stdin());
        }

        Ok(process)
    }
}

/// A [ProcessSpawner] that escalates the privileges of the process via the "sudo" CLI utility.
#[cfg(feature = "elevation-process-spawners")]
#[cfg_attr(docsrs, doc(cfg(feature = "elevation-process-spawners")))]
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SudoProcessSpawner(Arc<SudoProcessSpawnerInner>);

#[cfg(feature = "elevation-process-spawners")]
#[derive(Debug, PartialEq, Eq, Default)]
struct SudoProcessSpawnerInner {
    sudo_path: Option<PathBuf>,
    password: Option<String>,
}

#[cfg(feature = "elevation-process-spawners")]
#[cfg_attr(docsrs, doc(cfg(feature = "elevation-process-spawners")))]
impl SudoProcessSpawner {
    /// Create a new [SudoProcessSpawner] from, optionally, a [String] password to use for
    /// authentication and, optionally, a [PathBuf] pointing to the "sudo" binary to invoke.
    pub fn new(password: Option<String>, sudo_path: Option<PathBuf>) -> Self {
        Self(Arc::new(SudoProcessSpawnerInner { sudo_path, password }))
    }
}

#[cfg(feature = "elevation-process-spawners")]
static DEFAULT_SUDO_PROGRAM: LazyLock<OsString> = LazyLock::new(|| OsString::from("sudo"));

#[cfg(feature = "elevation-process-spawners")]
#[cfg_attr(docsrs, doc(cfg(feature = "elevation-process-spawners")))]
impl ProcessSpawner for SudoProcessSpawner {
    async fn spawn<R: Runtime>(
        &self,
        path: &Path,
        arguments: Vec<String>,
        pipes_to_null: bool,
        runtime: &R,
    ) -> Result<R::Child, std::io::Error> {
        let program = match self.0.sudo_path {
            Some(ref path) => path.as_os_str(),
            None => DEFAULT_SUDO_PROGRAM.as_os_str(),
        };

        let mut args = vec![OsString::from("-S"), OsString::from("-s"), OsString::from(path)];
        args.extend(arguments.into_iter().map(OsString::from));

        let mut child = runtime.spawn_process(program, args, !pipes_to_null, !pipes_to_null, true)?;
        let stdin_ref = child
            .get_stdin()
            .as_mut()
            .ok_or_else(|| std::io::Error::other("Stdin not received"))?;

        if let Some(ref password) = self.0.password {
            stdin_ref.write_all(format!("{password}\n").as_bytes()).await?;
        }

        if pipes_to_null {
            drop(child.take_stdin());
        }

        Ok(child)
    }
}
