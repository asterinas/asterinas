// SPDX-License-Identifier: MPL-2.0

use aster_bigtcp::wire::{IpAddress, IpEndpoint, Ipv4Address, Ipv6Address};

use crate::{net::socket::util::SocketAddr, prelude::*};

impl TryFrom<SocketAddr> for IpEndpoint {
    type Error = Error;

    fn try_from(value: SocketAddr) -> Result<Self> {
        match value {
            SocketAddr::IPv4(addr, port) => Ok(IpEndpoint::new(addr.into(), port)),
            SocketAddr::IPv6(addr, port) => Ok(IpEndpoint::new(addr.into(), port)),
            _ => return_errno_with_message!(
                Errno::EAFNOSUPPORT,
                "the address is in an unsupported address family"
            ),
        }
    }
}

/// The operation that an address is being converted for.
///
/// An `IPV6_V6ONLY` socket rejects IPv4-mapped IPv6 addresses, but the error
/// differs by operation: `bind(2)` reports `EINVAL` while `connect(2)` and
/// `sendmsg(2)` report `ENETUNREACH`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum SocketAddrOp {
    Bind,
    Connect,
}

pub(super) fn socket_addr_to_endpoint(
    socket_addr: SocketAddr,
    family: IpAddressFamily,
    is_ipv6_only: bool,
    op: SocketAddrOp,
) -> Result<IpEndpoint> {
    match socket_addr {
        SocketAddr::IPv4(addr, port) if family == IpAddressFamily::IPv4 => {
            Ok(IpEndpoint::new(addr.into(), port))
        }
        SocketAddr::IPv6(addr, port) if family == IpAddressFamily::IPv6 => {
            if let Some(ipv4_addr) = ipv4_mapped_ipv6_to_ipv4(addr) {
                if is_ipv6_only {
                    match op {
                        SocketAddrOp::Bind => return_errno_with_message!(
                            Errno::EINVAL,
                            "an IPv4-mapped IPv6 address cannot be bound on an IPV6_V6ONLY socket"
                        ),
                        SocketAddrOp::Connect => return_errno_with_message!(
                            Errno::ENETUNREACH,
                            "an IPv4-mapped IPv6 address is unreachable from an IPV6_V6ONLY socket"
                        ),
                    }
                }

                Ok(IpEndpoint::new(ipv4_addr.into(), port))
            } else {
                Ok(IpEndpoint::new(addr.into(), port))
            }
        }
        SocketAddr::IPv4(..) | SocketAddr::IPv6(..) => return_errno_with_message!(
            Errno::EAFNOSUPPORT,
            "the protocol family does not match the address family"
        ),
        _ => return_errno_with_message!(
            Errno::EAFNOSUPPORT,
            "the address is in an unsupported address family"
        ),
    }
}

impl From<IpEndpoint> for SocketAddr {
    fn from(endpoint: IpEndpoint) -> Self {
        let port = endpoint.port;
        match endpoint.addr {
            IpAddress::Ipv4(addr) => SocketAddr::IPv4(addr, port),
            IpAddress::Ipv6(addr) => SocketAddr::IPv6(addr, port),
        }
    }
}

pub(super) fn endpoint_to_socket_addr(endpoint: IpEndpoint, family: IpAddressFamily) -> SocketAddr {
    let port = endpoint.port;
    match (endpoint.addr, family) {
        (IpAddress::Ipv4(addr), IpAddressFamily::IPv6) => {
            SocketAddr::IPv6(addr.to_ipv6_mapped(), port)
        }
        (IpAddress::Ipv4(addr), IpAddressFamily::IPv4) => SocketAddr::IPv4(addr, port),
        (IpAddress::Ipv6(addr), _) => SocketAddr::IPv6(addr, port),
    }
}

fn ipv4_mapped_ipv6_to_ipv4(addr: Ipv6Address) -> Option<Ipv4Address> {
    let octets = addr.octets();
    if octets[..10] == [0; 10] && octets[10] == 0xff && octets[11] == 0xff {
        Some(Ipv4Address::from([
            octets[12], octets[13], octets[14], octets[15],
        ]))
    } else {
        None
    }
}

/// An IPv4 local endpoint, which indicates that the local endpoint is unspecified.
///
/// According to the Linux man pages and the Linux implementation, `getsockname()` will _not_ fail
/// even if the socket is unbound. Instead, it will return an unspecified socket address. This
/// unspecified endpoint helps with that.
pub(super) const UNSPECIFIED_LOCAL_ENDPOINT: IpEndpoint =
    IpEndpoint::new(IpAddress::Ipv4(Ipv4Address::UNSPECIFIED), 0);

/// An IPv6 local endpoint, which indicates that the local endpoint is unspecified.
pub(super) const UNSPECIFIED_LOCAL_ENDPOINT_V6: IpEndpoint =
    IpEndpoint::new(IpAddress::Ipv6(Ipv6Address::UNSPECIFIED), 0);

/// Address family for IP sockets.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IpAddressFamily {
    IPv4,
    IPv6,
}

impl IpAddressFamily {
    /// Returns the unspecified endpoint for this address family.
    pub(super) const fn unspecified_endpoint(&self) -> IpEndpoint {
        match self {
            IpAddressFamily::IPv4 => UNSPECIFIED_LOCAL_ENDPOINT,
            IpAddressFamily::IPv6 => UNSPECIFIED_LOCAL_ENDPOINT_V6,
        }
    }
}

// Note: This does not handle IPv4-mapped IPv6 addresses. When `IPV6_V6ONLY` is set,
// IPv4-mapped addresses are not permitted, so this function cannot be used to determine
// whether such an address is acceptable.
impl From<IpAddress> for IpAddressFamily {
    fn from(addr: IpAddress) -> Self {
        match addr {
            IpAddress::Ipv4(_) => IpAddressFamily::IPv4,
            IpAddress::Ipv6(_) => IpAddressFamily::IPv6,
        }
    }
}
