use std::{process::ExitStatus, sync::Arc};

use cidr::IpInet;

use crate::{shell::ShellSpawner, vm::models::VmNetworkInterface};

pub struct NatNetwork<S: ShellSpawner> {
    pub tap_name: String,
    pub host_ip: IpInet,
    pub host_iface: String,
    pub guest_ip: IpInet,
    pub shell_spawner: Arc<S>,
}

#[derive(Debug)]
pub enum NatNetworkError {
    IoError(tokio::io::Error),
    NonzeroStatus(ExitStatus),
}

impl<S: ShellSpawner> NatNetwork<S> {
    pub async fn create(&self) -> Result<(), NatNetworkError> {
        self.exec(format!("ip tuntap add {} mode tap", self.tap_name))
            .await?;
        self.exec(format!(
            "ip addr add {} dev {}",
            self.host_ip, self.tap_name
        ))
        .await?;
        self.exec(format!("ip link set {} up", self.tap_name))
            .await?;
        self.exec(format!("echo 1 > /proc/sys/net/ipv4/ip_forward"))
            .await?;
        self.exec(format!(
            "iptables -t nat -A POSTROUTING -o {} -j MASQUERADE",
            self.host_iface
        ))
        .await?;
        self.exec(
            "iptables -A FORWARD -m conntrack --ctstate RELATED,ESTABLISHED -j ACCEPT".to_string(),
        )
        .await?;
        self.exec(format!(
            "iptables -A FORWARD -i {} -o {} -j ACCEPT",
            self.tap_name, self.host_iface
        ))
        .await
    }

    pub async fn delete(&self) -> Result<(), NatNetworkError> {
        self.exec(format!("ip link del {}", self.tap_name)).await?;

        self.exec(format!(
            "iptables -t nat -D POSTROUTING -o {} -j MASQUERADE",
            self.host_iface
        ))
        .await?;
        self.exec(
            "iptables -D FORWARD -m conntrack --ctstate RELATED,ESTABLISHED -j ACCEPT".to_string(),
        )
        .await?;
        self.exec(format!(
            "iptables -D FORWARD -i {} -o {} -j ACCEPT",
            self.tap_name, self.host_iface
        ))
        .await
    }

    pub fn get_vm_network_interface(&self) -> VmNetworkInterface {
        VmNetworkInterface::new(self.host_iface.clone(), self.tap_name.clone())
    }

    pub fn append_ip_boot_arg(&self, boot_args: &mut String) {
        boot_args.push_str(
            format!(
                " ip={}::{}:255.255.255.0::eth0:off",
                self.guest_ip.address(),
                self.host_ip.address()
            )
            .as_str(),
        );
    }

    async fn exec(&self, command: String) -> Result<(), NatNetworkError> {
        let mut child = self
            .shell_spawner
            .spawn(command)
            .await
            .map_err(NatNetworkError::IoError)?;
        let exit_status = child.wait().await.map_err(NatNetworkError::IoError)?;
        if !exit_status.success() {
            return Err(NatNetworkError::NonzeroStatus(exit_status));
        }

        Ok(())
    }
}
