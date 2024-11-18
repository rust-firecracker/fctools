use std::{process::ExitStatus, time::Duration};

use tokio::io::AsyncWriteExt;

use crate::{fs_backend::FsBackend, process_spawner::ProcessSpawner, vmm::{executor::VmmExecutor, process::VmmProcessError}};

use super::{api::{VmApi, VmApiError}, Vm, VmStateCheckError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VmShutdownMethod {
    Kill,
    PauseThenKill,
    CtrlAltDel,
    WriteToSerial(Vec<u8>),
}

impl VmShutdownMethod {
    async fn run<E: VmmExecutor, S: ProcessSpawner, F: FsBackend>(&self, vm: &mut Vm<E, S, F>) -> Result<ExitStatus, VmShutdownError> {
        match self {
            VmShutdownMethod::Kill => vm.vmm_process.send_sigkill().map_err(VmShutdownError::KillError)?,
            VmShutdownMethod::PauseThenKill => {
                vm.api_pause().await.map_err(VmShutdownError::PauseError)?;
                vm.vmm_process.send_sigkill().map_err(VmShutdownError::KillError)?
            },
            VmShutdownMethod::CtrlAltDel => vm.vmm_process.send_ctrl_alt_del().await.map_err(VmShutdownError::SendCtrlAltDelError)?,
            VmShutdownMethod::WriteToSerial(bytes) => {
                let mut pipes = vm.vmm_process.take_pipes().map_err(VmShutdownError::TakePipesError)?;
                pipes.stdin.write_all(&bytes).await.map_err(VmShutdownError::SerialWriteError)?;
            },
        }

        vm.vmm_process.wait_for_exit().await.map_err(VmShutdownError::WaitForExitError)
    }
    
    async fn revert<E: VmmExecutor, S: ProcessSpawner, F: FsBackend>(&self, vm: &mut Vm<E, S, F>) -> Result<(), VmShutdownError> {
        Ok(())
    }
}

pub struct VmShutdownAction {
    method: VmShutdownMethod,
    timeout: Option<Duration>,
    graceful: bool,
    attempt_revert: bool,
}

impl VmShutdownAction {
    pub fn new(method: VmShutdownMethod) -> Self {
        Self {
            method,
            timeout: None,
            graceful: true,
            attempt_revert: false,
        }
    }

    pub fn graceful(mut self, graceful: bool) -> Self {
        self.graceful = graceful;
        self
    }

    pub fn timeout(mut self, duration: Duration) -> Self {
        self.timeout = Some(duration);
        self
    }

    pub fn attempt_revert(mut self, attempt_revert: bool) -> Self {
        self.attempt_revert = attempt_revert;
        self
    }
}

#[derive(Debug)]
pub enum VmShutdownError {
    StateCheckError(VmStateCheckError),
    NoActionsSpecified,
    Timeout,
    WaitForExitError(VmmProcessError),
    KillError(VmmProcessError),
    PauseError(VmApiError),
    ResumeError(VmApiError),
    SendCtrlAltDelError(VmmProcessError),
    TakePipesError(VmmProcessError),
    SerialWriteError(std::io::Error)
}

pub struct VmShutdownOutcome {
    pub exit_status: ExitStatus,
    pub graceful: bool,
    pub errors: Vec<VmShutdownError>,
}

impl VmShutdownOutcome {
    pub fn fully_graceful(&self) -> bool {
        self.graceful && self.exit_status.success()
    }
}

pub(super) async fn apply<E: VmmExecutor, S: ProcessSpawner, F: FsBackend>(vm: &mut Vm<E, S, F>, actions: impl IntoIterator<Item = VmShutdownAction>) -> Result<VmShutdownOutcome, VmShutdownError> {
    vm.ensure_paused_or_running().map_err(VmShutdownError::StateCheckError)?;
    let mut errors = Vec::new();

    for action in actions {
        let result = match action.timeout {
            Some(duration) => {
                tokio::time::timeout(duration, action.method.run(vm)).await.unwrap_or(Err(VmShutdownError::Timeout))
            },
            None => {
                action.method.run(vm).await
            }
        };
        
        match result {
            Ok(exit_status) => {
                return Ok(VmShutdownOutcome { exit_status, graceful: action.graceful, errors })
            },
            Err(error) => {
                errors.push(error);
            }
        }
    }

    match errors.into_iter().last() {
        Some(error) => Err(error),
        None => Err(VmShutdownError::NoActionsSpecified)
    }
}
