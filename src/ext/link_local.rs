use std::net::Ipv4Addr;

use cidr::Ipv4Inet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LinkLocalSubnet {
    subnet_index: u16,
    network_length: u8,
    ip_amount: u32,
}

const LINK_LOCAL_OCTET_1: u8 = 169;
const LINK_LOCAL_OCTET_2: u8 = 254;
const LINK_LOCAL_IP_AMOUNT: u32 = 65536;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkLocalSubnetError {
    NotLinkLocal,
    NetworkLengthDoesNotFit,
    SubnetIndexDoesNotFit,
    IpIndexDoesNotFit,
    UnexpectedOverflow,
}

#[inline(always)]
const fn get_ip_amount(network_length: u8) -> u32 {
    2_u32.pow((32 - network_length) as u32)
}

#[inline(always)]
const fn validate_network_length_and_subnet_index(
    network_length: u8,
    subnet_index: u16,
) -> Result<(), LinkLocalSubnetError> {
    if network_length > 30 || network_length < 17 {
        Err(LinkLocalSubnetError::NetworkLengthDoesNotFit)
    } else if LINK_LOCAL_IP_AMOUNT / (2_u32.pow(32 - network_length as u32)) <= subnet_index as u32 {
        Err(LinkLocalSubnetError::SubnetIndexDoesNotFit)
    } else {
        Ok(())
    }
}

impl LinkLocalSubnet {
    pub const fn new(subnet_index: u16, network_length: u8) -> Result<Self, LinkLocalSubnetError> {
        if let Err(err) = validate_network_length_and_subnet_index(network_length, subnet_index) {
            return Err(err);
        }

        Ok(Self {
            subnet_index,
            network_length,
            ip_amount: get_ip_amount(network_length),
        })
    }

    pub const fn from_inet(inet: &Ipv4Inet) -> Result<Self, LinkLocalSubnetError> {
        if !inet.address().is_link_local() {
            return Err(LinkLocalSubnetError::NotLinkLocal);
        }

        // where octet 3 is a, octet 4 is b, network length is c
        // subnet_index=ceil((256a-b%c)/c)
        let network_length = inet.network_length() as u16;
        let octet_3 = inet.address().octets()[2] as u16;
        let octet_4 = inet.address().octets()[3] as u16;
        let subnet_index = (256 * octet_3 - octet_4 % network_length).div_ceil(network_length);

        match validate_network_length_and_subnet_index(inet.network_length(), subnet_index) {
            Ok(_) => Ok(Self {
                subnet_index,
                network_length: inet.network_length(),
                ip_amount: get_ip_amount(inet.network_length()),
            }),
            Err(err) => Err(err),
        }
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

    #[inline(always)]
    pub fn get_ip(&self, ip_index: u32) -> Result<Ipv4Inet, LinkLocalSubnetError> {
        if ip_index >= self.ip_amount() {
            return Err(LinkLocalSubnetError::IpIndexDoesNotFit);
        }

        self.get_ip_imp(self.ip_amount() as u32 * self.subnet_index as u32 + ip_index)
    }

    #[inline(always)]
    pub fn get_host_ip(&self, ip_index: u32) -> Result<Ipv4Inet, LinkLocalSubnetError> {
        if ip_index >= self.host_ip_amount() {
            return Err(LinkLocalSubnetError::IpIndexDoesNotFit);
        }

        self.get_ip_imp(self.ip_amount() as u32 * self.subnet_index as u32 + ip_index + 1)
    }

    #[inline(always)]
    fn get_ip_imp(&self, x: u32) -> Result<Ipv4Inet, LinkLocalSubnetError> {
        let addr = Ipv4Addr::new(
            LINK_LOCAL_OCTET_1,
            LINK_LOCAL_OCTET_2,
            (x / 256)
                .try_into()
                .map_err(|_| LinkLocalSubnetError::UnexpectedOverflow)?,
            (x % 256)
                .try_into()
                .map_err(|_| LinkLocalSubnetError::UnexpectedOverflow)?,
        );

        Ipv4Inet::new(addr, self.network_length).map_err(|_| LinkLocalSubnetError::UnexpectedOverflow)
    }

    #[inline(always)]
    pub fn get_ips(&self) -> Result<Vec<Ipv4Inet>, LinkLocalSubnetError> {
        let ip_amount = self.ip_amount();
        let mut ips = Vec::with_capacity(
            ip_amount
                .try_into()
                .map_err(|_| LinkLocalSubnetError::UnexpectedOverflow)?,
        );

        for i in 0..ip_amount {
            ips.push(self.get_ip(i)?);
        }

        Ok(ips)
    }

    #[inline(always)]
    pub fn get_host_ips(&self) -> Result<Vec<Ipv4Inet>, LinkLocalSubnetError> {
        let host_ip_amount = self.host_ip_amount();
        let mut ips = Vec::with_capacity(
            host_ip_amount
                .try_into()
                .map_err(|_| LinkLocalSubnetError::UnexpectedOverflow)?,
        );

        for i in 0..host_ip_amount {
            ips.push(self.get_host_ip(i)?);
        }

        Ok(ips)
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use cidr::Ipv4Inet;

    use crate::ext::link_local::LinkLocalSubnetError;

    use super::LinkLocalSubnet;

    #[test]
    fn subnet_new_fails_with_wide_network_length() {
        for network_length in 0..=16 {
            assert_eq!(
                LinkLocalSubnet::new(0, network_length),
                Err(LinkLocalSubnetError::NetworkLengthDoesNotFit)
            );
        }
    }

    #[test]
    fn subnet_new_fails_with_thin_network_length() {
        for network_length in 31..=255 {
            assert_eq!(
                LinkLocalSubnet::new(0, network_length),
                Err(LinkLocalSubnetError::NetworkLengthDoesNotFit)
            );
        }
    }

    #[test]
    fn subnet_new_fails_with_not_fitting_subnet_index() {
        for network_length in 17..=30 {
            let min_forbidden_subnet_index = 65536 / (2_u32.pow(32 - network_length as u32));
            assert_eq!(
                LinkLocalSubnet::new(min_forbidden_subnet_index as u16, network_length),
                Err(LinkLocalSubnetError::SubnetIndexDoesNotFit)
            );
        }
    }

    #[test]
    fn subnet_new_succeeds_with_correct_params() {
        for network_length in 17..=30 {
            LinkLocalSubnet::new(0, network_length).unwrap();
        }
    }

    #[test]
    fn subnet_from_inet_fails_with_non_link_local_inet() {
        let inet = Ipv4Inet::from_str("168.253.1.1/30").unwrap();
        assert_eq!(
            LinkLocalSubnet::from_inet(&inet),
            Err(LinkLocalSubnetError::NotLinkLocal)
        );
    }

    #[test]
    fn subnet_from_inet_fails_with_incorrect_network_length() {
        for inet in ["169.254.1.1/31", "169.254.1.1/16"]
            .into_iter()
            .map(|slice| Ipv4Inet::from_str(slice).unwrap())
        {
            assert_eq!(
                LinkLocalSubnet::from_inet(&inet),
                Err(LinkLocalSubnetError::NetworkLengthDoesNotFit)
            );
        }
    }

    #[test]
    fn subnet_from_inet_succeeds_with_correct_params() {
        for a in 1..256 {
            for b in 1..256 {
                let inet = Ipv4Inet::from_str(format!("169.254.{a}.{b}/30").as_str()).unwrap();
                LinkLocalSubnet::from_inet(&inet).unwrap();
            }
        }
    }
}
