use std::{
    ffi::OsString,
    future::Future,
    path::{Path, PathBuf},
    process::Stdio,
    sync::LazyLock,
};

use tokio::{
    io::AsyncWriteExt,
    process::{Child, Command},
};

/// A [ProcessSpawner] concerns itself with spawning a rootful or rootless process from the given binary path and arguments.
/// The command delegated to the spawner is either a "firecracker" or "jailer" invocation for starting the respective
/// processes, or an elevated "chown"/"mkdir" invocation from the executors.
pub trait ProcessSpawner: Send + Sync + 'static {
    /// Whether this [ProcessSpawner] spawns processes that have an upgraded ownership status.
    fn upgrades_ownership(&self) -> bool;

    /// Spawn the process with the given binary path and arguments.
    fn spawn(
        &self,
        binary_path: &Path,
        arguments: Vec<String>,
        pipes_to_null: bool,
    ) -> impl Future<Output = Result<Child, std::io::Error>> + Send;
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
        Stdio::piped()
    }
}

#[cfg(feature = "direct-process-spawner")]
impl ProcessSpawner for DirectProcessSpawner {
    fn upgrades_ownership(&self) -> bool {
        false
    }

    async fn spawn(&self, path: &Path, arguments: Vec<String>, pipes_to_null: bool) -> Result<Child, std::io::Error> {
        let mut command = Command::new(path);
        command
            .args(arguments)
            .stderr(get_stdio(pipes_to_null))
            .stdout(get_stdio(pipes_to_null))
            .stdin(get_stdio(pipes_to_null));
        let child = command.spawn()?;
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
    fn upgrades_ownership(&self) -> bool {
        true
    }

    async fn spawn(&self, path: &Path, arguments: Vec<String>, pipes_to_null: bool) -> Result<Child, std::io::Error> {
        let mut command = Command::new(match self.su_path {
            Some(ref path) => path.as_os_str(),
            None => SU_OS_STRING.as_os_str(),
        });
        command
            .stderr(get_stdio(pipes_to_null))
            .stdout(get_stdio(pipes_to_null))
            .stdin(Stdio::piped());
        let mut child = command.spawn()?;

        let stdin_ref = child
            .stdin
            .as_mut()
            .ok_or_else(|| std::io::Error::other("Stdin not received"))?;
        stdin_ref.write(format!("{}\n", self.password).as_bytes()).await?;
        stdin_ref
            .write(format!("{path:?} {} ; exit\n", arguments.join(" ")).as_bytes())
            .await?;

        if pipes_to_null {
            drop(child.stdin.take());
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
    fn upgrades_ownership(&self) -> bool {
        true
    }

    async fn spawn(&self, path: &Path, arguments: Vec<String>, pipes_to_null: bool) -> Result<Child, std::io::Error> {
        let mut command = Command::new(match self.sudo_path {
            Some(ref path) => path.as_os_str(),
            None => SUDO_OS_STRING.as_os_str(),
        });
        command
            .arg("-S")
            .arg("-s")
            .arg(path)
            .args(arguments)
            .stderr(get_stdio(pipes_to_null))
            .stdout(get_stdio(pipes_to_null))
            .stdin(Stdio::piped());

        let mut child = command.spawn()?;
        let stdin_ref = child
            .stdin
            .as_mut()
            .ok_or_else(|| std::io::Error::other("Stdin not received"))?;

        if let Some(ref password) = self.password {
            stdin_ref.write_all(format!("{password}\n").as_bytes()).await?;
        } else {
            return Err(std::io::Error::other(
                "Sudo requested a password but it wasn't provided in the shell instance",
            ));
        }

        if pipes_to_null {
            drop(child.stdin.take());
        }

        Ok(child)
    }
}
