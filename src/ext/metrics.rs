use std::path::Path;

use serde::{Deserialize, Serialize};
use tokio::{
    fs::File,
    io::{AsyncBufReadExt, BufReader},
    sync::mpsc::{self, error::SendError},
    task::JoinHandle,
};

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MetricsEntry {
    pub utc_timestamp_ms: u64,
    pub api_server: ApiServerMetrics,
    pub balloon: BalloonMetrics,
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ApiServerMetrics {
    pub process_startup_time_us: u64,
    pub process_startup_time_cpu_us: u64,
    pub sync_response_fails: u64,
    pub sync_vmm_send_timeout_count: u64,
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BalloonMetrics {
    pub activate_fails: u64,
    pub inflate_count: u64,
    pub stats_updates_count: u64,
    pub stats_update_fails: u64,
    pub deflate_count: u64,
    pub event_fails: u64,
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MetricsAggregate {
    pub min_us: u64,
    pub max_us: u64,
    pub sum_us: u64,
}

pub enum MetricsTaskError {
    Io(tokio::io::Error),
    Serde(serde_json::Error),
    Send(SendError<MetricsEntry>),
}

pub struct MetricsTask {
    pub join_handle: JoinHandle<Result<(), MetricsTaskError>>,
    pub receiver: mpsc::UnboundedReceiver<MetricsEntry>,
}

pub fn spawn_metrics_task(metrics_path: impl AsRef<Path> + Send + 'static) -> MetricsTask {
    let (sender, receiver) = mpsc::unbounded_channel();

    let join_handle = tokio::task::spawn(async move {
        let mut buf_reader = BufReader::new(
            File::open(metrics_path)
                .await
                .map_err(MetricsTaskError::Io)?,
        )
        .lines();
        loop {
            let line = match buf_reader.next_line().await {
                Ok(Some(line)) => line,
                Ok(None) => continue,
                Err(err) => return Err(MetricsTaskError::Io(err)),
            };
            let metrics_entry =
                serde_json::from_str::<MetricsEntry>(&line).map_err(MetricsTaskError::Serde)?;
            sender.send(metrics_entry).map_err(MetricsTaskError::Send)?;
        }
    });

    MetricsTask {
        join_handle,
        receiver,
    }
}
