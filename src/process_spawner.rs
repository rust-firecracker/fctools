use std::{
    ffi::OsString,
    future::Future,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::LazyLock,
};

use futures_util::AsyncWriteExt;

use crate::runtime::{Runtime, RuntimeProcess};

/// A [ProcessSpawner] concerns itself with spawning a rootful or rootless process from the given binary path and arguments.
/// The command delegated to the spawner is either a "firecracker" or "jailer" invocation for starting the respective
/// processes, or an elevated "chown"/"mkdir" invocation from the executors.
pub trait ProcessSpawner: Send + Sync + 'static {
    /// Spawn the process with the given binary path and arguments.
    fn spawn<R: Runtime>(
        &self,
        binary_path: &Path,
        arguments: Vec<String>,
        pipes_to_null: bool,
    ) -> impl Future<Output = Result<R::Process, std::io::Error>> + Send;
}

/// A [ProcessSpawner] that directly invokes the underlying process.
#[derive(Debug)]
#[cfg(feature = "direct-process-spawner")]
pub struct DirectProcessSpawner;

#[inline(always)]
#[cfg(feature = "direct-process-spawner")]
fn get_stdio(pipes_to_null: bool) -> Stdio {
    if pipes_to_null {
        Stdio::null()
    } else {
        Stdio::inherit()
    }
}

#[cfg(feature = "direct-process-spawner")]
impl ProcessSpawner for DirectProcessSpawner {
    async fn spawn<R: Runtime>(
        &self,
        path: &Path,
        arguments: Vec<String>,
        pipes_to_null: bool,
    ) -> Result<R::Process, std::io::Error> {
        let mut command = Command::new(path);
        command.args(arguments);
        let child = R::Process::spawn(
            command,
            get_stdio(pipes_to_null),
            get_stdio(pipes_to_null),
            get_stdio(pipes_to_null),
        )?;
        Ok(child)
    }
}

#[cfg(feature = "elevation-process-spawners")]
static SU_OS_STRING: LazyLock<OsString> = LazyLock::new(|| OsString::from("su"));

/// A [ProcessSpawner] that elevates the permissions of the process via the "su" CLI utility.
#[cfg(feature = "elevation-process-spawners")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SuProcessSpawner {
    su_path: Option<PathBuf>,
    password: String,
}

#[cfg(feature = "elevation-process-spawners")]
impl SuProcessSpawner {
    pub fn new(password: impl Into<String>) -> Self {
        Self {
            su_path: None,
            password: password.into(),
        }
    }

    pub fn su_path(mut self, su_path: impl Into<PathBuf>) -> Self {
        self.su_path = Some(su_path.into());
        self
    }
}

#[cfg(feature = "elevation-process-spawners")]
impl ProcessSpawner for SuProcessSpawner {
    async fn spawn<R: Runtime>(
        &self,
        path: &Path,
        arguments: Vec<String>,
        pipes_to_null: bool,
    ) -> Result<R::Process, std::io::Error> {
        let command = Command::new(match self.su_path {
            Some(ref path) => path.as_os_str(),
            None => SU_OS_STRING.as_os_str(),
        });
        let mut child = R::Process::spawn(
            command,
            get_stdio(pipes_to_null),
            get_stdio(pipes_to_null),
            Stdio::piped(),
        )?;

        let stdin = child
            .stdin()
            .as_mut()
            .ok_or_else(|| std::io::Error::other("Stdin not received"))?;
        stdin.write_all(format!("{}\n", self.password).as_bytes()).await?;
        stdin
            .write_all(format!("{path:?} {} ; exit\n", arguments.join(" ")).as_bytes())
            .await?;

        if pipes_to_null {
            drop(child.take_stdin());
        }

        Ok(child)
    }
}

/// A [ProcessSpawner] that escalates the privileges of the process via the "sudo" CLI utility.
#[cfg(feature = "elevation-process-spawners")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SudoProcessSpawner {
    sudo_path: Option<PathBuf>,
    password: Option<String>,
}

#[cfg(feature = "elevation-process-spawners")]
impl SudoProcessSpawner {
    pub fn new() -> Self {
        Self {
            sudo_path: None,
            password: None,
        }
    }

    pub fn sudo_path(mut self, sudo_path: impl Into<PathBuf>) -> Self {
        self.sudo_path = Some(sudo_path.into());
        self
    }

    pub fn password(mut self, password: impl Into<String>) -> Self {
        self.password = Some(password.into());
        self
    }
}

#[cfg(feature = "elevation-process-spawners")]
static SUDO_OS_STRING: LazyLock<OsString> = LazyLock::new(|| OsString::from("sudo"));

#[cfg(feature = "elevation-process-spawners")]
impl ProcessSpawner for SudoProcessSpawner {
    async fn spawn<R: Runtime>(
        &self,
        path: &Path,
        arguments: Vec<String>,
        pipes_to_null: bool,
    ) -> Result<R::Process, std::io::Error> {
        let mut command = Command::new(match self.sudo_path {
            Some(ref path) => path.as_os_str(),
            None => SUDO_OS_STRING.as_os_str(),
        });
        command.arg("-S").arg("-s").arg(path).args(arguments);

        let mut child = R::Process::spawn(
            command,
            get_stdio(pipes_to_null),
            get_stdio(pipes_to_null),
            Stdio::piped(),
        )?;
        let stdin_ref = child
            .stdin()
            .as_mut()
            .ok_or_else(|| std::io::Error::other("Stdin not received"))?;

        if let Some(ref password) = self.password {
            stdin_ref.write_all(format!("{password}\n").as_bytes()).await?;
        }

        if pipes_to_null {
            drop(child.take_stdin());
        }

        Ok(child)
    }
}
