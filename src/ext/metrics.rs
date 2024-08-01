use std::{
    path::Path,
    sync::{Arc, Mutex},
};

use serde::{Deserialize, Serialize};
use tokio::{
    fs::File,
    io::{AsyncBufReadExt, BufReader},
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

pub trait MetricsBuffer {
    fn recv_metrics(&mut self, metrics_entry: MetricsEntry);
}

pub struct InMemoryMetricsBuffer {
    vec: Vec<MetricsEntry>,
}

impl InMemoryMetricsBuffer {
    pub fn new() -> Self {
        Self { vec: Vec::new() }
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            vec: Vec::with_capacity(capacity),
        }
    }
}

impl MetricsBuffer for InMemoryMetricsBuffer {
    fn recv_metrics(&mut self, metrics_entry: MetricsEntry) {
        self.vec.push(metrics_entry);
    }
}

pub enum MetricsTaskError {
    Io(tokio::io::Error),
    Serde(serde_json::Error),
}

pub fn spawn_metrics_task(
    metrics_path: impl AsRef<Path> + Send + 'static,
    buffer: Arc<Mutex<impl MetricsBuffer + Send + 'static>>,
) -> JoinHandle<Result<(), MetricsTaskError>> {
    tokio::spawn(async move {
        let mut buf_reader = BufReader::new(
            File::open(metrics_path)
                .await
                .map_err(MetricsTaskError::Io)?,
        )
        .lines();
        loop {
            let line = match buf_reader.next_line().await {
                Ok(Some(line)) => line,
                _ => break,
            };
            let metrics_entry =
                serde_json::from_str::<MetricsEntry>(&line).map_err(MetricsTaskError::Serde)?;
            buffer.lock().unwrap().recv_metrics(metrics_entry);
        }
        Ok(())
    })
}
