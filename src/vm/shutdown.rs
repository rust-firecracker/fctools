use std::{process::ExitStatus, time::Duration};

use futures_util::AsyncWriteExt;

use crate::{
    process_spawner::ProcessSpawner,
    runtime::Runtime,
    vmm::{executor::VmmExecutor, process::VmmProcessError},
};

use super::{
    api::{VmApi, VmApiError},
    Vm, VmStateCheckError,
};

/// The methods that can be used to shut down a [Vm].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VmShutdownMethod {
    /// Send a SIGKILL to the VMM process. Recommended as a last-resort option.
    Kill,
    /// Pause the VM, then send a SIGKILL to the VMM process. This minimizes the amount of damage inside the VM caused
    /// by a sudden shutdown (essentially, a force power-off) and is recommended as the primary option on ARM CPUs
    /// with no Ctrl+Alt+Del support.
    PauseThenKill,
    /// Performs a graceful shutdown by sending Ctrl+Alt+Del to the VM. Only supported on x86_64 CPUs and recommended
    /// as a primary option.
    CtrlAltDel,
    /// Performs a shutdown by taking the VMM process's stdin pipe and writing the provided byte sequence to it. The byte
    /// sequence can, for example, be "systemctl reboot\n". Recommended as a backup option on ARM CPUs with no Ctrl+Alt+Del
    /// support.
    WriteToSerial(Vec<u8>),
}

impl VmShutdownMethod {
    async fn run<E: VmmExecutor, S: ProcessSpawner, R: Runtime>(
        &self,
        vm: &mut Vm<E, S, R>,
    ) -> Result<ExitStatus, VmShutdownError> {
        match self {
            VmShutdownMethod::Kill => vm.vmm_process.send_sigkill().map_err(VmShutdownError::KillError)?,
            VmShutdownMethod::PauseThenKill => {
                vm.api_pause().await.map_err(VmShutdownError::PauseError)?;
                vm.vmm_process.send_sigkill().map_err(VmShutdownError::KillError)?
            }
            VmShutdownMethod::CtrlAltDel => vm
                .vmm_process
                .send_ctrl_alt_del()
                .await
                .map_err(VmShutdownError::SendCtrlAltDelError)?,
            VmShutdownMethod::WriteToSerial(bytes) => {
                let mut pipes = vm.vmm_process.take_pipes().map_err(VmShutdownError::TakePipesError)?;
                pipes
                    .stdin
                    .write_all(bytes)
                    .await
                    .map_err(VmShutdownError::SerialError)?;
                pipes.stdin.flush().await.map_err(VmShutdownError::SerialError)?
            }
        }

        vm.vmm_process
            .wait_for_exit()
            .await
            .map_err(VmShutdownError::WaitForExitError)
    }
}

/// A shutdown action for a [Vm]. A sequence of these can be applied to attempt to perform a shutdown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VmShutdownAction {
    /// The [VmShutdownMethod] used by this action.
    pub method: VmShutdownMethod,
    /// Optionally, a timeout for how long the action can take. If one is specified, the action future
    /// will be wrapped in a timeout future, thus not letting the shutdown hang. Otherwise, the
    /// future will be awaited normally with the possibility of hanging.
    pub timeout: Option<Duration>,
    /// Whether this action should be marked as graceful or not. This will reflect in the [VmShutdownOutcome]
    /// and can be used for diagnostic purposes.
    pub graceful: bool,
}

// Allow a single action to be passed by internally converting it to a Once iterator.
impl IntoIterator for VmShutdownAction {
    type Item = VmShutdownAction;

    type IntoIter = std::iter::Once<VmShutdownAction>;

    fn into_iter(self) -> Self::IntoIter {
        std::iter::once(self)
    }
}

/// An error that can occur while applying a sequence of [VmShutdownAction]s to a [Vm].
#[derive(Debug)]
pub enum VmShutdownError {
    StateCheckError(VmStateCheckError),
    NoActionsSpecified,
    Timeout,
    WaitForExitError(VmmProcessError),
    KillError(VmmProcessError),
    PauseError(VmApiError),
    SendCtrlAltDelError(VmmProcessError),
    TakePipesError(VmmProcessError),
    SerialError(std::io::Error),
}

impl std::error::Error for VmShutdownError {}

impl std::fmt::Display for VmShutdownError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VmShutdownError::StateCheckError(err) => {
                write!(f, "Checking the state of the VM for shutdown failed: {err}")
            }
            VmShutdownError::NoActionsSpecified => write!(f, "No shutdown actions were specified"),
            VmShutdownError::Timeout => write!(
                f,
                "The shutdown action future timed out according to the configured duration"
            ),
            VmShutdownError::WaitForExitError(err) => {
                write!(f, "Waiting for the VMM process to exit failed: {err}")
            }
            VmShutdownError::KillError(err) => write!(f, "Sending a SIGKILL failed: {err}"),
            VmShutdownError::PauseError(err) => write!(f, "Pausing the VM via the API server failed: {err}"),
            VmShutdownError::SendCtrlAltDelError(err) => write!(f, "Sending Ctrl+Alt+Del to the VM failed: {err}"),
            VmShutdownError::TakePipesError(err) => write!(
                f,
                "Taking the pipes from the VM to perform a serial write failed: {err}"
            ),
            VmShutdownError::SerialError(err) => write!(f, "Performing a serial write to stdin failed: {err}"),
        }
    }
}

/// A diagnostic outcome of a successful shutdown of a VM as a result of applying a sequence of
/// [VmShutdownAction]s.
#[derive(Debug)]
pub struct VmShutdownOutcome {
    /// The [ExitStatus] of the VMM process.
    pub exit_status: ExitStatus,
    /// Whether the action that performed the shutdown was marked as graceful.
    pub graceful: bool,
    /// The index of the action that performed the shutdown relative to the sequence of actions.
    pub index: usize,
    /// The recording of all errors that occurred prior to the successful shutdown.
    pub errors: Vec<VmShutdownError>,
}

impl VmShutdownOutcome {
    /// Whether the shutdown was "fully graceful": the action that performed it was marked as graceful
    /// and the [ExitStatus] of the process is successful (equal to zero).
    pub fn fully_graceful(&self) -> bool {
        self.graceful && self.exit_status.success()
    }
}

pub(super) async fn apply<E: VmmExecutor, S: ProcessSpawner, R: Runtime>(
    vm: &mut Vm<E, S, R>,
    actions: impl IntoIterator<Item = VmShutdownAction>,
) -> Result<VmShutdownOutcome, VmShutdownError> {
    vm.ensure_paused_or_running()
        .map_err(VmShutdownError::StateCheckError)?;
    let mut errors = Vec::new();

    for (index, action) in actions.into_iter().enumerate() {
        let result = match action.timeout {
            Some(duration) => vm
                .runtime
                .clone()
                .timeout(duration, action.method.run(vm))
                .await
                .unwrap_or(Err(VmShutdownError::Timeout)),
            None => action.method.run(vm).await,
        };

        match result {
            Ok(exit_status) => {
                return Ok(VmShutdownOutcome {
                    exit_status,
                    index,
                    graceful: action.graceful,
                    errors,
                })
            }
            Err(error) => {
                errors.push(error);
            }
        }
    }

    match errors.into_iter().last() {
        Some(error) => Err(error),
        None => Err(VmShutdownError::NoActionsSpecified),
    }
}
