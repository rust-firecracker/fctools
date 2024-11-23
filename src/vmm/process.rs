use std::{future::Future, path::PathBuf, process::ExitStatus, sync::Arc};

use async_once_cell::OnceCell;
use bytes::{Bytes, BytesMut};
use http::{Request, Response, StatusCode, Uri};
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper_client_sockets::unix::{connector::HyperUnixConnector, UnixUriExt};
use hyper_util::client::legacy::Client;

use crate::{
    process_spawner::ProcessSpawner,
    runtime::Runtime,
    vmm::{
        executor::{VmmExecutor, VmmExecutorError},
        installation::VmmInstallation,
    },
};

use super::{
    executor::process_handle::{ProcessHandle, ProcessHandlePipes, ProcessHandlePipesError},
    ownership::{upgrade_owner, ChangeOwnerError, VmmOwnershipModel},
    resource::VmmResourceReferences,
};

/// A [VmmProcess] is an abstraction that manages a (possibly jailed) Firecracker process. It is
/// tied to the given [VmmExecutor] E, [ProcessSpawner] S and [Runtime] R.
#[derive(Debug)]
pub struct VmmProcess<E: VmmExecutor, S: ProcessSpawner, R: Runtime> {
    executor: E,
    ownership_model: VmmOwnershipModel,
    process_spawner: Arc<S>,
    installation: Arc<VmmInstallation>,
    process_handle: Option<ProcessHandle<R::Process>>,
    state: VmmProcessState,
    hyper_client: OnceCell<Client<HyperUnixConnector, Full<Bytes>>>,
}

/// The state of a [VmmProcess].
/// Keep in mind: the [VmmProcess] lifecycle is not that of the VM! If the process has
/// started without a config file, API requests will need to be issued first in order
/// to start it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VmmProcessState {
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

impl std::fmt::Display for VmmProcessState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VmmProcessState::AwaitingPrepare => write!(f, "Awaiting prepare"),
            VmmProcessState::AwaitingStart => write!(f, "Awaiting start"),
            VmmProcessState::Started => write!(f, "Started"),
            VmmProcessState::Exited => write!(f, "Exited"),
            VmmProcessState::Crashed(exit_status) => write!(f, "Crashed with exit status: {exit_status}"),
        }
    }
}

/// Error caused during a [VmmProcess] operation.
#[derive(Debug)]
pub enum VmmProcessError {
    ExpectedState {
        expected: VmmProcessState,
        actual: VmmProcessState,
    },
    ExpectedExitedOrCrashed {
        actual: VmmProcessState,
    },
    ApiSocketDisabledError,
    ChangeOwnerError(ChangeOwnerError),
    ApiRequestError(hyper_util::client::legacy::Error),
    ApiInvalidRouteError {
        route: String,
        error: http::uri::InvalidUri,
    },
    SigkillFailed(std::io::Error),
    CtrlAltDelRequestNotBuilt(hyper::http::Error),
    CtrlAltDelRequestFailed(StatusCode),
    ProcessWaitFailed(std::io::Error),
    ExecutorError(VmmExecutorError),
    ProcessHandlePipesError(ProcessHandlePipesError),
}

impl std::error::Error for VmmProcessError {}

impl std::fmt::Display for VmmProcessError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VmmProcessError::ExpectedState { expected, actual } => write!(
                f,
                "Expected the process to be in the {expected} state, but it was in the {actual} state"
            ),
            VmmProcessError::ExpectedExitedOrCrashed { actual } => write!(
                f,
                "Expected the process to have exited or crashed, but it was in the {actual} state"
            ),
            VmmProcessError::ApiSocketDisabledError => write!(
                f,
                "Attempted to perform an API request despite having disabled the socket"
            ),
            VmmProcessError::ChangeOwnerError(err) => write!(f, "An ownership change failed: {err}"),
            VmmProcessError::ApiRequestError(err) => write!(f, "An issued API HTTP request failed: {err}"),
            VmmProcessError::ApiInvalidRouteError { route, error } => {
                write!(f, "The \"{route}\" route for an API HTTP request is invalid: {error}")
            }
            VmmProcessError::SigkillFailed(err) => write!(f, "Sending SIGKILL via process handle failed: {err}"),
            VmmProcessError::CtrlAltDelRequestNotBuilt(err) => {
                write!(f, "The Ctrl+Alt+Del HTTP request could not be built: {err}")
            }
            VmmProcessError::CtrlAltDelRequestFailed(status_code) => {
                write!(f, "The Ctrl+Alt+Del HTTP request failed with {status_code} status code")
            }
            VmmProcessError::ProcessWaitFailed(err) => write!(f, "Waiting on the exit of the process failed: {err}"),
            VmmProcessError::ExecutorError(err) => write!(f, "The underlying VMM executor returned an error: {err}"),
            VmmProcessError::ProcessHandlePipesError(err) => {
                write!(f, "Getting the pipes from the process handle failed: {err}")
            }
        }
    }
}

impl<E: VmmExecutor, S: ProcessSpawner, R: Runtime> VmmProcess<E, S, R> {
    /// Create a new [VmmProcess] from the given component. Each component is either its owned value or an [Arc]; in the
    /// former case it will be put into an [Arc], otherwise the [Arc] will be kept. This allows for performant non-clone-based
    /// sharing of [VmmProcess] components between multiple [VmmProcess]-es.
    pub fn new(
        executor: E,
        ownership_model: VmmOwnershipModel,
        process_spawner: impl Into<Arc<S>>,
        installation: impl Into<Arc<VmmInstallation>>,
    ) -> Self {
        let installation = installation.into();
        Self {
            executor,
            ownership_model,
            process_spawner: process_spawner.into(),
            installation,
            process_handle: None,
            state: VmmProcessState::AwaitingPrepare,
            hyper_client: OnceCell::new(),
        }
    }

    /// Prepare the [VmmProcess] environment. Allowed in [VmmProcessState::AwaitingPrepare], will result in [VmmProcessState::AwaitingStart].
    pub async fn prepare(&mut self, resource_references: VmmResourceReferences<'_>) -> Result<(), VmmProcessError> {
        self.ensure_state(VmmProcessState::AwaitingPrepare)?;
        self.executor
            .prepare::<R>(
                self.installation.as_ref(),
                self.process_spawner.clone(),
                self.ownership_model,
                resource_references,
            )
            .await
            .map_err(VmmProcessError::ExecutorError)?;
        self.state = VmmProcessState::AwaitingStart;
        Ok(())
    }

    /// Invoke the [VmmProcess] with the given configuration [PathBuf] for the VMM. Allowed in [VmmProcessState::AwaitingStart],
    /// will result in [VmmProcessState::Started].
    pub async fn invoke(&mut self, config_path: Option<PathBuf>) -> Result<(), VmmProcessError> {
        self.ensure_state(VmmProcessState::AwaitingStart)?;
        self.process_handle = Some(
            self.executor
                .invoke::<R>(
                    self.installation.as_ref(),
                    self.process_spawner.clone(),
                    config_path,
                    self.ownership_model,
                )
                .await
                .map_err(VmmProcessError::ExecutorError)?,
        );
        self.state = VmmProcessState::Started;
        Ok(())
    }

    /// Send a given request (without a URI being set) to the given route of the Firecracker API server.
    /// Allowed in [VmmProcessState::Started].
    pub async fn send_api_request(
        &mut self,
        route: impl AsRef<str>,
        mut request: Request<Full<Bytes>>,
    ) -> Result<Response<Incoming>, VmmProcessError> {
        self.ensure_state(VmmProcessState::Started)?;
        let route = route.as_ref();
        let socket_path = self.get_socket_path().ok_or(VmmProcessError::ApiSocketDisabledError)?;

        let hyper_client = self
            .hyper_client
            .get_or_try_init(async {
                upgrade_owner::<R>(&socket_path, self.ownership_model, self.process_spawner.as_ref())
                    .await
                    .map_err(VmmProcessError::ChangeOwnerError)?;

                Ok(Client::builder(R::get_hyper_executor()).build(HyperUnixConnector {
                    backend: R::get_hyper_client_sockets_backend(),
                }))
            })
            .await?;

        *request.uri_mut() = Uri::unix(socket_path, route).map_err(|error| VmmProcessError::ApiInvalidRouteError {
            route: route.to_owned(),
            error,
        })?;

        hyper_client
            .request(request)
            .await
            .map_err(VmmProcessError::ApiRequestError)
    }

    /// Take out the stdout, stdin, stderr pipes of the underlying process. This can be only done once,
    /// if some code takes out the pipes, it now owns them for the remaining lifespan of the process.
    /// Allowed in [VmmProcessState::Started].
    pub fn take_pipes(&mut self) -> Result<ProcessHandlePipes<R::Process>, VmmProcessError> {
        self.ensure_state(VmmProcessState::Started)?;
        self.process_handle
            .as_mut()
            .expect("No process handle after having started cannot happen")
            .get_pipes()
            .map_err(VmmProcessError::ProcessHandlePipesError)
    }

    /// Gets the outer path to the API server socket, if one has been configured, via the executor.
    pub fn get_socket_path(&self) -> Option<PathBuf> {
        self.executor.get_socket_path(self.installation.as_ref())
    }

    /// Send a graceful shutdown request via Ctrl+Alt+Del to the [VmmProcess]. Allowed on x86_64 as per Firecracker docs,
    /// on ARM either try to write "reboot\n" to stdin or pause the VM and SIGKILL it for a comparable effect.
    /// Allowed in [VmmProcessState::Started], will result in [VmmProcessState::Exited].
    pub async fn send_ctrl_alt_del(&mut self) -> Result<(), VmmProcessError> {
        let response = self
            .send_api_request(
                "/actions",
                Request::builder()
                    .method("PUT")
                    .body(Full::new(Bytes::from(r#"{"action_type": "SendCtrlAltDel"}"#)))
                    .map_err(VmmProcessError::CtrlAltDelRequestNotBuilt)?,
            )
            .await?;
        if !response.status().is_success() {
            return Err(VmmProcessError::CtrlAltDelRequestFailed(response.status()));
        }

        Ok(())
    }

    /// Send an immediate forceful shutdown request in the form of a SIGKILL signal to the [VmmProcess].
    /// Allowed in [VmmProcessState::Started] state, will result in [VmmProcessState::Crashed] state.
    pub fn send_sigkill(&mut self) -> Result<(), VmmProcessError> {
        self.ensure_state(VmmProcessState::Started)?;
        self.process_handle
            .as_mut()
            .expect("No child while running")
            .send_sigkill()
            .map_err(VmmProcessError::SigkillFailed)
    }

    /// Wait until the [VmmProcess] exits. Careful not to wait forever! Allowed in [VmmProcessState::Started], will result
    /// in either [VmmProcessState::Started] or [VmmProcessState::Crashed], returning the [ExitStatus] of the process.
    pub async fn wait_for_exit(&mut self) -> Result<ExitStatus, VmmProcessError> {
        self.ensure_state(VmmProcessState::Started)?;
        self.process_handle
            .as_mut()
            .expect("No child while running")
            .wait()
            .await
            .map_err(VmmProcessError::ProcessWaitFailed)
    }

    /// Returns the current [VmmProcessState] of the [VmmProcess]. Needs mutable access (as well as most other
    /// [VmmProcess] methods relying on it) in order to query the process Allowed in any [VmmProcessState].
    pub fn state(&mut self) -> VmmProcessState {
        if let Some(ref mut child) = self.process_handle {
            if let Ok(Some(exit_status)) = child.try_wait() {
                if exit_status.success() {
                    self.state = VmmProcessState::Exited;
                } else {
                    self.state = VmmProcessState::Crashed(exit_status);
                }
            }
        }

        self.state
    }

    /// Cleans up the [VmmProcess]'s environment. Always call this as a sort of async [Drop] mechanism! Allowed in
    /// [VmmProcessState::Exited] or [VmmProcessState::Crashed].
    pub async fn cleanup(&mut self, resource_references: VmmResourceReferences<'_>) -> Result<(), VmmProcessError> {
        self.ensure_exited_or_crashed()?;
        self.executor
            .cleanup::<R>(
                self.installation.as_ref(),
                self.process_spawner.clone(),
                self.ownership_model,
                resource_references,
            )
            .await
            .map_err(VmmProcessError::ExecutorError)?;
        Ok(())
    }

    fn ensure_state(&mut self, expected: VmmProcessState) -> Result<(), VmmProcessError> {
        if self.state() != expected {
            return Err(VmmProcessError::ExpectedState {
                expected,
                actual: self.state,
            });
        }
        Ok(())
    }

    fn ensure_exited_or_crashed(&mut self) -> Result<(), VmmProcessError> {
        let state = self.state();
        if let VmmProcessState::Crashed(_) = state {
            return Ok(());
        }

        if state == VmmProcessState::Exited {
            return Ok(());
        }

        Err(VmmProcessError::ExpectedExitedOrCrashed { actual: state })
    }
}

/// An extension to a hyper [Response<Incoming>] (returned by the Firecracker API socket) that allows
/// easy streaming of the response body into a [String] or [BytesMut].
pub trait HyperResponseExt: Send {
    /// Stream the entire response body into a byte buffer (BytesMut).
    fn recv_to_buf(&mut self) -> impl Future<Output = Result<BytesMut, hyper::Error>> + Send;

    /// Stream the entire response body into an owned string.
    fn recv_to_string(&mut self) -> impl Future<Output = Result<String, hyper::Error>> + Send {
        async {
            let buf = self.recv_to_buf().await?;
            Ok(String::from_utf8_lossy(&buf).into_owned())
        }
    }
}

impl HyperResponseExt for Response<Incoming> {
    async fn recv_to_buf(&mut self) -> Result<BytesMut, hyper::Error> {
        let mut buf = BytesMut::new();
        while let Some(frame) = self.frame().await {
            if let Ok(bytes) = frame?.into_data() {
                buf.extend(bytes);
            }
        }
        Ok(buf)
    }
}
