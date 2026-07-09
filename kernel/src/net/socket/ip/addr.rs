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

impl From<IpEndpoint> for SocketAddr {
    fn from(endpoint: IpEndpoint) -> Self {
        let port = endpoint.port;
        match endpoint.addr {
            IpAddress::Ipv4(addr) => SocketAddr::IPv4(addr, port),
            IpAddress::Ipv6(addr) => SocketAddr::IPv6(addr, port),
        }
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

/// Returns `true` if the address is an IPv4-mapped IPv6 address (`::ffff:x.x.x.x`).
pub(super) fn is_ipv4_mapped(addr: IpAddress) -> bool {
    matches!(addr, IpAddress::Ipv6(v6) if v6.to_ipv4_mapped().is_some())
}

/// Maps a bare IPv4 endpoint to an IPv4-mapped IPv6 [`SocketAddr`].
///
/// Native IPv6 endpoints pass through unchanged.
// Used by `present_to_user` to present dual-stack addresses
// to the user in IPv4-mapped IPv6 form per RFC 4038.
pub(super) fn ipv4_to_ipv4_mapped(endpoint: IpEndpoint) -> SocketAddr {
    if let IpAddress::Ipv4(ipv4) = endpoint.addr {
        let mapped = IpAddress::Ipv6(ipv4.to_ipv6_mapped());
        return SocketAddr::from(IpEndpoint::new(mapped, endpoint.port));
    }
    SocketAddr::from(endpoint)
}

/// Strips the IPv4-mapped prefix (`::ffff:x.x.x.x` → `x.x.x.x`).
///
/// Native IPv4 and native IPv6 pass through unchanged.
/// Must be called before handing an address to smoltcp, which does not
/// understand mapped addresses. Idempotent — safe to call defensively.
pub(crate) fn unmap_ipv4_addr(addr: IpAddress) -> IpAddress {
    match addr {
        IpAddress::Ipv6(addr) => match addr.to_ipv4_mapped() {
            Some(ipv4) => IpAddress::Ipv4(ipv4),
            None => IpAddress::Ipv6(addr),
        },
        other => other,
    }
}

/// Normalizes an endpoint for a socket's address family.
///
/// In dual-stack mode (IPv6 + !v6only), maps bare IPv4 addresses to
/// IPv4-mapped IPv6 so the socket layer can process them uniformly.
pub(super) fn normalize_endpoint(
    family: IpAddressFamily,
    v6only: bool,
    endpoint: IpEndpoint,
) -> IpEndpoint {
    if family == IpAddressFamily::IPv6
        && !v6only
        && let IpAddress::Ipv4(ipv4) = endpoint.addr
    {
        return IpEndpoint::new(IpAddress::Ipv6(ipv4.to_ipv6_mapped()), endpoint.port);
    }
    endpoint
}

/// Normalizes and validates the endpoint for a socket's address family.
/// Returns `Err` if the endpoint is incompatible with the socket.
pub(super) fn validate_endpoint(
    family: IpAddressFamily,
    v6only: bool,
    endpoint: IpEndpoint,
) -> Result<IpEndpoint> {
    let endpoint = normalize_endpoint(family, v6only, endpoint);

    if is_ipv4_mapped(endpoint.addr) && v6only {
        return_errno_with_message!(
            Errno::EAFNOSUPPORT,
            "IPv4-mapped IPv6 addresses are not allowed when IPV6_V6ONLY is set"
        );
    }

    if IpAddressFamily::from(endpoint.addr) != family {
        return_errno_with_message!(
            Errno::EAFNOSUPPORT,
            "the protocol family does not match the address family"
        );
    }

    Ok(endpoint)
}

/// Presents a stored endpoint to the user per RFC 4038.
///
/// For AF_INET6 sockets, bare IPv4 addresses are mapped to IPv4-mapped IPv6 form.
/// The `IPV6_V6ONLY` setting is intentionally **ignored** — per RFC 4038,
/// `getsockname`/`getpeername` always present dual-stack addresses in mapped form
/// regardless of the `IPV6_V6ONLY` socket option.
pub(super) fn present_to_user(family: IpAddressFamily, endpoint: IpEndpoint) -> SocketAddr {
    if family == IpAddressFamily::IPv6 && matches!(endpoint.addr, IpAddress::Ipv4(_)) {
        return ipv4_to_ipv4_mapped(endpoint);
    }
    SocketAddr::from(endpoint)
}

// Note: This does not handle IPv4-mapped IPv6 addresses — it returns `IPv6`
// for them. Callers that need to reject IPv4-mapped addresses when
// `IPV6_V6ONLY` is set must combine this with an explicit `is_ipv4_mapped`
// check (see `validate_endpoint`).
impl From<IpAddress> for IpAddressFamily {
    fn from(addr: IpAddress) -> Self {
        match addr {
            IpAddress::Ipv4(_) => IpAddressFamily::IPv4,
            IpAddress::Ipv6(_) => IpAddressFamily::IPv6,
        }
    }
}
