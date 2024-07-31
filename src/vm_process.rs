use std::{
    collections::HashMap,
    io,
    path::{Path, PathBuf},
    process::ExitStatus,
    sync::Arc,
};

use bytes::Bytes;
use http_body_util::Full;
use hyper::{body::Incoming, Request, Response, StatusCode, Uri};
use hyper_client_sockets::{HyperUnixConnector, UnixUriExt};
use hyper_util::{client::legacy::Client, rt::TokioExecutor};
use tokio::{
    process::{Child, ChildStderr, ChildStdin, ChildStdout},
    sync::OnceCell,
};

use crate::{
    arguments::FirecrackerConfigOverride,
    executor::{force_chown, ExecuteFirecracker, FirecrackerExecutorError},
    installation::FirecrackerInstallation,
    shell::SpawnShell,
};

/// A VM process is the low-level abstraction that manages a Firecracker process. It is
/// tied to the given executor E and shell spawner S.
#[derive(Debug)]
pub struct VmProcess<E: ExecuteFirecracker, S: SpawnShell> {
    executor: E,
    shell_spawner: Arc<S>,
    installation: Arc<FirecrackerInstallation>,
    child: Option<Child>,
    state: VmProcessState,
    socket_path: Option<PathBuf>,
    hyper_client: OnceCell<Client<HyperUnixConnector, Full<Bytes>>>,
    outer_paths: Option<Vec<PathBuf>>,
}

/// Raw Tokio pipes of a VM process: stdin, stdout and stderr. All must always be redirected
/// by the respective shell spawner.
#[derive(Debug)]
pub struct VmProcessPipes {
    pub stdin: ChildStdin,
    pub stdout: ChildStdout,
    pub stderr: ChildStderr,
}

/// The state of a VM process.
/// Keep in mind: the VM process lifecycle is not that of the VM! If the process has
/// started without a config file, API requests will need to be issued first in order
/// to start it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VmProcessState {
    /// The process hasn't started, and calling "prepare" is needed.
    AwaitingPrepare,
    /// The process hasn't started, a request to start it must be issued.
    AwaitingStart,
    /// The process is running without any faults.
    Started,
    /// The process has exited gracefully after a host-issued exit request.
    Exited,
    /// The process has crashed with the given non-zero exit status code.
    Crashed(ExitStatus),
}

/// Error caused during a VM process operation.
#[derive(Debug)]
pub enum VmProcessError {
    ExpectedState {
        expected: VmProcessState,
        actual: VmProcessState,
    },
    ExpectedExitedOrCrashed {
        actual: VmProcessState,
    },
    SocketWasDisabled,
    CouldNotChownSocket(io::Error),
    HyperClientFailed(hyper_util::client::legacy::Error),
    SigkillFailed(io::Error),
    CtrlAltDelRequestNotBuilt(hyper::http::Error),
    CtrlAltDelRequestFailed(StatusCode),
    WaitFailed(io::Error),
    IncorrectSocketUri(Box<dyn std::error::Error>),
    ExecutorError(FirecrackerExecutorError),
    PipesAlreadyTaken,
}

impl<E: ExecuteFirecracker, S: SpawnShell> VmProcess<E, S> {
    /// Create a new process instance while moving in its components.
    pub fn new(executor: E, shell_spawner: S, installation: FirecrackerInstallation, outer_paths: Vec<PathBuf>) -> Self {
        Self::new_arced(executor, Arc::new(shell_spawner), Arc::new(installation), outer_paths)
    }

    /// Create a new process instance without moving in its components, instead using Arc-s. This is recommended over moving in clones in
    /// all scenarios.
    pub fn new_arced(
        executor: E,
        shell_spawner_arc: Arc<S>,
        installation_arc: Arc<FirecrackerInstallation>,
        outer_paths: Vec<PathBuf>,
    ) -> Self {
        let socket_path = executor.get_outer_socket_path();
        Self {
            executor,
            shell_spawner: shell_spawner_arc,
            installation: installation_arc,
            child: None,
            state: VmProcessState::AwaitingPrepare,
            socket_path,
            hyper_client: OnceCell::new(),
            outer_paths: Some(outer_paths),
        }
    }

    /// Prepare the VM process environment. Allowed in AwaitingPrepare state, will result in AwaitingStart state.
    pub async fn prepare(&mut self) -> Result<HashMap<PathBuf, PathBuf>, VmProcessError> {
        self.ensure_state(VmProcessState::AwaitingPrepare)?;
        let path_mappings = self
            .executor
            .prepare(
                self.shell_spawner.as_ref(),
                self.outer_paths.take().expect("Outer paths cannot ever be none, unreachable"),
            )
            .await
            .map_err(VmProcessError::ExecutorError)?;
        self.state = VmProcessState::AwaitingStart;
        Ok(path_mappings)
    }

    /// Invoke the VM process. Allowed in AwaitingStart state, will result in Started state.
    pub async fn invoke(&mut self, config_override: FirecrackerConfigOverride) -> Result<(), VmProcessError> {
        self.ensure_state(VmProcessState::AwaitingStart)?;
        self.child = Some(
            self.executor
                .invoke(self.shell_spawner.as_ref(), self.installation.as_ref(), config_override)
                .await
                .map_err(VmProcessError::ExecutorError)?,
        );
        self.state = VmProcessState::Started;
        Ok(())
    }

    /// Send a given request (without a URI being set) to the given route of the Firecracker API server.
    /// Allowed in Started state.
    pub async fn send_api_request(
        &mut self,
        route: impl AsRef<str>,
        mut request: Request<Full<Bytes>>,
    ) -> Result<Response<Incoming>, VmProcessError> {
        self.ensure_state(VmProcessState::Started)?;
        let socket_path = self.socket_path.as_ref().ok_or(VmProcessError::SocketWasDisabled)?;
        let hyper_client = self
            .hyper_client
            .get_or_try_init(|| async {
                force_chown(&socket_path, self.shell_spawner.as_ref())
                    .await
                    .map_err(VmProcessError::ExecutorError)?;
                Ok(Client::builder(TokioExecutor::new()).build(HyperUnixConnector))
            })
            .await?;

        *request.uri_mut() = Uri::unix(socket_path, route).map_err(VmProcessError::IncorrectSocketUri)?;
        hyper_client.request(request).await.map_err(VmProcessError::HyperClientFailed)
    }

    /// Take out the stdout, stdin, stderr pipes of the underlying VM process. This can be only done once,
    /// if some code takes out the pipes, it now owns them for the remaining lifespan of the process.
    /// Allowed in Started state.
    pub fn take_pipes(&mut self) -> Result<VmProcessPipes, VmProcessError> {
        self.ensure_state(VmProcessState::Started)?;
        let child_ref = self.child.as_mut().expect("No child while running");
        let stdin = child_ref.stdin.take().ok_or(VmProcessError::PipesAlreadyTaken)?;
        let stdout = child_ref.stdout.take().ok_or(VmProcessError::PipesAlreadyTaken)?;
        let stderr = child_ref.stderr.take().ok_or(VmProcessError::PipesAlreadyTaken)?;
        Ok(VmProcessPipes { stdin, stdout, stderr })
    }

    /// Gets the outer path to the VM process's socket, if one has been configured
    pub fn get_socket_path(&self) -> Option<PathBuf> {
        self.executor.get_outer_socket_path()
    }

    /// Converts a given inner path to an outer path via the executor
    pub fn inner_to_outer_path(&self, inner_path: impl AsRef<Path>) -> PathBuf {
        self.executor.inner_to_outer_path(inner_path.as_ref())
    }

    /// Send a graceful shutdown request via Ctrl+Alt+Del to the VM process. Allowed on x86_64 as per Firecracker docs,
    /// on ARM either try to write "reboot\n" to stdin or pause the VM and SIGKILL it for a comparable effect.
    /// Allowed in Started state, will result in Exited state.
    pub async fn send_ctrl_alt_del(&mut self) -> Result<(), VmProcessError> {
        let response = self
            .send_api_request(
                "/actions",
                Request::builder()
                    .method("PUT")
                    .body(Full::new(Bytes::from(r#"{"action_type": "SendCtrlAltDel"}"#)))
                    .map_err(VmProcessError::CtrlAltDelRequestNotBuilt)?,
            )
            .await?;
        if !response.status().is_success() {
            return Err(VmProcessError::CtrlAltDelRequestFailed(response.status()));
        }

        Ok(())
    }

    /// Send an immediate forceful shutdown request in the form of a SIGKILL signal to the VM process.
    /// Allowed in Started state, will result in Crashed state.
    pub fn send_sigkill(&mut self) -> Result<(), VmProcessError> {
        self.ensure_state(VmProcessState::Started)?;
        self.child
            .as_mut()
            .expect("No child while running")
            .start_kill()
            .map_err(VmProcessError::SigkillFailed)
    }

    /// Wait until the VM process exits. Careful not to wait forever! Allowed in Started state, will result
    /// in either Exited or Crashed state, returning the exit status.
    pub async fn wait_for_exit(&mut self) -> Result<ExitStatus, VmProcessError> {
        self.ensure_state(VmProcessState::Started)?;
        self.child
            .as_mut()
            .expect("No child while running")
            .wait()
            .await
            .map_err(VmProcessError::WaitFailed)
    }

    /// Returns the current state of the VM process. Needs mutability since the tracking will need to be updated.
    /// Allowed in any state.
    pub fn state(&mut self) -> VmProcessState {
        self.update_state();
        self.state
    }

    /// Cleans up the VM process environment. Always call this as a sort of async drop mechanism! Allowed in Exited
    /// or Crashed state.
    pub async fn cleanup(&mut self) -> Result<(), VmProcessError> {
        self.ensure_exited_or_crashed()?;
        self.executor
            .cleanup(self.shell_spawner.as_ref())
            .await
            .map_err(VmProcessError::ExecutorError)?;
        Ok(())
    }

    fn ensure_state(&mut self, expected: VmProcessState) -> Result<(), VmProcessError> {
        if self.state() != expected {
            return Err(VmProcessError::ExpectedState {
                expected,
                actual: self.state,
            });
        }
        Ok(())
    }

    fn ensure_exited_or_crashed(&mut self) -> Result<(), VmProcessError> {
        let state = self.state();
        if let VmProcessState::Crashed(_) = state {
            return Ok(());
        }

        if state == VmProcessState::Exited {
            return Ok(());
        }

        Err(VmProcessError::ExpectedExitedOrCrashed { actual: state })
    }

    fn update_state(&mut self) {
        if let Some(child) = &mut self.child {
            if let Ok(Some(exit_status)) = child.try_wait() {
                if exit_status.success() {
                    self.state = VmProcessState::Exited;
                } else {
                    self.state = VmProcessState::Crashed(exit_status);
                }
            }
        }
    }
}
