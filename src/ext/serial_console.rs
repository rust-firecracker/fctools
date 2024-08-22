use pin_project_lite::pin_project;
use tokio::{
    io::{AsyncBufReadExt, AsyncRead, BufReader},
    process::{ChildStderr, ChildStdin, ChildStdout},
    sync::mpsc,
    task::JoinHandle,
};

use crate::process::VmmProcessPipes;

pub struct VmmSerialConsole {
    stdin: ChildStdin,
    stdout_io: SerialIo,
    stderr_io: SerialIo,
}

impl VmmSerialConsole {
    pub fn new(pipes: VmmProcessPipes) -> Self {
        Self {
            stdin: pipes.stdin,
            stdout_io: SerialIo::new(ChildStdio::Stdout { inner: pipes.stdout }),
            stderr_io: SerialIo::new(ChildStdio::Stderr { inner: pipes.stderr }),
        }
    }

    pub fn stdin(&mut self) -> &mut ChildStdin {
        &mut self.stdin
    }

    pub fn stdout(&mut self) -> &mut SerialIo {
        &mut self.stdout_io
    }

    pub fn stderr(&mut self) -> &mut SerialIo {
        &mut self.stderr_io
    }
}

pub struct SerialIo {
    join_handle: JoinHandle<()>,
    error_recv: mpsc::UnboundedReceiver<tokio::io::Error>,
    data_recv: mpsc::UnboundedReceiver<String>,
}

pin_project! {
    #[project = ChildStdioProj]
    enum ChildStdio {
        Stdout { #[pin] inner: ChildStdout },
        Stderr { #[pin] inner: ChildStderr },
    }
}

impl AsyncRead for ChildStdio {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        match self.project() {
            ChildStdioProj::Stdout { inner } => inner.poll_read(cx, buf),
            ChildStdioProj::Stderr { inner } => inner.poll_read(cx, buf),
        }
    }
}

impl SerialIo {
    fn new(stdio: ChildStdio) -> Self {
        let (error_send, error_recv) = mpsc::unbounded_channel();
        let (data_send, data_recv) = mpsc::unbounded_channel();

        let join_handle = tokio::spawn(async move {
            let mut buf_reader = BufReader::new(stdio).lines();

            loop {
                let line = match buf_reader.next_line().await {
                    Ok(Some(line)) => line,
                    Ok(None) => continue,
                    Err(err) => {
                        let _ = error_send.send(err);
                        continue;
                    }
                };

                let _ = data_send.send(line);
            }
        });

        Self {
            join_handle,
            error_recv,
            data_recv,
        }
    }

    pub async fn expect(&mut self, function: impl Fn(&str) -> bool) -> String {
        loop {
            let line = match self.data_recv.recv().await {
                Some(line) => line,
                None => continue,
            };

            if function(&line) {
                return line;
            }
        }
    }

    pub async fn recv_data(&mut self) -> Option<String> {
        self.data_recv.recv().await
    }

    pub async fn recv_error(&mut self) -> Option<tokio::io::Error> {
        self.error_recv.recv().await
    }
}

impl Drop for SerialIo {
    fn drop(&mut self) {
        self.join_handle.abort();
    }
}
