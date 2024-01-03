// SPDX-License-Identifier: MPL-2.0

use crate::net::iface::{IpAddress, Ipv4Address};
use crate::net::iface::{IpEndpoint, IpListenEndpoint};
use crate::net::socket::unix::UnixSocketAddr;
use crate::prelude::*;

type PortNum = u16;

#[derive(Debug)]
pub enum SocketAddr {
    Unix(UnixSocketAddr),
    IPv4(Ipv4Address, PortNum),
    IPv6,
}

impl TryFrom<SocketAddr> for IpEndpoint {
    type Error = Error;

    fn try_from(value: SocketAddr) -> Result<Self> {
        match value {
            SocketAddr::IPv4(addr, port) => Ok(IpEndpoint::new(addr.into_address(), port)),
            _ => return_errno_with_message!(
                Errno::EINVAL,
                "sock addr cannot be converted as IpEndpoint"
            ),
        }
    }
}

impl TryFrom<IpEndpoint> for SocketAddr {
    type Error = Error;

    fn try_from(endpoint: IpEndpoint) -> Result<Self> {
        let port = endpoint.port;
        let socket_addr = match endpoint.addr {
            IpAddress::Ipv4(addr) => SocketAddr::IPv4(addr, port), // TODO: support IPv6
        };
        Ok(socket_addr)
    }
}

impl TryFrom<IpListenEndpoint> for SocketAddr {
    type Error = Error;

    fn try_from(value: IpListenEndpoint) -> Result<Self> {
        let port = value.port;
        let socket_addr = match value.addr {
            None => return_errno_with_message!(Errno::EINVAL, "address is unspecified"),
            Some(IpAddress::Ipv4(address)) => SocketAddr::IPv4(address, port),
        };
        Ok(socket_addr)
    }
}
