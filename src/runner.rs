use std::{
    future::Future,
    path::{Path, PathBuf},
    process::Stdio,
};

use tokio::{
    io::AsyncWriteExt,
    process::{Child, Command},
};

/// A runner is layer 1 of fctools and concerns itself with spawning a rootful or rootless process.
/// The command delegated to the runner is either a firecracker or jailer invocation for starting the respective
/// processes, or an elevated chown/mkdir invocation from the executors.
pub trait Runner: Send + Sync + 'static {
    /// Whether the child processes spawned by this shell spawner have the same user and group ID as that of the
    /// main process itself (e.g. whether the shell spawner increases privileges for the child process).
    fn increases_privileges(&self) -> bool;

    /// Spawn the shell and enter shell_command in it, with the shell exiting as soon as the command completes.
    /// The returned tokio Child must be the shell's process.
    fn spawn(
        &self,
        path: &Path,
        arguments: Vec<String>,
        pipes_to_null: bool,
    ) -> impl Future<Output = Result<Child, std::io::Error>> + Send;
}

/// A runner implementation that directly invokes the underlying process.
#[derive(Debug)]
pub struct DirectRunner;

#[inline(always)]
fn get_stdio(pipes_to_null: bool) -> Stdio {
    if pipes_to_null {
        Stdio::null()
    } else {
        Stdio::piped()
    }
}

impl Runner for DirectRunner {
    fn increases_privileges(&self) -> bool {
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

/// A runner that elevates the permissions of the process via the "su" CLI utility.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SuRunner {
    su_path: PathBuf,
    password: String,
}

impl SuRunner {
    pub fn new(password: impl Into<String>) -> Self {
        Self {
            su_path: PathBuf::from("/usr/bin/su"),
            password: password.into(),
        }
    }

    pub fn su_path(mut self, su_path: impl Into<PathBuf>) -> Self {
        self.su_path = su_path.into();
        self
    }
}

impl Runner for SuRunner {
    fn increases_privileges(&self) -> bool {
        true
    }

    async fn spawn(&self, path: &Path, arguments: Vec<String>, pipes_to_null: bool) -> Result<Child, std::io::Error> {
        let mut command = Command::new(self.su_path.as_os_str());
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

        Ok(child)
    }
}

/// A runner that escalates the privileges of the process via the "sudo" CLI utility.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SudoRunner {
    /// The path to the "sudo" binary on the system, typically: /usr/bin/sudo.
    pub sudo_path: PathBuf,
    /// Optionally, the password needed to authenticate. Sudo often doesn't prompt for it if the
    /// user has already logged in, but it's recommended to pass it anyway so that authentication
    /// doesn't unexpectedly start failing.
    pub password: Option<String>,
}

impl Runner for SudoRunner {
    fn increases_privileges(&self) -> bool {
        true
    }

    async fn spawn(&self, path: &Path, arguments: Vec<String>, pipes_to_null: bool) -> Result<Child, std::io::Error> {
        let mut command = Command::new(self.sudo_path.as_os_str());
        command.arg("-S");
        command.arg("-s");
        command.arg(path);
        command.args(arguments);
        command
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

        Ok(child)
    }
}

#[cfg(test)]
#[test]
fn shell_spawners_have_correct_increases_privileges_flags() {
    assert!(!DirectRunner::new(which::which("sh").unwrap()).increases_privileges());
    assert!(SuRunner::new("password").increases_privileges());
    assert!(SudoRunner {
        sudo_path: which::which("sudo").unwrap(),
        password: None
    }
    .increases_privileges());
}
