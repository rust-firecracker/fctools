use std::path::PathBuf;

use serde::Serialize;

use crate::vmm::resource::VmmResourceReferences;

use super::models::{
    BalloonDevice, BootSource, CpuTemplate, Drive, EntropyDevice, LoadSnapshot, LoggerSystem, MachineConfiguration,
    MetricsSystem, MmdsConfiguration, NetworkInterface, VsockDevice,
};

/// A configuration for a VM, either being new or having been restored from a snapshot. fctools seamlessly exposes
/// the same amount of features for both new and restored VMs, and this layer abstracts away most snapshot-related
/// work.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VmConfiguration {
    /// The VM is new, thus its initialization process is controlled.
    New {
        init_method: InitMethod,
        data: VmConfigurationData,
    },
    /// The VM is restored from a snapshot, thus its initialization process is derived from that of the snapshot.
    RestoredFromSnapshot {
        load_snapshot: LoadSnapshot,
        data: VmConfigurationData,
    },
}

impl VmConfiguration {
    /// Get a mutable reference to the [VmConfigurationData] inside this configuration.
    pub fn data_mut(&mut self) -> &mut VmConfigurationData {
        match self {
            VmConfiguration::New {
                init_method: _,
                ref mut data,
            } => data,
            VmConfiguration::RestoredFromSnapshot {
                load_snapshot: _,
                ref mut data,
            } => data,
        }
    }

    /// Get a shared reference to the [VmConfigurationData] inside this configuration.
    pub fn data(&self) -> &VmConfigurationData {
        match self {
            VmConfiguration::New {
                init_method: _,
                ref data,
            } => data,
            VmConfiguration::RestoredFromSnapshot {
                load_snapshot: _,
                ref data,
            } => data,
        }
    }

    /// Create a set of [VmmResourceReferences] from this [VmConfiguration] by correctly mapping all internal resources
    /// in all configuration structures. This will mutably borrow the [VmConfiguration] and the returned value will be
    /// bound to its lifetime.
    pub fn resource_references(&mut self) -> VmmResourceReferences<'_> {
        let mut references = VmmResourceReferences::new();

        let data = match self {
            VmConfiguration::RestoredFromSnapshot { load_snapshot, data } => {
                references.moved_resources.push(&mut load_snapshot.snapshot);
                references.moved_resources.push(&mut load_snapshot.mem_backend.backend);

                data
            }
            VmConfiguration::New { init_method: _, data } => data,
        };

        references.moved_resources.push(&mut data.boot_source.kernel_image);

        if let Some(ref mut initrd) = data.boot_source.initrd {
            references.moved_resources.push(initrd);
        }

        for drive in &mut data.drives {
            if let Some(ref mut block) = drive.block {
                references.moved_resources.push(block);
            }

            if let Some(ref mut socket) = drive.socket {
                references.moved_resources.push(socket);
            }
        }

        if let Some(ref mut vsock_device) = data.vsock_device {
            references.produced_resources.push(&mut vsock_device.uds);
        }

        if let Some(ref mut logger_system) = data.logger_system {
            if let Some(ref mut logs) = logger_system.logs {
                references.created_resources.push(logs);
            }
        }

        if let Some(ref mut metrics_system) = data.metrics_system {
            references.created_resources.push(&mut metrics_system.metrics);
        }

        references
    }
}

/// The full data of various devices associated with a VM. Even when restoring from a snapshot, this information
/// is required for initialization to proceed.
#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
pub struct VmConfigurationData {
    #[serde(rename = "boot-source")]
    pub boot_source: BootSource,
    pub drives: Vec<Drive>,
    #[serde(rename = "machine-config")]
    pub machine_configuration: MachineConfiguration,
    #[serde(rename = "cpu-config")]
    pub cpu_template: Option<CpuTemplate>,
    #[serde(rename = "network-interfaces")]
    pub network_interfaces: Vec<NetworkInterface>,
    pub balloon_device: Option<BalloonDevice>,
    pub vsock_device: Option<VsockDevice>,
    pub logger_system: Option<LoggerSystem>,
    pub metrics_system: Option<MetricsSystem>,
    #[serde(rename = "mmds-config")]
    pub mmds_configuration: Option<MmdsConfiguration>,
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
    /// given inner path, and pass it to Firecracker in order for initialization and boot
    /// to be performed automatically.
    ViaJsonConfiguration(PathBuf),
}
