use std::{net::Ipv4Addr, path::PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReprAction {
    pub action_type: ReprActionType,
}

#[derive(Deserialize, Serialize, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum ReprActionType {
    FlushMetrics,
    InstanceStart,
    SendCtrlAltDel,
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Eq)]
pub struct BalloonDevice {
    amount_mib: i32,
    deflate_on_oom: bool,
    stats_polling_interval_s: i32,
}

impl BalloonDevice {
    pub fn new(amount_mib: i32, deflate_on_oom: bool) -> Self {
        Self {
            amount_mib,
            deflate_on_oom,
            stats_polling_interval_s: 0,
        }
    }

    pub fn stats_polling_interval_s(mut self, interval: i32) -> Self {
        self.stats_polling_interval_s = interval;
        self
    }

    pub fn get_amount_mib(&self) -> i32 {
        self.amount_mib
    }

    pub fn get_deflate_on_oom(&self) -> bool {
        self.deflate_on_oom
    }

    pub fn get_stats_polling_interval_s(&self) -> i32 {
        self.stats_polling_interval_s
    }
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Eq)]
pub struct UpdateBalloonDevice {
    amount_mib: u16,
}

impl UpdateBalloonDevice {
    pub fn new(amount_mib: u16) -> Self {
        Self { amount_mib }
    }
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Eq)]
pub struct BalloonStatistics {
    pub target_pages: u32,
    pub actual_pages: u32,
    pub target_mib: u32,
    pub actual_mib: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub swap_in: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub swap_out: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub major_faults: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub minor_faults: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub free_memory: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub available_memory: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disk_caches: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hugetlb_allocations: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hugetlb_failures: Option<u64>,
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Eq)]
pub struct UpdateBalloonStatistics {
    stats_polling_interval_s: u16,
}

impl UpdateBalloonStatistics {
    pub fn new(stats_polling_interval_s: u16) -> Self {
        Self {
            stats_polling_interval_s,
        }
    }
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Eq)]
pub struct BootSource {
    pub(crate) kernel_image_path: PathBuf,
    #[serde(skip_serializing_if = "Option::is_none")]
    boot_args: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) initrd_path: Option<PathBuf>,
}

impl BootSource {
    pub fn new(kernel_image_path: impl Into<PathBuf>) -> Self {
        Self {
            kernel_image_path: kernel_image_path.into(),
            boot_args: None,
            initrd_path: None,
        }
    }

    pub fn boot_args(mut self, boot_args: impl Into<String>) -> Self {
        self.boot_args = Some(boot_args.into());
        self
    }

    pub fn initrd_path(mut self, initrd_path: impl Into<PathBuf>) -> Self {
        self.initrd_path = Some(initrd_path.into());
        self
    }
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Eq)]
pub struct Drive {
    pub(crate) drive_id: String,
    is_root_device: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    cache_type: Option<DriveCacheType>,
    #[serde(skip_serializing_if = "Option::is_none")]
    partuuid: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    is_read_only: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) path_on_host: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    rate_limiter: Option<RateLimiter>,
    #[serde(skip_serializing_if = "Option::is_none")]
    io_engine: Option<DriveIoEngine>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) socket: Option<PathBuf>,
}

impl Drive {
    pub fn new(drive_id: impl Into<String>, is_root_device: bool) -> Self {
        Self {
            drive_id: drive_id.into(),
            is_root_device,
            cache_type: None,
            partuuid: None,
            is_read_only: None,
            path_on_host: None,
            rate_limiter: None,
            io_engine: None,
            socket: None,
        }
    }

    pub fn cache_type(mut self, cache_type: DriveCacheType) -> Self {
        self.cache_type = Some(cache_type);
        self
    }

    pub fn partuuid(mut self, partuuid: impl Into<String>) -> Self {
        self.partuuid = Some(partuuid.into());
        self
    }

    pub fn is_read_only(mut self, is_read_only: bool) -> Self {
        self.is_read_only = Some(is_read_only);
        self
    }

    pub fn path_on_host(mut self, path_on_host: impl Into<PathBuf>) -> Self {
        self.path_on_host = Some(path_on_host.into());
        self
    }

    pub fn rate_limiter(mut self, rate_limiter: RateLimiter) -> Self {
        self.rate_limiter = Some(rate_limiter);
        self
    }

    pub fn io_engine(mut self, io_engine: DriveIoEngine) -> Self {
        self.io_engine = Some(io_engine);
        self
    }

    pub fn socket(mut self, socket: impl Into<PathBuf>) -> Self {
        self.socket = Some(socket.into());
        self
    }
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Eq)]
pub struct UpdateDrive {
    pub(crate) drive_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    path_on_host: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    rate_limiter: Option<RateLimiter>,
}

impl UpdateDrive {
    pub fn new(drive_id: impl Into<String>) -> Self {
        Self {
            drive_id: drive_id.into(),
            path_on_host: None,
            rate_limiter: None,
        }
    }

    pub fn path_on_host(mut self, path_on_host: impl Into<PathBuf>) -> Self {
        self.path_on_host = Some(path_on_host.into());
        self
    }

    pub fn rate_limiter(mut self, rate_limiter: RateLimiter) -> Self {
        self.rate_limiter = Some(rate_limiter);
        self
    }
}

#[derive(Deserialize, Serialize, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum DriveCacheType {
    Unsafe,
    Writeback,
}

#[derive(Deserialize, Serialize, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum DriveIoEngine {
    Sync,
    Async,
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Eq)]
pub struct RateLimiter {
    bandwidth: TokenBucket,
    ops: TokenBucket,
}

impl RateLimiter {
    pub fn new(bandwidth: TokenBucket, ops: TokenBucket) -> Self {
        Self { bandwidth, ops }
    }
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Eq)]
pub struct TokenBucket {
    size: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    one_time_burst: Option<u64>,
    refill_time: u64,
}

impl TokenBucket {
    pub fn new(size: u64, refill_time: u64) -> Self {
        Self {
            size,
            one_time_burst: None,
            refill_time,
        }
    }

    pub fn one_time_burst(mut self, one_time_burst: u64) -> Self {
        self.one_time_burst = Some(one_time_burst);
        self
    }
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Eq)]
pub struct LoggerSystem {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) log_path: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    level: Option<LogLevel>,
    #[serde(skip_serializing_if = "Option::is_none")]
    show_level: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    show_log_origin: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    module: Option<String>,
}

impl LoggerSystem {
    pub fn new() -> Self {
        Self {
            log_path: None,
            level: None,
            show_level: None,
            show_log_origin: None,
            module: None,
        }
    }

    pub fn log_path(mut self, log_path: impl Into<PathBuf>) -> Self {
        self.log_path = Some(log_path.into());
        self
    }

    pub fn level(mut self, level: LogLevel) -> Self {
        self.level = Some(level);
        self
    }

    pub fn show_level(mut self, show_level: bool) -> Self {
        self.show_level = Some(show_level);
        self
    }

    pub fn show_log_origin(mut self, show_log_origin: bool) -> Self {
        self.show_log_origin = Some(show_log_origin);
        self
    }

    pub fn module(mut self, module: impl Into<String>) -> Self {
        self.module = Some(module.into());
        self
    }
}

#[derive(Deserialize, Serialize, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LogLevel {
    Off,
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Eq)]
pub struct MachineConfiguration {
    vcpu_count: u8,
    mem_size_mib: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    smt: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    track_dirty_pages: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    huge_pages: Option<HugePages>,
}

impl MachineConfiguration {
    pub fn new(vcpu_count: u8, mem_size_mib: usize) -> Self {
        Self {
            vcpu_count,
            mem_size_mib,
            smt: None,
            track_dirty_pages: None,
            huge_pages: None,
        }
    }

    pub fn smt(mut self, smt: bool) -> Self {
        self.smt = Some(smt);
        self
    }

    pub fn track_dirty_pages(mut self, track_dirty_pages: bool) -> Self {
        self.track_dirty_pages = Some(track_dirty_pages);
        self
    }

    pub fn huge_pages(mut self, huge_pages: HugePages) -> Self {
        self.huge_pages = Some(huge_pages);
        self
    }

    pub fn get_vcpu_count(&self) -> u8 {
        self.vcpu_count
    }

    pub fn get_mem_size_mib(&self) -> usize {
        self.mem_size_mib
    }

    pub fn get_smt(&self) -> Option<bool> {
        self.smt
    }

    pub fn get_track_dirty_pages(&self) -> Option<bool> {
        self.track_dirty_pages
    }

    pub fn get_huge_pages(&self) -> Option<HugePages> {
        self.huge_pages
    }
}

#[derive(Deserialize, Serialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum HugePages {
    None,
    #[serde(rename = "2M")]
    Hugetlbfs2M,
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Eq)]
pub struct MetricsSystem {
    pub(crate) metrics_path: PathBuf,
}

impl MetricsSystem {
    pub fn new(metrics_path: impl Into<PathBuf>) -> Self {
        Self {
            metrics_path: metrics_path.into(),
        }
    }
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Eq)]
pub struct MmdsConfiguration {
    version: MmdsVersion,
    network_interfaces: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    ipv4_address: Option<Ipv4Addr>,
}

impl MmdsConfiguration {
    pub fn new(version: MmdsVersion, network_interfaces: Vec<String>) -> Self {
        Self {
            version,
            network_interfaces,
            ipv4_address: None,
        }
    }

    pub fn ipv4_address(mut self, ipv4_address: Ipv4Addr) -> Self {
        self.ipv4_address = Some(ipv4_address);
        self
    }
}

#[derive(Deserialize, Serialize, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum MmdsVersion {
    V1,
    V2,
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Eq)]
pub struct EntropyDevice {
    #[serde(skip_serializing_if = "Option::is_none")]
    rate_limiter: Option<RateLimiter>,
}

impl EntropyDevice {
    pub fn new() -> Self {
        Self { rate_limiter: None }
    }

    pub fn rate_limiter(mut self, rate_limiter: RateLimiter) -> Self {
        self.rate_limiter = Some(rate_limiter);
        self
    }
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Eq)]
pub struct NetworkInterface {
    pub(crate) iface_id: String,
    host_dev_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    guest_mac: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    rx_rate_limiter: Option<RateLimiter>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tx_rate_limiter: Option<RateLimiter>,
}

impl NetworkInterface {
    pub fn new(iface_id: impl Into<String>, host_dev_name: impl Into<String>) -> Self {
        Self {
            iface_id: iface_id.into(),
            host_dev_name: host_dev_name.into(),
            guest_mac: None,
            rx_rate_limiter: None,
            tx_rate_limiter: None,
        }
    }

    pub fn guest_mac(mut self, guest_mac: impl Into<String>) -> Self {
        self.guest_mac = Some(guest_mac.into());
        self
    }

    pub fn rx_rate_limiter(mut self, rx_rate_limiter: RateLimiter) -> Self {
        self.rx_rate_limiter = Some(rx_rate_limiter);
        self
    }

    pub fn tx_rate_limiter(mut self, tx_rate_limiter: RateLimiter) -> Self {
        self.tx_rate_limiter = Some(tx_rate_limiter);
        self
    }
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Eq)]
pub struct UpdateNetworkInterface {
    pub(crate) iface_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    rx_rate_limiter: Option<RateLimiter>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tx_rate_limiter: Option<RateLimiter>,
}

impl UpdateNetworkInterface {
    pub fn new(iface_id: impl Into<String>) -> Self {
        Self {
            iface_id: iface_id.into(),
            rx_rate_limiter: None,
            tx_rate_limiter: None,
        }
    }

    pub fn rx_rate_limiter(mut self, rx_rate_limiter: RateLimiter) -> Self {
        self.rx_rate_limiter = Some(rx_rate_limiter);
        self
    }

    pub fn tx_rate_limiter(mut self, tx_rate_limiter: RateLimiter) -> Self {
        self.tx_rate_limiter = Some(tx_rate_limiter);
        self
    }
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Eq)]
pub struct CreateSnapshot {
    #[serde(skip_serializing_if = "Option::is_none")]
    snapshot_type: Option<SnapshotType>,
    pub(crate) snapshot_path: PathBuf,
    pub(crate) mem_file_path: PathBuf,
}

impl CreateSnapshot {
    pub fn new(snapshot_path: impl Into<PathBuf>, mem_file_path: impl Into<PathBuf>) -> Self {
        Self {
            snapshot_type: None,
            snapshot_path: snapshot_path.into(),
            mem_file_path: mem_file_path.into(),
        }
    }

    pub fn snapshot_type(mut self, snapshot_type: SnapshotType) -> Self {
        self.snapshot_type = Some(snapshot_type);
        self
    }
}

#[derive(Deserialize, Serialize, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SnapshotType {
    Full,
    Diff,
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Eq)]
pub struct LoadSnapshot {
    #[serde(skip_serializing_if = "Option::is_none")]
    enable_diff_snapshots: Option<bool>,
    pub(crate) mem_backend: MemoryBackend,
    pub(crate) snapshot_path: PathBuf,
    #[serde(skip_serializing_if = "Option::is_none")]
    resume_vm: Option<bool>,
}

impl LoadSnapshot {
    pub fn new(snapshot_path: impl Into<PathBuf>, mem_backend: MemoryBackend) -> Self {
        Self {
            enable_diff_snapshots: None,
            mem_backend,
            snapshot_path: snapshot_path.into(),
            resume_vm: None,
        }
    }

    pub fn enable_diff_snapshots(mut self, enable_diff_snapshots: bool) -> Self {
        self.enable_diff_snapshots = Some(enable_diff_snapshots);
        self
    }

    pub fn resume_vm(mut self, resume_vm: bool) -> Self {
        self.resume_vm = Some(resume_vm);
        self
    }
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Eq)]
pub struct MemoryBackend {
    backend_type: MemoryBackendType,
    pub(crate) backend_path: PathBuf,
}

impl MemoryBackend {
    pub fn new(backend_type: MemoryBackendType, backend_path: impl Into<PathBuf>) -> Self {
        Self {
            backend_type,
            backend_path: backend_path.into(),
        }
    }
}

#[derive(Deserialize, Serialize, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum MemoryBackendType {
    File,
    Uffd,
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReprFirecrackerVersion {
    pub firecracker_version: String,
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReprUpdateState {
    pub state: ReprUpdatedState,
}

#[derive(Deserialize, Serialize, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum ReprUpdatedState {
    Paused,
    Resumed,
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Eq)]
pub struct VsockDevice {
    guest_cid: u32,
    pub(crate) uds_path: PathBuf,
}

impl VsockDevice {
    pub fn new(guest_cid: u32, uds_path: impl Into<PathBuf>) -> Self {
        Self {
            guest_cid,
            uds_path: uds_path.into(),
        }
    }
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Eq)]
pub struct VmInfo {
    pub id: String,
    #[serde(rename = "state")]
    pub is_paused: IsPaused,
    pub vmm_version: String,
    pub app_name: String,
}

#[derive(Deserialize, Serialize, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum IsPaused {
    Running,
    Paused,
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Eq)]
pub struct EffectiveConfiguration {
    pub balloon: Option<BalloonDevice>,
    pub drives: Vec<Drive>,
    #[serde(rename = "boot-source")]
    pub boot_source: Option<BootSource>,
    pub logger: Option<LoggerSystem>,
    #[serde(rename = "machine-config")]
    pub machine_configuration: Option<MachineConfiguration>,
    pub metrics: Option<MetricsSystem>,
    #[serde(rename = "mmds-config")]
    pub mmds_configuration: Option<MmdsConfiguration>,
    #[serde(rename = "network-interfaces")]
    pub network_interfaces: Vec<NetworkInterface>,
    pub vsock: Option<VsockDevice>,
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Eq)]
pub struct CpuTemplate {
    pub kvm_capabilities: Vec<String>,
    #[cfg(target_arch = "x86_64")]
    pub cpuid_modifiers: Vec<CpuidModifier>,
    #[cfg(target_arch = "x86_64")]
    pub msr_modifiers: Vec<MsrModifier>,
    #[cfg(target_arch = "aarch64")]
    pub vcpu_features: Vec<VcpuFeature>,
    #[cfg(target_arch = "aarch64")]
    pub reg_modifiers: Vec<RegModifier>,
}

#[cfg(target_arch = "x86_64")]
#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Eq)]
pub struct CpuidModifier {
    pub leaf: String,
    pub subleaf: String,
    pub flags: u32,
    pub modifiers: Vec<CpuidRegisterModifier>,
}

#[cfg(target_arch = "x86_64")]
#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Eq)]
pub struct CpuidRegisterModifier {
    pub register: String,
    pub bitmap: String,
}

#[cfg(target_arch = "x86_64")]
#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Eq)]
pub struct MsrModifier {
    pub addr: String,
    pub bitmap: String,
}

#[cfg(target_arch = "aarch64")]
#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Eq)]
pub struct VcpuFeature {
    pub index: u32,
    pub bitmap: String,
}

#[cfg(target_arch = "aarch64")]
#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Eq)]
pub struct RegModifier {
    pub addr: String,
    pub bitmap: String,
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct ReprApiError {
    pub fault_message: String,
}
