use std::path::PathBuf;

use serde::Serialize;

use crate::vm::models::{
    BalloonDevice, BootSource, CpuTemplate, Drive, EntropyDevice, LoadSnapshot, LoggerSystem, MachineConfiguration,
    MemoryHotplugConfiguration, MetricsSystem, MmdsConfiguration, NetworkInterface, PmemDevice, VsockDevice,
};

/// A configuration for a VM, either being new or having been restored from a snapshot. fctools seamlessly exposes
/// the same amount of features for both new and restored VMs, and this layer abstracts away most snapshot-related
/// work.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VmConfiguration {
    /// The VM is new, thus its initialization process is controlled.
    New {
        /// The [InitMethod] used for initializing this VM.
        init_method: InitMethod,
        /// The [VmConfigurationData] tied to this VM.
        data: VmConfigurationData,
    },
    /// The VM is restored from a snapshot, thus its initialization process is derived from that of the snapshot.
    RestoredFromSnapshot {
        /// The [LoadSnapshot] used for loading the snapshot this VM needs to be restored from.
        load_snapshot: LoadSnapshot,
        /// The [VmConfigurationData] tied to this VM. It must be the exact same as the one from the original VM,
        /// which is guaranteed if you use the default snapshot-creating functionality via the API.
        data: VmConfigurationData,
    },
}

impl VmConfiguration {
    /// Get a mutable reference to the [VmConfigurationData] inside this configuration.
    #[inline]
    pub fn get_data_mut(&mut self) -> &mut VmConfigurationData {
        match self {
            VmConfiguration::New { init_method: _, data } => data,
            VmConfiguration::RestoredFromSnapshot { load_snapshot: _, data } => data,
        }
    }

    /// Get a shared reference to the [VmConfigurationData] inside this configuration.
    #[inline]
    pub fn get_data(&self) -> &VmConfigurationData {
        match self {
            VmConfiguration::New { init_method: _, data } => data,
            VmConfiguration::RestoredFromSnapshot { load_snapshot: _, data } => data,
        }
    }
}

/// The full data of various devices associated with a VM. Even when restoring from a snapshot, this information
/// is required for initialization to proceed.
#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
pub struct VmConfigurationData {
    /// The [BootSource] for this VM, mandatory.
    #[serde(rename = "boot-source")]
    pub boot_source: BootSource,
    /// A buffer of all [Drive]s attached to this VM, mandatory.
    pub drives: Vec<Drive>,
    /// A buffer of all [PmemDevice]s attached to this VM, mandatory but can be empty.
    #[serde(rename = "pmem")]
    pub pmem_devices: Vec<PmemDevice>,
    /// The [MachineConfiguration] for this VM, mandatory.
    #[serde(rename = "machine-config")]
    pub machine_configuration: MachineConfiguration,
    /// The [CpuTemplate] for this VM, optional.
    #[serde(rename = "cpu-config")]
    pub cpu_template: Option<CpuTemplate>,
    /// A buffer of all [NetworkInterface]s attached to this VM, mandatory.
    #[serde(rename = "network-interfaces")]
    pub network_interfaces: Vec<NetworkInterface>,
    /// The [BalloonDevice] for this VM, optional.
    #[serde(rename = "balloon")]
    pub balloon_device: Option<BalloonDevice>,
    /// The [VsockDevice] for this VM, optional.
    #[serde(rename = "vsock")]
    pub vsock_device: Option<VsockDevice>,
    /// The [LoggerSystem] for this VM, optional.
    #[serde(rename = "logger")]
    pub logger_system: Option<LoggerSystem>,
    /// The [MetricsSystem] for this VM, optional.
    #[serde(rename = "metrics")]
    pub metrics_system: Option<MetricsSystem>,
    /// The [MemoryHotplugConfiguration] for this VM, optional.
    #[serde(rename = "memory-hotplug")]
    pub memory_hotplug_configuration: Option<MemoryHotplugConfiguration>,
    /// The [MmdsConfiguration] for this VM, optional.
    #[serde(rename = "mmds-config")]
    pub mmds_configuration: Option<MmdsConfiguration>,
    /// The [EntropyDevice] for this VM, optional.
    #[serde(rename = "entropy")]
    pub entropy_device: Option<EntropyDevice>,
}

/// A method of initialization used when booting a new (not restored from snapshot) VM.
/// The performance differences between using both have proven negligible.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub enum InitMethod {
    /// Issue sequential calls to the Management API to perform initialization and boot.
    #[default]
    ViaApiCalls,
    /// Create an intermittent Firecracker JSON configuration that is serialized to the
    /// given virtual path, and pass it to Firecracker in order for initialization and boot
    /// to be performed automatically.
    ViaJsonConfiguration(PathBuf),
}
