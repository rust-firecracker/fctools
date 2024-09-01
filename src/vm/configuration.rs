use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::models::{
    BalloonDevice, BootSource, CpuTemplate, Drive, EntropyDevice, LoadSnapshot, LoggerSystem, MachineConfiguration,
    MetricsSystem, MmdsConfiguration, NetworkInterface, VsockDevice,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VmConfiguration {
    New {
        init_method: InitMethod,
        data: VmConfigurationData,
    },
    RestoredFromSnapshot {
        load_snapshot: LoadSnapshot,
        data: VmConfigurationData,
    },
}

impl VmConfiguration {
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
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct VmConfigurationData {
    #[serde(rename = "boot-source")]
    pub(crate) boot_source: BootSource,
    pub(crate) drives: Vec<Drive>,
    #[serde(rename = "machine-config")]
    pub(crate) machine_configuration: MachineConfiguration,
    #[serde(rename = "cpu-config")]
    pub(crate) cpu_template: Option<CpuTemplate>,
    #[serde(rename = "network-interfaces")]
    pub(crate) network_interfaces: Vec<NetworkInterface>,
    pub(crate) balloon_device: Option<BalloonDevice>,
    pub(crate) vsock_device: Option<VsockDevice>,
    pub(crate) logger_system: Option<LoggerSystem>,
    pub(crate) metrics_system: Option<MetricsSystem>,
    #[serde(rename = "mmds-config")]
    pub(crate) mmds_configuration: Option<MmdsConfiguration>,
    pub(crate) entropy_device: Option<EntropyDevice>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum InitMethod {
    ViaApiCalls,
    ViaJsonConfiguration(PathBuf),
}

impl Default for InitMethod {
    fn default() -> Self {
        InitMethod::ViaApiCalls
    }
}

impl VmConfigurationData {
    pub fn new(boot_source: BootSource, machine_configuration: MachineConfiguration) -> Self {
        Self {
            boot_source,
            drives: vec![],
            machine_configuration,
            cpu_template: None,
            network_interfaces: vec![],
            balloon_device: None,
            vsock_device: None,
            logger_system: None,
            metrics_system: None,
            mmds_configuration: None,
            entropy_device: None,
        }
    }

    pub fn drive(mut self, drive: Drive) -> Self {
        self.drives.push(drive);
        self
    }

    pub fn drives(mut self, drives: impl IntoIterator<Item = Drive>) -> Self {
        self.drives.extend(drives);
        self
    }

    pub fn cpu_template(mut self, cpu_template: CpuTemplate) -> Self {
        self.cpu_template = Some(cpu_template);
        self
    }

    pub fn network_interface(mut self, network_interface: NetworkInterface) -> Self {
        self.network_interfaces.push(network_interface);
        self
    }

    pub fn network_interfaces(mut self, network_interfaces: impl IntoIterator<Item = NetworkInterface>) -> Self {
        self.network_interfaces.extend(network_interfaces);
        self
    }

    pub fn balloon_device(mut self, balloon_device: BalloonDevice) -> Self {
        self.balloon_device = Some(balloon_device);
        self
    }

    pub fn vsock_device(mut self, vsock_device: VsockDevice) -> Self {
        self.vsock_device = Some(vsock_device);
        self
    }

    pub fn logger_system(mut self, logger_system: LoggerSystem) -> Self {
        self.logger_system = Some(logger_system);
        self
    }

    pub fn metrics_system(mut self, metrics_system: MetricsSystem) -> Self {
        self.metrics_system = Some(metrics_system);
        self
    }

    pub fn mmds_configuration(mut self, mmds_configuration: MmdsConfiguration) -> Self {
        self.mmds_configuration = Some(mmds_configuration);
        self
    }

    pub fn entropy_device(mut self, entropy_device: EntropyDevice) -> Self {
        self.entropy_device = Some(entropy_device);
        self
    }

    pub fn get_cpu_template(&self) -> Option<&CpuTemplate> {
        self.cpu_template.as_ref()
    }

    pub fn get_drives(&self) -> &Vec<Drive> {
        &self.drives
    }

    pub fn get_network_interfaces(&self) -> &Vec<NetworkInterface> {
        &self.network_interfaces
    }

    pub fn get_balloon(&self) -> Option<&BalloonDevice> {
        self.balloon_device.as_ref()
    }

    pub fn get_vsock(&self) -> Option<&VsockDevice> {
        self.vsock_device.as_ref()
    }

    pub fn get_logger(&self) -> Option<&LoggerSystem> {
        self.logger_system.as_ref()
    }

    pub fn get_metrics(&self) -> Option<&MetricsSystem> {
        self.metrics_system.as_ref()
    }

    pub fn get_mmds_configuration(&self) -> Option<&MmdsConfiguration> {
        self.mmds_configuration.as_ref()
    }

    pub fn get_entropy(&self) -> Option<&EntropyDevice> {
        self.entropy_device.as_ref()
    }
}
