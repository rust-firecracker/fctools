use std::path::PathBuf;

use futures_channel::mpsc;
use futures_util::{AsyncBufReadExt, SinkExt, StreamExt, io::BufReader};
use serde::{Deserialize, Serialize};

use crate::runtime::Runtime;

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Metrics {
    pub utc_timestamp_ms: u64,
    pub api_server: ApiServerMetrics,
    pub balloon: BalloonMetrics,
    pub block: BlockMetrics,
    pub deprecated_api: DeprecatedApiMetrics,
    pub get_api_requests: GetApiRequestsMetrics,
    pub patch_api_requests: PatchApiRequestsMetrics,
    pub put_api_requests: PutApiRequestsMetrics,
    pub i8042: I8042Metrics,
    pub uart: UartMetrics,
    pub latencies_us: LatencyMetrics,
    pub logger: LoggerMetrics,
    pub mmds: MmdsMetrics,
    pub net: NetMetrics,
    pub seccomp: SeccompMetrics,
    pub vcpu: VcpuMetrics,
    pub vmm: VmmMetrics,
    pub signals: SignalsMetrics,
    pub vsock: VsockMetrics,
    pub entropy: EntropyMetrics,
    #[serde(default)]
    pub rtc: Option<RtcMetrics>,
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
pub struct BlockMetrics {
    pub activate_fails: u64,
    pub cfg_fails: u64,
    pub no_avail_buffer: u64,
    pub event_fails: u64,
    pub execute_fails: u64,
    pub invalid_reqs_count: u64,
    pub flush_count: u64,
    pub queue_event_count: u64,
    pub rate_limiter_event_count: u64,
    pub update_count: u64,
    pub update_fails: u64,
    pub read_bytes: u64,
    pub write_bytes: u64,
    pub read_count: u64,
    pub write_count: u64,
    pub read_agg: MetricsAggregate,
    pub write_agg: MetricsAggregate,
    pub rate_limiter_throttled_events: u64,
    pub io_engine_throttled_events: u64,
    pub remaining_reqs_count: u64,
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DeprecatedApiMetrics {
    pub deprecated_http_api_calls: u64,
    pub deprecated_cmd_line_api_calls: u64,
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GetApiRequestsMetrics {
    pub instance_info_count: u64,
    pub machine_cfg_count: u64,
    pub mmds_count: u64,
    pub vmm_version_count: u64,
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PatchApiRequestsMetrics {
    pub drive_count: u64,
    pub drive_fails: u64,
    pub network_count: u64,
    pub network_fails: u64,
    pub machine_cfg_count: u64,
    pub machine_cfg_fails: u64,
    pub mmds_count: u64,
    pub mmds_fails: u64,
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PutApiRequestsMetrics {
    pub actions_count: u64,
    pub actions_fails: u64,
    pub boot_source_count: u64,
    pub boot_source_fails: u64,
    pub drive_count: u64,
    pub drive_fails: u64,
    pub logger_count: u64,
    pub logger_fails: u64,
    pub machine_cfg_count: u64,
    pub machine_cfg_fails: u64,
    pub cpu_cfg_count: u64,
    pub cpu_cfg_fails: u64,
    pub metrics_count: u64,
    pub metrics_fails: u64,
    pub network_count: u64,
    pub network_fails: u64,
    pub mmds_count: u64,
    pub mmds_fails: u64,
    pub vsock_count: u64,
    pub vsock_fails: u64,
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct I8042Metrics {
    pub error_count: u64,
    pub missed_read_count: u64,
    pub missed_write_count: u64,
    pub read_count: u64,
    pub write_count: u64,
    pub reset_count: u64,
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct UartMetrics {
    pub error_count: u64,
    pub flush_count: u64,
    pub missed_read_count: u64,
    pub missed_write_count: u64,
    pub read_count: u64,
    pub write_count: u64,
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LatencyMetrics {
    pub full_create_snapshot: u64,
    pub diff_create_snapshot: u64,
    pub load_snapshot: u64,
    pub pause_vm: u64,
    pub resume_vm: u64,
    pub vmm_full_create_snapshot: u64,
    pub vmm_diff_create_snapshot: u64,
    pub vmm_load_snapshot: u64,
    pub vmm_pause_vm: u64,
    pub vmm_resume_vm: u64,
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LoggerMetrics {
    pub missed_metrics_count: u64,
    pub metrics_fails: u64,
    pub missed_log_count: u64,
    pub log_fails: u64,
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MmdsMetrics {
    pub rx_accepted: u64,
    pub rx_accepted_err: u64,
    pub rx_accepted_unusual: u64,
    pub rx_bad_eth: u64,
    pub rx_invalid_token: u64,
    pub rx_no_token: u64,
    pub rx_count: u64,
    pub tx_bytes: u64,
    pub tx_count: u64,
    pub tx_errors: u64,
    pub tx_frames: u64,
    pub connections_created: u64,
    pub connections_destroyed: u64,
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NetMetrics {
    pub activate_fails: u64,
    pub cfg_fails: u64,
    pub mac_address_updates: u64,
    pub no_rx_avail_buffer: u64,
    pub no_tx_avail_buffer: u64,
    pub event_fails: u64,
    pub rx_queue_event_count: u64,
    pub rx_event_rate_limiter_count: u64,
    pub rx_partial_writes: u64,
    pub rx_rate_limiter_throttled: u64,
    pub rx_tap_event_count: u64,
    pub rx_bytes_count: u64,
    pub rx_packets_count: u64,
    pub rx_fails: u64,
    pub rx_count: u64,
    pub tap_read_fails: u64,
    pub tap_write_fails: u64,
    pub tap_write_agg: MetricsAggregate,
    pub tx_bytes_count: u64,
    pub tx_malformed_frames: u64,
    pub tx_fails: u64,
    pub tx_count: u64,
    pub tx_packets_count: u64,
    pub tx_partial_reads: u64,
    pub tx_queue_event_count: u64,
    pub tx_rate_limiter_event_count: u64,
    pub tx_rate_limiter_throttled: u64,
    pub tx_spoofed_mac_count: u64,
    pub tx_remaining_reqs_count: u64,
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SeccompMetrics {
    pub num_faults: u64,
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct VcpuMetrics {
    pub exit_io_in: u64,
    pub exit_io_out: u64,
    pub exit_mmio_read: u64,
    pub exit_mmio_write: u64,
    pub failures: u64,
    pub exit_io_in_agg: MetricsAggregate,
    pub exit_io_out_agg: MetricsAggregate,
    pub exit_mmio_read_agg: MetricsAggregate,
    pub exit_mmio_write_agg: MetricsAggregate,
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct VmmMetrics {
    pub device_events: u64,
    pub panic_count: u64,
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SignalsMetrics {
    pub sigbus: u64,
    pub sigsegv: u64,
    pub sigxfsz: u64,
    pub sigxcpu: u64,
    pub sigpipe: u64,
    pub sighup: u64,
    pub sigill: u64,
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct VsockMetrics {
    pub activate_fails: u64,
    pub cfg_fails: u64,
    pub rx_queue_event_fails: u64,
    pub tx_queue_event_fails: u64,
    pub ev_queue_event_fails: u64,
    pub muxer_event_fails: u64,
    pub conn_event_fails: u64,
    pub rx_queue_event_count: u64,
    pub tx_queue_event_count: u64,
    pub rx_bytes_count: u64,
    pub tx_bytes_count: u64,
    pub rx_packets_count: u64,
    pub tx_packets_count: u64,
    pub conns_added: u64,
    pub conns_killed: u64,
    pub conns_removed: u64,
    pub killq_resync: u64,
    pub tx_flush_fails: u64,
    pub tx_write_fails: u64,
    pub rx_read_fails: u64,
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EntropyMetrics {
    pub activate_fails: u64,
    pub entropy_event_fails: u64,
    pub entropy_event_count: u64,
    pub entropy_bytes: u64,
    pub host_rng_fails: u64,
    pub entropy_rate_limiter_throttled: u64,
    pub rate_limiter_event_count: u64,
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RtcMetrics {
    pub error_count: u64,
    pub missed_read_count: u64,
    pub missed_write_count: u64,
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MetricsAggregate {
    pub min_us: u64,
    pub max_us: u64,
    pub sum_us: u64,
}

/// An error that the dedicated metrics async task can fail with.
#[derive(Debug)]
pub enum MetricsTaskError {
    /// An I/O error occurred while either opening the metrics file/pipe in read-only mode or reading from it.
    FilesystemError(std::io::Error),
    /// An error occurred while trying to deserialize the metrics line received from the metrics file/pipe.
    SerdeError(serde_json::Error),
    /// An error occurred while sending the deserialized [Metrics] object into the [mpsc] channel.
    SendError(mpsc::SendError),
}

impl std::error::Error for MetricsTaskError {}

impl std::fmt::Display for MetricsTaskError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MetricsTaskError::FilesystemError(err) => {
                write!(f, "A filesystem operation backed by the runtime failed: {err}")
            }
            MetricsTaskError::SerdeError(err) => write!(f, "Deserializing the metrics JSON failed: {err}"),
            MetricsTaskError::SendError(err) => write!(f, "Sending the metrics to the channel failed: {err}"),
        }
    }
}

/// A spawned async task that gathers Firecracker's metrics.
#[derive(Debug)]
pub struct MetricsTask<R: Runtime> {
    /// The task that can be detached, cancelled or joined on.
    pub task: R::Task<Result<(), MetricsTaskError>>,
    /// An asynchronous [mpsc::Receiver] that can be used to fetch the metrics sent out by the task.
    pub receiver: mpsc::Receiver<Metrics>,
}

/// Spawn a dedicated async task that gathers Firecracker's metrics from the given metrics path with an
/// asynchronous [mpsc] channel limited by the provided upper bound (buffer), using the provided [Runtime].
pub fn spawn_metrics_task<R: Runtime, P: Into<PathBuf>>(metrics_path: P, buffer: usize, runtime: R) -> MetricsTask<R> {
    let (mut sender, receiver) = mpsc::channel(buffer);
    let metrics_path = metrics_path.into();

    let task = runtime.clone().spawn_task(async move {
        let mut buf_reader = BufReader::new(
            runtime
                .fs_open_file_for_read(&metrics_path)
                .await
                .map_err(MetricsTaskError::FilesystemError)?,
        )
        .lines();

        loop {
            let line = match buf_reader.next().await {
                Some(Ok(line)) => line,
                None => return Ok(()),
                Some(Err(err)) => return Err(MetricsTaskError::FilesystemError(err)),
            };

            let metrics_entry = serde_json::from_str::<Metrics>(&line).map_err(MetricsTaskError::SerdeError)?;
            sender.send(metrics_entry).await.map_err(MetricsTaskError::SendError)?;
        }
    });

    MetricsTask { task, receiver }
}
