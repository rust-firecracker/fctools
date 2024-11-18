use std::{process::ExitStatus, time::Duration};

use crate::vmm::process::VmmProcessError;

use super::api::VmApiError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VmShutdownMethod {
    SendSigkill,
    PauseThenKill,
    SendCtrlAltDel,
    WriteToSerial,
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

pub enum VmShutdownError {
    Timeout,
    KillError(VmmProcessError),
    WaitForExitError(VmmProcessError),
    PauseError(VmApiError)
}

pub struct VmShutdownOutcome {
    pub exit_status: ExitStatus,
    pub graceful: bool,
    pub errors: Vec<VmShutdownError>,
}
