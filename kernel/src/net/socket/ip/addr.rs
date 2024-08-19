// SPDX-License-Identifier: MPL-2.0

pub use smoltcp::wire::{IpAddress, IpEndpoint, Ipv4Address};

use crate::{net::socket::SocketAddr, prelude::*, return_errno_with_message};

pub type PortNum = u16;

impl TryFrom<SocketAddr> for IpEndpoint {
    type Error = Error;

    fn try_from(value: SocketAddr) -> Result<Self> {
        match value {
            SocketAddr::IPv4(addr, port) => Ok(IpEndpoint::new(addr.into_address(), port)),
            _ => return_errno_with_message!(
                Errno::EAFNOSUPPORT,
                "the address is in an unsupported address family"
            ),
        }
    }
}

impl From<IpEndpoint> for SocketAddr {
    fn from(endpoint: IpEndpoint) -> Self {
        let port = endpoint.port;
        match endpoint.addr {
            IpAddress::Ipv4(addr) => SocketAddr::IPv4(addr, port),
            // TODO: support IPv6
        }
    }
}
