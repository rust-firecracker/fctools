use std::net::Ipv4Addr;

use cidr::Ipv4Inet;

/// A link-local IPv4 subnet. Internally this type is incredibly lean, not storing any
/// actual IPv4 addresses but rather only a u16, a u8 and a u32.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LinkLocalSubnet {
    subnet_index: u16,
    network_length: u8,
    ip_amount: u32,
}

const LINK_LOCAL_OCTET_1: u8 = 169;
const LINK_LOCAL_OCTET_2: u8 = 254;
const LINK_LOCAL_IP_AMOUNT: u32 = 65536;

/// An error that can be returned by operations with a LinkLocalSubnet.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LinkLocalSubnetError {
    #[error("The given subnet is not link-local (fits into 169.254.0.0/16)")]
    NotLinkLocal,
    #[error("The given network length is thinner than /30 or wider than /17")]
    NetworkLengthDoesNotFit,
    #[error("The given subnet index does not fit into the link-local range (169.254.0.0/16)")]
    SubnetIndexDoesNotFit,
    #[error("The given IP index does not fit into the subnet")]
    IpIndexDoesNotFit,
    #[error("An unexpected unsigned integer overflow occurred. This should never happen")]
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
    /// Try to create a new link-local subnet with the given network length (mask-short) and "subnet index", i.e. its offset relative
    /// to the beginning of all allocatable link-local subnets with this network length. Sanity checks to the integer values are
    /// always applied.
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

    /// Try to convert an Ipv4Inet into a link-local subnet.
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

    /// Return the amount of "theoretical" IPs in this subnet, which includes 2 IPv4 addresses that can't
    /// be used by Internet hosts.
    pub const fn ip_amount(&self) -> u32 {
        self.ip_amount
    }

    /// Return the amount of IPs in this subnet that can be used by Internet hosts.
    pub const fn host_ip_amount(&self) -> u32 {
        self.ip_amount - 2
    }

    /// Get a "theoretical" IPv4 address within this subnet that is offset by the given IP index.
    #[inline(always)]
    pub fn get_ip(&self, ip_index: u32) -> Result<Ipv4Inet, LinkLocalSubnetError> {
        if ip_index >= self.ip_amount() {
            return Err(LinkLocalSubnetError::IpIndexDoesNotFit);
        }

        self.get_ip_imp(self.ip_amount() * self.subnet_index as u32 + ip_index)
    }

    /// Get a host IPv4 address within this subnet that is offset by the given IP index.
    #[inline(always)]
    pub fn get_host_ip(&self, ip_index: u32) -> Result<Ipv4Inet, LinkLocalSubnetError> {
        if ip_index >= self.host_ip_amount() {
            return Err(LinkLocalSubnetError::IpIndexDoesNotFit);
        }

        self.get_ip_imp(self.ip_amount() * self.subnet_index as u32 + ip_index + 1)
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

    /// Get all "theoretical" IP addresses (sequentially) within this subnet. Unlike other methods on this struct,
    /// this one should not return an error unless there's a problem in the library.
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

    /// Get host "theoretical" IP addresses (sequentially) within this subnet. Unlike other methods on this struct,
    /// this one should not return an error unless there's a problem in the library.
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

    use crate::extension::link_local::LinkLocalSubnetError;

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

    #[test]
    fn ip_amounts_are_reported_correctly() {
        for network_length in 17_u8..=30_u8 {
            let ip_amount = 2_u32.pow(32 - network_length as u32);
            let subnet = LinkLocalSubnet::new(0, network_length).unwrap();
            assert_eq!(subnet.ip_amount(), ip_amount);
            assert_eq!(subnet.host_ip_amount(), ip_amount - 2);
        }
    }

    #[test]
    fn get_ip_reports_correctly() {
        for network_length in 17_u8..=30_u8 {
            let subnet = LinkLocalSubnet::new(0, network_length).unwrap();
            for i in 0..subnet.ip_amount() {
                let ip = subnet.get_ip(i).unwrap();
                assert_eq!(ip.address().octets()[0], 169);
                assert_eq!(ip.address().octets()[1], 254);
                assert_eq!(ip.address().octets()[2], (i / 256) as u8);
                assert_eq!(ip.address().octets()[3], (i % 256) as u8);
            }
        }
    }

    #[test]
    fn get_host_ip_reports_correctly() {
        for network_length in 17_u8..=30_u8 {
            let subnet = LinkLocalSubnet::new(0, network_length).unwrap();
            for i in 0..subnet.host_ip_amount() {
                let ip = subnet.get_host_ip(i).unwrap();
                assert_eq!(ip.address().octets()[0], 169);
                assert_eq!(ip.address().octets()[1], 254);
                assert_eq!(ip.address().octets()[2], ((i + 1) / 256) as u8);
                assert_eq!(ip.address().octets()[3], ((i + 1) % 256) as u8);
            }
        }
    }
}
