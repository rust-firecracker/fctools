use std::net::Ipv4Addr;

use cidr::{errors::NetworkLengthTooLongError, Ipv4Inet};

pub struct LinkLocalSubnet {
    offset: u16,
    network_length: u8,
}

const LINK_LOCAL_OCTET_1: u8 = 169;
const LINK_LOCAL_OCTET_2: u8 = 254;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubnetCalculationError {
    NotLinkLocal,
    Overflow,
    NetworkLengthTooLong(NetworkLengthTooLongError),
}

impl LinkLocalSubnet {
    pub const fn new(offset: u16, network_length: u8) -> Self {
        Self { offset, network_length }
    }

    pub fn of_inet(inet: impl AsRef<Ipv4Inet>) -> Result<Self, SubnetCalculationError> {
        let inet = inet.as_ref();
        if !inet.address().is_link_local() {
            return Err(SubnetCalculationError::NotLinkLocal);
        }

        // where octet 3 is A, octet 4 is B, network length is N
        // offset=ceil((256A-B%N)/N)
        let network_length: u16 = inet.network_length().into();
        let octet_3: u16 = inet.address().octets()[2].into();
        let octet_4: u16 = inet.address().octets()[3].into();
        let offset = (256 * octet_3 - octet_4 % network_length).div_ceil(network_length);

        Ok(Self {
            offset,
            network_length: inet.network_length(),
        })
    }

    pub const fn offset(&self) -> u16 {
        self.offset
    }

    pub const fn network_length(&self) -> u8 {
        self.network_length
    }

    pub fn get_member(&self, member: u32) -> Result<Ipv4Inet, SubnetCalculationError> {
        let x = self.network_length as u32 * self.offset as u32 + member;
        let addr = Ipv4Addr::new(
            LINK_LOCAL_OCTET_1,
            LINK_LOCAL_OCTET_2,
            (x / 256).try_into().map_err(|_| SubnetCalculationError::Overflow)?,
            (x % 256).try_into().map_err(|_| SubnetCalculationError::Overflow)?,
        );

        Ipv4Inet::new(addr, self.network_length).map_err(SubnetCalculationError::NetworkLengthTooLong)
    }

    pub fn get_members(&self) -> Result<Vec<Ipv4Inet>, SubnetCalculationError> {
        let ip_amount = 2_u32.pow((32 - self.network_length).into());
        let mut members = Vec::with_capacity(ip_amount.try_into().map_err(|_| SubnetCalculationError::Overflow)?);

        for i in 1..=ip_amount {
            members.push(self.get_member(i)?);
        }

        Ok(members)
    }
}

#[cfg(test)]
mod tests {
    use super::LinkLocalSubnet;

    #[test]
    fn t() {
        let subnet = LinkLocalSubnet::new(10, 24);
        dbg!(subnet.get_members().unwrap());
    }
}
