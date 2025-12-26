use std::{future::Future, path::PathBuf, process::ExitStatus};

use async_once_cell::OnceCell;
use bytes::{Bytes, BytesMut};
use http::{Request, Response, StatusCode, Uri, uri::InvalidUri};
use http_body_util::{BodyExt, Full};
use hyper::body::{Body, Incoming};
use hyper_client_sockets::{connector::UnixConnector, uri::UnixUri};
use hyper_util::client::legacy::Client;

use super::{
    executor::{
        VmmExecutorContext,
        process_handle::{ProcessHandle, ProcessHandlePipes, ProcessHandlePipesError},
    },
    ownership::{ChangeOwnerError, upgrade_owner},
    resource::system::{ResourceSystem, ResourceSystemError},
};
use crate::{
    process_spawner::ProcessSpawner,
    runtime::{Runtime, util::RuntimeHyperExecutor},
    vmm::{
        executor::{VmmExecutor, VmmExecutorError},
        installation::VmmInstallation,
    },
};

/// A [VmmProcess] is an abstraction that manages a (possibly jailed) Firecracker process. It is
/// generic over a given [VmmExecutor] E, [ProcessSpawner] S and [Runtime] R.
#[derive(Debug)]
pub struct VmmProcess<E: VmmExecutor, S: ProcessSpawner, R: Runtime> {
    executor: E,
    pub(crate) resource_system: ResourceSystem<S, R>,
    pub(crate) installation: VmmInstallation,
    process_handle: Option<ProcessHandle<R>>,
    state: VmmProcessState,
    hyper_client: OnceCell<Client<UnixConnector<R::SocketBackend>, Full<Bytes>>>,
}

/// The state of a [VmmProcess]. Keep in mind that the [VmmProcess] lifecycle is not that of the VM!
/// If the process has been started without a config file, API requests will need to be issued first
/// in order to start the VM.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VmmProcessState {
    /// The process hasn't started, and calling "prepare" is needed.
    AwaitingPrepare,
    /// The process hasn't started, a request to start it must be issued.
    AwaitingStart,
    /// The process is currently running. This doesn't guarantee that the VM itself has booted.
    Started,
    /// The process has exited gracefully after an exit request that was issued through the [VmmProcess].
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
    /// The [VmmProcess] was in an incorrect [VmmProcessState], which is not allowed for the
    /// requested operation.
    IncorrectState(VmmProcessState),
    /// Attempted to send an API request or send Ctrl+Alt+Del (which is done via an API request)
    /// with the API socket being disabled.
    ApiSocketDisabled,
    /// A [ChangeOwnerError] occurred while changing the ownership of the VMM's API socket file.
    ChangeOwnerError(ChangeOwnerError),
    /// Completing an API request failed due to an HTTP error. The actual error is boxed and
    /// opaque in order not to expose the underlying HTTP connection pool's implementation, which
    /// is currently that of [hyper_util].
    RequestError(Box<dyn std::error::Error + Send + Sync>),
    /// A given URI for an API request was incorrect.
    InvalidUri {
        /// The invalid URI of the API request.
        uri: String,
        /// The [InvalidUri] error with the reason for the URI being invalid.
        error: InvalidUri,
    },
    /// An I/O error occurred while attempting to send a SIGKILL signal via the [ProcessHandle].
    SigkillError(std::io::Error),
    /// The Ctrl+Alt+Del HTTP request was invalid due to an [http::Error]. This is usually caused
    /// by an internal bug in the library.
    CtrlAltDelRequestInvalid(http::Error),
    /// The Ctrl+Alt+Del HTTP request was denied by the API with the given unsuccessful [StatusCode].
    CtrlAltDelRequestDenied(StatusCode),
    /// An I/O error occurred while waiting for the process to exit via the [ProcessHandle].
    ProcessWaitFailed(std::io::Error),
    /// A [VmmExecutorError] occurred while performing a prepare/invoke/cleanup invocation via the
    /// [VmmExecutor] of the [VmmProcess].
    ExecutorError(VmmExecutorError),
    /// A [ProcessHandlePipesError] occurred while attempting to take out pipes via the [ProcessHandle].
    ProcessHandlePipesError(ProcessHandlePipesError),
    /// A [ResourceSystemError] occurred while performing manual synchronization with the [ResourceSystem]
    /// after a [VmmExecutor] prepare/invoke/cleanup invocation.
    ResourceSystemError(ResourceSystemError),
}

impl std::error::Error for VmmProcessError {}

impl std::fmt::Display for VmmProcessError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VmmProcessError::IncorrectState(state) => {
                write!(
                    f,
                    "The VMM process was in the {state} state, which is invalid for the requested operation"
                )
            }
            VmmProcessError::ApiSocketDisabled => write!(
                f,
                "Attempted to perform an API request despite having disabled the socket"
            ),
            VmmProcessError::ChangeOwnerError(err) => write!(f, "An ownership change failed: {err}"),
            VmmProcessError::RequestError(err) => write!(f, "An issued API HTTP request failed: {err}"),
            VmmProcessError::InvalidUri { uri, error } => {
                write!(f, "The \"{uri}\" URI for an API HTTP request is invalid: {error}")
            }
            VmmProcessError::SigkillError(err) => write!(f, "Sending SIGKILL via process handle failed: {err}"),
            VmmProcessError::CtrlAltDelRequestInvalid(err) => {
                write!(f, "The Ctrl+Alt+Del HTTP request could not be built: {err}")
            }
            VmmProcessError::CtrlAltDelRequestDenied(status_code) => {
                write!(f, "The Ctrl+Alt+Del HTTP request failed with {status_code} status code")
            }
            VmmProcessError::ProcessWaitFailed(err) => write!(f, "Waiting on the exit of the process failed: {err}"),
            VmmProcessError::ExecutorError(err) => write!(f, "The underlying VMM executor returned an error: {err}"),
            VmmProcessError::ProcessHandlePipesError(err) => {
                write!(f, "Getting the pipes from the process handle failed: {err}")
            }
            VmmProcessError::ResourceSystemError(err) => {
                write!(f, "An error occurred within the resource system: {err}")
            }
        }
    }
}

impl<E: VmmExecutor, S: ProcessSpawner, R: Runtime> VmmProcess<E, S, R> {
    /// Create a new [VmmProcess] from a [VmmExecutor], a [VmmInstallation] and a [ResourceSystem]. All resources necessary for
    /// the [VmmProcess]'s operation should already be created within this [ResourceSystem] prior to creating a [VmmProcess] and
    /// preparing its environment.
    pub fn new(executor: E, resource_system: ResourceSystem<S, R>, installation: VmmInstallation) -> Self {
        Self {
            executor,
            resource_system,
            installation,
            process_handle: None,
            state: VmmProcessState::AwaitingPrepare,
            hyper_client: OnceCell::new(),
        }
    }

    /// Prepare the [VmmProcess] environment. Allowed in [VmmProcessState::AwaitingPrepare], will result in [VmmProcessState::AwaitingStart].
    pub async fn prepare(&mut self) -> Result<(), VmmProcessError> {
        self.ensure_state(VmmProcessState::AwaitingPrepare)?;
        self.executor
            .prepare(self.executor_context())
            .await
            .map_err(VmmProcessError::ExecutorError)?;
        self.resource_system
            .synchronize()
            .await
            .map_err(VmmProcessError::ResourceSystemError)?;
        self.state = VmmProcessState::AwaitingStart;
        Ok(())
    }

    /// Invoke the [VmmProcess] with the given configuration [PathBuf] for the VMM. Allowed in [VmmProcessState::AwaitingStart],
    /// will result in [VmmProcessState::Started].
    pub async fn invoke(&mut self, config_path: Option<PathBuf>) -> Result<(), VmmProcessError> {
        self.ensure_state(VmmProcessState::AwaitingStart)?;
        self.process_handle = Some(
            self.executor
                .invoke(self.executor_context(), config_path)
                .await
                .map_err(VmmProcessError::ExecutorError)?,
        );
        self.resource_system
            .synchronize()
            .await
            .map_err(VmmProcessError::ResourceSystemError)?;
        self.state = VmmProcessState::Started;
        Ok(())
    }

    /// Send a given request (without a URI being set) to the given route of the Firecracker API server.
    /// Allowed in [VmmProcessState::Started].
    pub async fn send_api_request<U: AsRef<str>>(
        &mut self,
        uri: U,
        mut request: Request<Full<Bytes>>,
    ) -> Result<Response<Incoming>, VmmProcessError> {
        self.ensure_state(VmmProcessState::Started)?;
        let route = uri.as_ref();
        let socket_path = self.get_socket_path().ok_or(VmmProcessError::ApiSocketDisabled)?;

        let hyper_client = self
            .hyper_client
            .get_or_try_init(async {
                upgrade_owner(
                    &socket_path,
                    self.resource_system.ownership_model,
                    &self.resource_system.process_spawner,
                    &self.resource_system.runtime,
                )
                .await
                .map_err(VmmProcessError::ChangeOwnerError)?;

                Ok(
                    Client::builder(RuntimeHyperExecutor(self.resource_system.runtime.clone()))
                        .build(UnixConnector::new()),
                )
            })
            .await?;

        *request.uri_mut() = Uri::unix(socket_path, route).map_err(|error| VmmProcessError::InvalidUri {
            uri: route.to_owned(),
            error,
        })?;

        hyper_client
            .request(request)
            .await
            .map_err(|err| VmmProcessError::RequestError(Box::new(err)))
    }

    /// Take out the stdout, stdin, stderr pipes of the underlying process. This can be only done once,
    /// if some code takes out the pipes, it now owns them for the remaining lifespan of the process.
    /// Allowed in [VmmProcessState::Started].
    pub fn take_pipes(&mut self) -> Result<ProcessHandlePipes<R::Child>, VmmProcessError> {
        self.ensure_state(VmmProcessState::Started)?;
        self.process_handle
            .as_mut()
            .expect("No process handle after having started cannot happen")
            .get_pipes()
            .map_err(VmmProcessError::ProcessHandlePipesError)
    }

    /// Gets the outer path to the API server socket, if one has been configured, via the executor.
    pub fn get_socket_path(&self) -> Option<PathBuf> {
        self.executor.get_socket_path(&self.installation)
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
                    .map_err(VmmProcessError::CtrlAltDelRequestInvalid)?,
            )
            .await?;
        if !response.status().is_success() {
            return Err(VmmProcessError::CtrlAltDelRequestDenied(response.status()));
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
            .map_err(VmmProcessError::SigkillError)
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

    /// Retrieve the current [VmmProcessState] of the [VmmProcess]. Needs mutable access (as well as most other
    /// [VmmProcess] methods relying on it) in order to query the underlying [ProcessHandle] for whether the process
    /// has exited. Allowed in any [VmmProcessState].
    pub fn get_state(&mut self) -> VmmProcessState {
        if let Some(ref mut process_handle) = self.process_handle {
            if let Ok(Some(exit_status)) = process_handle.try_wait() {
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
    pub async fn cleanup(&mut self) -> Result<(), VmmProcessError> {
        self.ensure_exited_or_crashed()?;
        self.executor
            .cleanup(self.executor_context())
            .await
            .map_err(VmmProcessError::ExecutorError)?;
        self.resource_system
            .synchronize()
            .await
            .map_err(VmmProcessError::ResourceSystemError)
    }

    /// Transforms a given local resource path into an effective resource path using the underlying [VmmExecutor].
    /// This should be used with care and only in cases when the facilities of the [ResourceSystem] prove to be insufficient.
    pub fn resolve_effective_path<P: Into<PathBuf>>(&self, local_path: P) -> PathBuf {
        self.executor
            .resolve_effective_path(&self.installation, local_path.into())
    }

    /// Get a shared reference to the [ResourceSystem] used by this [VmmProcess].
    pub fn get_resource_system(&self) -> &ResourceSystem<S, R> {
        &self.resource_system
    }

    /// Get a mutable reference to the [ResourceSystem] used by this [VmmProcess].
    pub fn get_resource_system_mut(&mut self) -> &mut ResourceSystem<S, R> {
        &mut self.resource_system
    }

    #[inline]
    fn ensure_state(&mut self, expected: VmmProcessState) -> Result<(), VmmProcessError> {
        match self.get_state() == expected {
            true => Ok(()),
            false => Err(VmmProcessError::IncorrectState(self.state)),
        }
    }

    #[inline]
    fn ensure_exited_or_crashed(&mut self) -> Result<(), VmmProcessError> {
        let state = self.get_state();

        if let VmmProcessState::Crashed(_) = state {
            return Ok(());
        }

        if state == VmmProcessState::Exited {
            return Ok(());
        }

        Err(VmmProcessError::IncorrectState(state))
    }

    #[inline]
    fn executor_context(&self) -> VmmExecutorContext<'_, S, R> {
        VmmExecutorContext {
            installation: self.installation.clone(),
            process_spawner: self.resource_system.process_spawner.clone(),
            runtime: self.resource_system.runtime.clone(),
            ownership_model: self.resource_system.ownership_model,
            resources: self.resource_system.get_resources(),
        }
    }
}

/// An extension to a hyper [Response] of [Incoming] (returned by the Firecracker API socket) that allows
/// easy streaming of the response body into a [String] or [BytesMut].
pub trait HyperResponseExt: Send {
    /// Stream the entire response body into a [BytesMut] byte buffer.
    fn read_body_to_buffer(&mut self) -> impl Future<Output = Result<BytesMut, hyper::Error>> + Send;

    /// Stream the entire response body into an owned [String].
    fn read_body_to_string(&mut self) -> impl Future<Output = Result<String, hyper::Error>> + Send {
        async {
            let buffer = self.read_body_to_buffer().await?;
            Ok(String::from_utf8_lossy(&buffer).into_owned())
        }
    }
}

impl HyperResponseExt for Response<Incoming> {
    async fn read_body_to_buffer(&mut self) -> Result<BytesMut, hyper::Error> {
        let mut buffer = BytesMut::with_capacity(self.size_hint().lower() as usize);

        while let Some(frame) = self.frame().await {
            if let Ok(bytes) = frame?.into_data() {
                buffer.extend(bytes);
            }
        }

        Ok(buffer)
    }
}
