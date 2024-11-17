use std::time::Duration;

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
