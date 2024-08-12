use tokio::{sync::mpsc, task::JoinHandle};

use crate::process::VmmProcessPipes;

pub struct VmmSerialConsole {
    pipes: VmmProcessPipes,
    stdout_read_mode: PipeReadMode,
    stderr_read_mode: PipeReadMode,
    stdout_join_handle: JoinHandle<()>,
    stderr_join_handle: JoinHandle<()>,
}

pub struct SerialConsoleReader {
    read_mode: PipeReadMode,
    join_handle: JoinHandle<()>,
    error_receiver: mpsc::Receiver<tokio::io::Error>,
}

impl Drop for SerialConsoleReader {
    fn drop(&mut self) {
        self.join_handle.abort();
    }
}

pub enum PipeReadMode {
    LineByLine,
}
