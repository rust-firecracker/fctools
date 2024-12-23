use std::net::Ipv4Addr;

use serde::{Deserialize, Serialize};

use crate::vmm::{
    arguments::VmmLogLevel,
    resource::{created::CreatedVmmResource, moved::MovedVmmResource, produced::ProducedVmmResource},
};

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
    pub amount_mib: i32,
    pub deflate_on_oom: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stats_polling_interval_s: Option<i32>,
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Eq)]
pub struct UpdateBalloonDevice {
    pub amount_mib: u16,
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
    pub stats_polling_interval_s: u16,
}

#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
pub struct BootSource {
    #[serde(rename = "kernel_image_path")]
    pub kernel_image: MovedVmmResource,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub boot_args: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "initrd_path")]
    pub initrd: Option<MovedVmmResource>,
}

#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
pub struct Drive {
    pub drive_id: String,
    pub is_root_device: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_type: Option<DriveCacheType>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub partuuid: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_read_only: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "path_on_host")]
    pub block: Option<MovedVmmResource>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rate_limiter: Option<RateLimiter>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub io_engine: Option<DriveIoEngine>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub socket: Option<MovedVmmResource>,
}

#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
pub struct UpdateDrive {
    pub drive_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "path_on_host")]
    pub block: Option<MovedVmmResource>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rate_limiter: Option<RateLimiter>,
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
    pub bandwidth: TokenBucket,
    pub ops: TokenBucket,
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Eq)]
pub struct TokenBucket {
    pub size: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub one_time_burst: Option<u64>,
    pub refill_time: u64,
}

#[derive(Serialize, Debug, Clone, PartialEq, Eq, Default)]
pub struct LoggerSystem {
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "log_path")]
    pub logs: Option<CreatedVmmResource>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub level: Option<VmmLogLevel>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub show_level: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub show_log_origin: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub module: Option<String>,
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Eq)]
pub struct MachineConfiguration {
    pub vcpu_count: u8,
    pub mem_size_mib: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub smt: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub track_dirty_pages: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub huge_pages: Option<HugePages>,
}

#[derive(Deserialize, Serialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum HugePages {
    None,
    #[serde(rename = "2M")]
    Hugetlbfs2M,
}

#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
pub struct MetricsSystem {
    #[serde(rename = "metrics_path")]
    pub metrics: CreatedVmmResource,
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Eq)]
pub struct MmdsConfiguration {
    pub version: MmdsVersion,
    pub network_interfaces: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ipv4_address: Option<Ipv4Addr>,
}

#[derive(Deserialize, Serialize, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum MmdsVersion {
    V1,
    V2,
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Eq, Default)]
pub struct EntropyDevice {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rate_limiter: Option<RateLimiter>,
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Eq)]
pub struct NetworkInterface {
    pub iface_id: String,
    pub host_dev_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub guest_mac: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rx_rate_limiter: Option<RateLimiter>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tx_rate_limiter: Option<RateLimiter>,
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Eq)]
pub struct UpdateNetworkInterface {
    pub iface_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rx_rate_limiter: Option<RateLimiter>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tx_rate_limiter: Option<RateLimiter>,
}

#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
pub struct CreateSnapshot {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snapshot_type: Option<SnapshotType>,
    #[serde(rename = "snapshot_path")]
    pub snapshot: ProducedVmmResource,
    #[serde(rename = "mem_file_path")]
    pub mem_file: ProducedVmmResource,
}

#[derive(Deserialize, Serialize, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SnapshotType {
    Full,
    Diff,
}

#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
pub struct LoadSnapshot {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enable_diff_snapshots: Option<bool>,
    pub mem_backend: MemoryBackend,
    #[serde(rename = "snapshot_path")]
    pub snapshot: MovedVmmResource,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resume_vm: Option<bool>,
}

#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
pub struct MemoryBackend {
    pub backend_type: MemoryBackendType,
    #[serde(rename = "backend_path")]
    pub backend: MovedVmmResource,
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

#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
pub struct VsockDevice {
    pub guest_cid: u32,
    #[serde(rename = "uds_path")]
    pub uds: ProducedVmmResource,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Info {
    pub id: String,
    pub is_paused: bool,
    pub vmm_version: String,
    pub app_name: String,
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReprInfo {
    pub id: String,
    #[serde(rename = "state")]
    pub is_paused: ReprIsPaused,
    pub vmm_version: String,
    pub app_name: String,
}

#[derive(Deserialize, Serialize, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum ReprIsPaused {
    Running,
    Paused,
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Eq)]
pub struct CpuTemplate {
    pub kvm_capabilities: Vec<String>,
    #[cfg(target_arch = "x86_64")]
    #[cfg_attr(docsrs, doc(cfg(target_arch = "x86_64")))]
    pub cpuid_modifiers: Vec<CpuidModifier>,
    #[cfg(target_arch = "x86_64")]
    #[cfg_attr(docsrs, doc(cfg(target_arch = "x86_64")))]
    pub msr_modifiers: Vec<MsrModifier>,
    #[cfg(target_arch = "aarch64")]
    #[cfg_attr(docsrs, doc(cfg(target_arch = "aarch64")))]
    pub vcpu_features: Vec<VcpuFeature>,
    #[cfg(target_arch = "aarch64")]
    #[cfg_attr(docsrs, doc(cfg(target_arch = "aarch64")))]
    pub reg_modifiers: Vec<RegModifier>,
}

#[cfg(target_arch = "x86_64")]
#[cfg_attr(docsrs, doc(cfg(target_arch = "x86_64")))]
#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Eq)]
pub struct CpuidModifier {
    pub leaf: String,
    pub subleaf: String,
    pub flags: u32,
    pub modifiers: Vec<CpuidRegisterModifier>,
}

#[cfg(target_arch = "x86_64")]
#[cfg_attr(docsrs, doc(cfg(target_arch = "x86_64")))]
#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Eq)]
pub struct CpuidRegisterModifier {
    pub register: String,
    pub bitmap: String,
}

#[cfg(target_arch = "x86_64")]
#[cfg_attr(docsrs, doc(cfg(target_arch = "x86_64")))]
#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Eq)]
pub struct MsrModifier {
    pub addr: String,
    pub bitmap: String,
}

#[cfg(target_arch = "aarch64")]
#[cfg_attr(docsrs, doc(cfg(target_arch = "aarch64")))]
#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Eq)]
pub struct VcpuFeature {
    pub index: u32,
    pub bitmap: String,
}

#[cfg(target_arch = "aarch64")]
#[cfg_attr(docsrs, doc(cfg(target_arch = "aarch64")))]
#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Eq)]
pub struct RegModifier {
    pub addr: String,
    pub bitmap: String,
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct ReprApiError {
    pub fault_message: String,
}
