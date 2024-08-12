use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::models::{
    VmBalloon, VmBootSource, VmCpuTemplate, VmDrive, VmEntropy, VmLoadSnapshot, VmLogger, VmMachineConfiguration,
    VmMetricsSystem, VmMmdsConfiguration, VmNetworkInterface, VmVsock,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VmConfiguration {
    New(NewVmConfiguration),
    FromSnapshot(FromSnapshotVmConfiguration),
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct NewVmConfiguration {
    #[serde(skip)]
    pub(crate) applier: NewVmConfigurationApplier,
    #[serde(rename = "boot-source")]
    pub(crate) boot_source: VmBootSource,
    pub(crate) drives: Vec<VmDrive>,
    #[serde(rename = "machine-config")]
    pub(crate) machine_configuration: VmMachineConfiguration,
    #[serde(rename = "cpu-config")]
    pub(crate) cpu_template: Option<VmCpuTemplate>,
    #[serde(rename = "network-interfaces")]
    pub(crate) network_interfaces: Vec<VmNetworkInterface>,
    pub(crate) balloon: Option<VmBalloon>,
    pub(crate) vsock: Option<VmVsock>,
    pub(crate) logger: Option<VmLogger>,
    pub(crate) metrics: Option<VmMetricsSystem>,
    #[serde(rename = "mmds-config")]
    pub(crate) mmds_configuration: Option<VmMmdsConfiguration>,
    pub(crate) entropy: Option<VmEntropy>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum NewVmConfigurationApplier {
    ViaApiCalls,
    ViaJsonConfiguration(PathBuf),
}

impl Default for NewVmConfigurationApplier {
    fn default() -> Self {
        NewVmConfigurationApplier::ViaApiCalls
    }
}

impl NewVmConfiguration {
    pub fn new(boot_source: VmBootSource, machine_configuration: VmMachineConfiguration) -> Self {
        Self {
            applier: NewVmConfigurationApplier::ViaApiCalls,
            boot_source,
            drives: vec![],
            machine_configuration,
            cpu_template: None,
            network_interfaces: vec![],
            balloon: None,
            vsock: None,
            logger: None,
            metrics: None,
            mmds_configuration: None,
            entropy: None,
        }
    }

    pub fn applier(mut self, applier: NewVmConfigurationApplier) -> Self {
        self.applier = applier;
        self
    }

    pub fn drive(mut self, drive: VmDrive) -> Self {
        self.drives.push(drive);
        self
    }

    pub fn cpu_template(mut self, cpu_template: VmCpuTemplate) -> Self {
        self.cpu_template = Some(cpu_template);
        self
    }

    pub fn network_interface(mut self, network_interface: VmNetworkInterface) -> Self {
        self.network_interfaces.push(network_interface);
        self
    }

    pub fn balloon(mut self, balloon: VmBalloon) -> Self {
        self.balloon = Some(balloon);
        self
    }

    pub fn vsock(mut self, vsock: VmVsock) -> Self {
        self.vsock = Some(vsock);
        self
    }

    pub fn logger(mut self, logger: VmLogger) -> Self {
        self.logger = Some(logger);
        self
    }

    pub fn metrics(mut self, metrics: VmMetricsSystem) -> Self {
        self.metrics = Some(metrics);
        self
    }

    pub fn mmds_configuration(mut self, mmds_configuration: VmMmdsConfiguration) -> Self {
        self.mmds_configuration = Some(mmds_configuration);
        self
    }

    pub fn entropy(mut self, entropy: VmEntropy) -> Self {
        self.entropy = Some(entropy);
        self
    }

    pub fn get_applier(&self) -> &NewVmConfigurationApplier {
        &self.applier
    }

    pub fn get_cpu_template(&self) -> Option<&VmCpuTemplate> {
        self.cpu_template.as_ref()
    }

    pub fn get_drives(&self) -> &Vec<VmDrive> {
        &self.drives
    }

    pub fn get_network_interfaces(&self) -> &Vec<VmNetworkInterface> {
        &self.network_interfaces
    }

    pub fn get_balloon(&self) -> Option<&VmBalloon> {
        self.balloon.as_ref()
    }

    pub fn get_vsock(&self) -> Option<&VmVsock> {
        self.vsock.as_ref()
    }

    pub fn get_logger(&self) -> Option<&VmLogger> {
        self.logger.as_ref()
    }

    pub fn get_metrics(&self) -> Option<&VmMetricsSystem> {
        self.metrics.as_ref()
    }

    pub fn get_mmds_configuration(&self) -> Option<&VmMmdsConfiguration> {
        self.mmds_configuration.as_ref()
    }

    pub fn get_entropy(&self) -> Option<&VmEntropy> {
        self.entropy.as_ref()
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct FromSnapshotVmConfiguration {
    pub(crate) load_snapshot: VmLoadSnapshot,
    pub(crate) logger: Option<VmLogger>,
    pub(crate) metrics: Option<VmMetricsSystem>,
}

impl FromSnapshotVmConfiguration {
    pub fn new(load_snapshot: VmLoadSnapshot) -> Self {
        Self {
            load_snapshot,
            logger: None,
            metrics: None,
        }
    }

    pub fn logger(mut self, logger: VmLogger) -> Self {
        self.logger = Some(logger);
        self
    }

    pub fn metrics(mut self, metrics: VmMetricsSystem) -> Self {
        self.metrics = Some(metrics);
        self
    }

    pub fn get_load_snapshot(&self) -> &VmLoadSnapshot {
        &self.load_snapshot
    }

    pub fn get_logger(&self) -> &Option<VmLogger> {
        &self.logger
    }

    pub fn get_metrics(&self) -> &Option<VmMetricsSystem> {
        &self.metrics
    }
}
