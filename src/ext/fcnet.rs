use std::{
    net::IpAddr,
    path::{Path, PathBuf},
};

use cidr::IpInet;

use crate::shell_spawner::ShellSpawner;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct FcnetConfiguration {
    iptables_path: Option<PathBuf>,
    iface_name: Option<String>,
    tap_name: Option<String>,
    tap_ip: Option<IpInet>,
    configuration_type: FcnetConfigurationType,
}

impl FcnetConfiguration {
    pub fn simple() -> Self {
        Self {
            iptables_path: None,
            iface_name: None,
            tap_name: None,
            tap_ip: None,
            configuration_type: FcnetConfigurationType::Simple,
        }
    }

    pub fn netns(netns_options: FcnetNetnsOptions) -> Self {
        Self {
            iptables_path: None,
            iface_name: None,
            tap_name: None,
            tap_ip: None,
            configuration_type: FcnetConfigurationType::Netns(netns_options),
        }
    }

    pub fn iptables_path(mut self, iptables_path: impl Into<PathBuf>) -> Self {
        self.iptables_path = Some(iptables_path.into());
        self
    }

    pub fn iface_name(mut self, iface_name: impl Into<String>) -> Self {
        self.iface_name = Some(iface_name.into());
        self
    }

    pub fn tap_name(mut self, tap_name: impl Into<String>) -> Self {
        self.tap_name = Some(tap_name.into());
        self
    }

    pub fn tap_ip(mut self, tap_ip: IpInet) -> Self {
        self.tap_ip = Some(tap_ip);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum FcnetConfigurationType {
    Simple,
    Netns(FcnetNetnsOptions),
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct FcnetNetnsOptions {
    netns_name: Option<String>,
    veth1_name: Option<String>,
    veth2_name: Option<String>,
    veth1_ip: Option<IpInet>,
    veth2_ip: Option<IpInet>,
    guest_ip: Option<IpAddr>,
    forwarded_guest_ip: Option<IpAddr>,
}

impl FcnetNetnsOptions {
    pub fn new() -> Self {
        Self {
            netns_name: None,
            veth1_name: None,
            veth2_name: None,
            veth1_ip: None,
            veth2_ip: None,
            guest_ip: None,
            forwarded_guest_ip: None,
        }
    }

    pub fn netns_name(mut self, netns_name: impl Into<String>) -> Self {
        self.netns_name = Some(netns_name.into());
        self
    }

    pub fn veth1_name(mut self, veth1_name: impl Into<String>) -> Self {
        self.veth1_name = Some(veth1_name.into());
        self
    }

    pub fn veth2_name(mut self, veth2_name: impl Into<String>) -> Self {
        self.veth2_name = Some(veth2_name.into());
        self
    }

    pub fn veth1_ip(mut self, veth1_ip: IpInet) -> Self {
        self.veth1_ip = Some(veth1_ip);
        self
    }

    pub fn veth2_ip(mut self, veth2_ip: IpInet) -> Self {
        self.veth2_ip = Some(veth2_ip);
        self
    }

    pub fn guest_ip(mut self, guest_ip: IpAddr) -> Self {
        self.guest_ip = Some(guest_ip);
        self
    }

    pub fn forwarded_guest_ip(mut self, forwarded_guest_ip: IpAddr) -> Self {
        self.forwarded_guest_ip = Some(forwarded_guest_ip);
        self
    }
}

#[derive(Debug)]
pub enum FcnetError {
    ShellSpawnFailed(tokio::io::Error),
    ProcessWaitFailed(tokio::io::Error),
    FcnetReturnedError(std::process::Output),
}

impl FcnetConfiguration {
    pub async fn add(&self, fcnet_path: impl AsRef<Path>, shell_spawner: &impl ShellSpawner) -> Result<(), FcnetError> {
        self.exec("--add", fcnet_path, shell_spawner).await
    }

    pub async fn check(
        &self,
        fcnet_path: impl AsRef<Path>,
        shell_spawner: &impl ShellSpawner,
    ) -> Result<(), FcnetError> {
        self.exec("--check", fcnet_path, shell_spawner).await
    }

    pub async fn delete(
        &self,
        fcnet_path: impl AsRef<Path>,
        shell_spawner: &impl ShellSpawner,
    ) -> Result<(), FcnetError> {
        self.exec("--del", fcnet_path, shell_spawner).await
    }

    pub fn get_guest_routing_command(&self) -> String {
        format!("ip route add default via {}", self.get_tap_ip_str())
    }

    pub fn get_guest_ip_boot_arg(&self, guest_ip: &IpInet, guest_iface_name: impl AsRef<str>) -> String {
        format!(
            "ip={}::{}:{}::{}:off",
            guest_ip.address().to_string(),
            self.get_tap_ip_str(),
            guest_ip.mask().to_string(),
            guest_iface_name.as_ref()
        )
    }

    fn get_tap_ip_str(&self) -> String {
        match self.tap_ip {
            Some(ref tap_ip) => tap_ip.address().to_string(),
            None => "172.16.0.1".to_string(),
        }
    }

    async fn exec(
        &self,
        operation: &str,
        fcnet_path: impl AsRef<Path>,
        shell_spawner: &impl ShellSpawner,
    ) -> Result<(), FcnetError> {
        let mut arguments = String::from(operation);
        let mut push_arg = |name: &str, value: &str| {
            if value.is_empty() {
                arguments.push(' ');
                arguments.push_str(name);
            } else {
                arguments.push_str(format!(" --{name} {value}").as_str());
            }
        };

        if let Some(ref iptables_path) = self.iptables_path {
            push_arg("iptables-path", iptables_path.to_string_lossy().to_string().as_str());
        }

        if let Some(ref iface_name) = self.iface_name {
            push_arg("iface", iface_name);
        }

        if let Some(ref tap_name) = self.tap_name {
            push_arg("tap", tap_name);
        }

        if let Some(ref tap_ip) = self.tap_ip {
            push_arg("tap-ip", tap_ip.to_string().as_str());
        }

        match self.configuration_type {
            FcnetConfigurationType::Simple => {
                push_arg("simple", "");
            }
            FcnetConfigurationType::Netns(ref netns_options) => {
                push_arg("netns", "");

                if let Some(ref netns_name) = netns_options.netns_name {
                    push_arg("netns", netns_name);
                }

                if let Some(ref veth1_name) = netns_options.veth1_name {
                    push_arg("veth1", veth1_name);
                }

                if let Some(ref veth2_name) = netns_options.veth2_name {
                    push_arg("veth2", veth2_name);
                }

                if let Some(ref veth1_ip) = netns_options.veth1_ip {
                    push_arg("veth1-ip", veth1_ip.to_string().as_str());
                }

                if let Some(ref veth2_ip) = netns_options.veth2_ip {
                    push_arg("veth2-ip", veth2_ip.to_string().as_str());
                }

                if let Some(ref guest_ip) = netns_options.guest_ip {
                    push_arg("guest-ip", guest_ip.to_string().as_str());
                }

                if let Some(ref forwarded_guest_ip) = netns_options.forwarded_guest_ip {
                    push_arg("forwarded-guest-ip", forwarded_guest_ip.to_string().as_str());
                }
            }
        }

        dbg!(&arguments);

        let child = shell_spawner
            .spawn(format!("{} {arguments}", fcnet_path.as_ref().to_string_lossy()))
            .await
            .map_err(FcnetError::ShellSpawnFailed)?;

        let process_output = child.wait_with_output().await.map_err(FcnetError::ProcessWaitFailed)?;
        if !process_output.status.success() {
            return Err(FcnetError::FcnetReturnedError(process_output));
        }

        Ok(())
    }
}
