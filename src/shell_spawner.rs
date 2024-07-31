use std::{io, path::PathBuf, process::Stdio};

use async_trait::async_trait;
use tokio::{
    io::AsyncWriteExt,
    process::{Child, Command},
};

/// ShellSpawner is layer 1 of fctools and concerns itself with spawning a rootful or rootless shell process.
/// The command delegated to the shell is either a firecracker or jailer invocation for starting the respective
/// processes, or a chown operation used by executors in order to elevate permissions.
#[async_trait]
pub trait ShellSpawner: Send + Sync {
    fn belongs_to_process(&self) -> bool;

    /// Spawn the shell and enter shell_command in it, with the shell exiting as soon as the command completes.
    /// The returned tokio Child must be the shell's process.
    async fn spawn(&self, shell_command: String) -> Result<Child, io::Error>;
}

/// A shell spawner that doesn't do privilege escalation and simply launches the given shell
/// as the current user. Acceptable for production scenarios when running as root or for development
/// when not using the jailer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SameUserShellSpawner {
    /// The path to the shell that should be used, typically to sh or bash.
    pub shell_path: PathBuf,
}

impl Default for SameUserShellSpawner {
    fn default() -> Self {
        Self {
            shell_path: PathBuf::from("/usr/bin/sh"),
        }
    }
}

#[async_trait]
impl ShellSpawner for SameUserShellSpawner {
    fn belongs_to_process(&self) -> bool {
        true
    }

    async fn spawn(&self, shell_command: String) -> Result<Child, io::Error> {
        let mut command = Command::new(self.shell_path.as_os_str());
        command
            .arg("-c")
            .arg(shell_command)
            .stderr(Stdio::piped())
            .stdout(Stdio::piped())
            .stdin(Stdio::piped());
        let child = command.spawn()?;
        Ok(child)
    }
}

/// A shell spawner that uses the universally available "su" utility in order to escalate to root
/// via the given root password, allowing use of the jailer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SuShellSpawner {
    /// The path to the "su" binary on the system, typically: /usr/bin/su.
    pub su_path: PathBuf,
    /// The root password to be used for escalation.
    pub password: String,
}

#[async_trait]
impl ShellSpawner for SuShellSpawner {
    fn belongs_to_process(&self) -> bool {
        false
    }

    async fn spawn(&self, shell_command: String) -> Result<Child, io::Error> {
        let mut command = Command::new(self.su_path.as_os_str());
        command
            .stderr(Stdio::piped())
            .stdout(Stdio::piped())
            .stdin(Stdio::piped());
        let mut child = command.spawn()?;

        let stdin_ref = child
            .stdin
            .as_mut()
            .ok_or_else(|| io::Error::other("Stdin not received"))?;
        stdin_ref
            .write(format!("{}\n", self.password).as_bytes())
            .await?;
        stdin_ref
            .write(format!("{shell_command} ; exit\n").as_bytes())
            .await?;

        Ok(child)
    }
}

/// A shell spawner that uses the "sudo" utility (ensure it is installed on the OS!) in order to
/// escalate to root, optionally providing a root password. This allows use of the jailer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SudoShellSpawner {
    /// The path to the "sudo" binary on the system, typically: /usr/bin/sudo.
    pub sudo_path: PathBuf,
    /// Optionally, the password needed to authenticate. Sudo often doesn't prompt for it if the
    /// user has already logged in, but it's recommended to pass it anyway so that authentication
    /// doesn't unexpectedly start failing.
    pub password: Option<String>,
}

#[async_trait]
impl ShellSpawner for SudoShellSpawner {
    fn belongs_to_process(&self) -> bool {
        false
    }

    async fn spawn(&self, shell_command: String) -> Result<Child, io::Error> {
        let mut command = Command::new(self.sudo_path.as_os_str());
        command.arg("-S");
        command.arg("-s");
        for component in shell_command.split(' ') {
            command.arg(component);
        }
        command
            .stderr(Stdio::piped())
            .stdout(Stdio::piped())
            .stdin(Stdio::piped());
        let mut child = command.spawn()?;

        let stdin_ref = child
            .stdin
            .as_mut()
            .ok_or_else(|| io::Error::other("Stdin not received"))?;
        if let Some(password) = &self.password {
            stdin_ref.write(format!("{password}\n").as_bytes()).await?;
        } else {
            return Err(io::Error::other(
                "Sudo requested a password but it wasn't provided in the shell instance",
            ));
        }

        Ok(child)
    }
}
