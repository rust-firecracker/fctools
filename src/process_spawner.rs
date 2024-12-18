use std::{future::Future, path::Path};

#[cfg(feature = "elevation-process-spawners")]
use futures_util::AsyncWriteExt;

#[cfg(feature = "elevation-process-spawners")]
use std::{
    ffi::OsString,
    path::PathBuf,
    sync::{Arc, LazyLock},
};

#[cfg(feature = "elevation-process-spawners")]
use crate::runtime::RuntimeChild;

#[cfg(any(feature = "direct-process-spawner", feature = "elevation-process-spawners"))]
use std::process::{Command, Stdio};

use crate::runtime::Runtime;

/// A [ProcessSpawner] concerns itself with spawning a rootful or rootless process from the given binary path and arguments.
/// The command delegated to the spawner is either a "firecracker" or "jailer" invocation for starting the respective
/// processes, or an elevated "chown"/"mkdir" invocation from the executors.
///
/// Implementations of a [ProcessSpawner] are cloned highly frequently by fctools, so the [Clone] implementation must be fast
/// and cheap. If some inner state is stored, storing an [Arc](std::sync::Arc) of it internally is recommended to avoid
/// expensive copying operations.
pub trait ProcessSpawner: Clone + Send + Sync + 'static {
    /// Spawn the process with the given binary path and arguments.
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

#[inline(always)]
#[cfg(any(feature = "direct-process-spawner", feature = "elevation-process-spawners"))]
fn get_stdio(pipes_to_null: bool) -> Stdio {
    if pipes_to_null {
        Stdio::null()
    } else {
        Stdio::piped()
    }
}

#[cfg(feature = "direct-process-spawner")]
#[cfg_attr(docsrs, doc(cfg(feature = "direct-process-spawner")))]
impl ProcessSpawner for DirectProcessSpawner {
    async fn spawn<R: Runtime>(
        &self,
        path: &Path,
        arguments: Vec<String>,
        pipes_to_null: bool,
        runtime: &R,
    ) -> Result<R::Child, std::io::Error> {
        let mut command = Command::new(path);
        command.args(arguments);
        let child = runtime.spawn_child(
            command,
            get_stdio(pipes_to_null),
            get_stdio(pipes_to_null),
            get_stdio(pipes_to_null),
        )?;

        Ok(child)
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
    pub fn new(password: String, su_path: Option<PathBuf>) -> Self {
        Self(Arc::new(SuProcessSpawnerInner { su_path, password }))
    }
}

#[cfg(feature = "elevation-process-spawners")]
static SU_OS_STRING: LazyLock<OsString> = LazyLock::new(|| OsString::from("su"));

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
        let command = Command::new(match self.0.su_path {
            Some(ref path) => path.as_os_str(),
            None => SU_OS_STRING.as_os_str(),
        });

        let mut process = runtime.spawn_child(
            command,
            get_stdio(pipes_to_null),
            get_stdio(pipes_to_null),
            Stdio::piped(),
        )?;

        let stdin = process
            .stdin()
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
    pub fn new(password: Option<String>, sudo_path: Option<PathBuf>) -> Self {
        Self(Arc::new(SudoProcessSpawnerInner { sudo_path, password }))
    }
}

#[cfg(feature = "elevation-process-spawners")]
static SUDO_OS_STRING: LazyLock<OsString> = LazyLock::new(|| OsString::from("sudo"));

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
        let mut command = Command::new(match self.0.sudo_path {
            Some(ref path) => path.as_os_str(),
            None => SUDO_OS_STRING.as_os_str(),
        });
        command.arg("-S").arg("-s").arg(path).args(arguments);

        let mut child = runtime.spawn_child(
            command,
            get_stdio(pipes_to_null),
            get_stdio(pipes_to_null),
            Stdio::piped(),
        )?;
        let stdin_ref = child
            .stdin()
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
