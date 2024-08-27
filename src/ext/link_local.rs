use std::net::Ipv4Addr;

use cidr::Ipv4Inet;

pub struct LinkLocalSubnet {
    subnet_index: u16,
    network_length: u8,
    ip_amount: u32,
}

const LINK_LOCAL_OCTET_1: u8 = 169;
const LINK_LOCAL_OCTET_2: u8 = 254;
const THINNEST_NETWORK_LENGTH: u8 = 30;
const WIDEST_NETWORK_LENGTH: u8 = 17;
const LINK_LOCAL_IP_AMOUNT: u32 = 65536;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkLocalSubnetError {
    NotLinkLocal,
    NetworkLengthDoesNotFit,
    SubnetIndexDoesNotFit,
    IpIndexDoesNotFit,
    Overflow,
    Other,
}

const fn get_ip_amount(network_length: u8) -> u32 {
    2_u32.pow((32 - network_length) as u32)
}

impl LinkLocalSubnet {
    pub const fn new(subnet_index: u16, network_length: u8) -> Result<Self, LinkLocalSubnetError> {
        if network_length > THINNEST_NETWORK_LENGTH || network_length < WIDEST_NETWORK_LENGTH {
            return Err(LinkLocalSubnetError::NetworkLengthDoesNotFit);
        }

        if LINK_LOCAL_IP_AMOUNT / (2_u32.pow(32 - network_length as u32)) <= subnet_index as u32 {
            return Err(LinkLocalSubnetError::SubnetIndexDoesNotFit);
        }

        Ok(Self {
            subnet_index,
            network_length,
            ip_amount: get_ip_amount(network_length),
        })
    }

    pub fn of_inet(inet: impl AsRef<Ipv4Inet>) -> Result<Self, LinkLocalSubnetError> {
        let inet = inet.as_ref();
        if !inet.address().is_link_local() {
            return Err(LinkLocalSubnetError::NotLinkLocal);
        }

        if inet.network_length() > THINNEST_NETWORK_LENGTH || inet.network_length() < WIDEST_NETWORK_LENGTH {
            return Err(LinkLocalSubnetError::NetworkLengthDoesNotFit);
        }

        // where octet 3 is A, octet 4 is B, network length is N
        // offset=ceil((256A-B%N)/N)
        let network_length: u16 = inet.network_length().into();
        let octet_3: u16 = inet.address().octets()[2].into();
        let octet_4: u16 = inet.address().octets()[3].into();
        let index = (256 * octet_3 - octet_4 % network_length).div_ceil(network_length);

        if LINK_LOCAL_IP_AMOUNT / (2_u32.pow(32 - inet.network_length() as u32)) <= index as u32 {
            return Err(LinkLocalSubnetError::SubnetIndexDoesNotFit);
        }

        Ok(Self {
            subnet_index: index,
            network_length: inet.network_length(),
            ip_amount: get_ip_amount(inet.network_length()),
        })
    }

    pub const fn subnet_index(&self) -> u16 {
        self.subnet_index
    }

    pub const fn network_length(&self) -> u8 {
        self.network_length
    }

    pub const fn ip_amount(&self) -> u32 {
        self.ip_amount
    }

    pub const fn host_ip_amount(&self) -> u32 {
        self.ip_amount - 2
    }

    pub fn get_ip(&self, ip_index: u32) -> Result<Ipv4Inet, LinkLocalSubnetError> {
        if ip_index >= self.ip_amount() {
            return Err(LinkLocalSubnetError::IpIndexDoesNotFit);
        }

        let x = self.ip_amount() as u32 * self.subnet_index as u32 + ip_index;
        let addr = Ipv4Addr::new(
            LINK_LOCAL_OCTET_1,
            LINK_LOCAL_OCTET_2,
            (x / 256).try_into().map_err(|_| LinkLocalSubnetError::Overflow)?,
            (x % 256).try_into().map_err(|_| LinkLocalSubnetError::Overflow)?,
        );

        Ipv4Inet::new(addr, self.network_length).map_err(|_| LinkLocalSubnetError::Other)
    }

    pub fn get_ips(&self) -> Result<Vec<Ipv4Inet>, LinkLocalSubnetError> {
        let ip_amount = self.ip_amount();
        let mut ips = Vec::with_capacity(ip_amount.try_into().map_err(|_| LinkLocalSubnetError::Overflow)?);

        for i in 0..ip_amount {
            ips.push(self.get_ip(i)?);
        }

        Ok(ips)
    }
}

#[cfg(test)]
mod tests {
    use super::LinkLocalSubnet;

    #[test]
    fn t() {
        let subnet = LinkLocalSubnet::new(1, 31).unwrap();
        dbg!(subnet.get_ip(3).unwrap());
    }
}
