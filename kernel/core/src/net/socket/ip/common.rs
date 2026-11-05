// SPDX-License-Identifier: MPL-2.0

use aster_bigtcp::{
    errors::BindError,
    iface::BindPortConfig,
    wire::{IpAddress, IpEndpoint, Ipv4Address, Ipv6Address},
};

use crate::{
    net::{
        iface::{Iface, iter_all_ifaces},
        route,
        socket::util::check_port_privilege,
    },
    prelude::*,
};

pub(super) fn resolve_bind_iface_and_config(
    endpoint: &IpEndpoint,
    can_reuse: bool,
) -> Result<(Arc<Iface>, BindPortConfig)> {
    check_port_privilege(endpoint.port)?;

    // FIXME: An unspecified bind address (0.0.0.0 / ::) should accept traffic on
    // every interface. aster-bigtcp attaches a bound socket to exactly one
    // interface and dispatches by (addr, port), so instead bind to the default
    // (last registered, i.e. virtio) interface *and* rewrite the address to that
    // interface's own address. Without this, bind() to 0.0.0.0 fails with
    // EADDRNOTAVAIL (e.g. every plain gen_tcp:listen and OTP's inet_res DNS
    // client), or, with the address left unspecified, replies are never
    // delivered and the kernel heap is exhausted.
    let (iface, endpoint) = if endpoint.addr.is_unspecified() {
        let iface = iter_all_ifaces()
            .last()
            .cloned()
            .ok_or_else(|| Error::with_message(Errno::EADDRNOTAVAIL, "no interface"))?;
        let addr = match endpoint.addr {
            IpAddress::Ipv4(_) => iface.ipv4_cidr().map(|c| IpAddress::Ipv4(c.address())),
            IpAddress::Ipv6(_) => iface.ipv6_cidr().map(|c| IpAddress::Ipv6(c.address())),
        }
        .ok_or_else(|| Error::with_message(Errno::EADDRNOTAVAIL, "interface has no address"))?;
        (iface, IpEndpoint::new(addr, endpoint.port))
    } else {
        (route::lookup_local_iface(endpoint.addr)?, *endpoint)
    };

    let bind_port_config = BindPortConfig::new(endpoint, can_reuse);

    Ok((iface, bind_port_config))
}

impl From<BindError> for Error {
    fn from(value: BindError) -> Self {
        match value {
            BindError::Exhausted => {
                Error::with_message(Errno::EAGAIN, "no ephemeral port is available")
            }
            BindError::InUse => {
                Error::with_message(Errno::EADDRINUSE, "the address is already in use")
            }
        }
    }
}

pub(super) fn get_ephemeral_endpoint(remote_endpoint: &IpEndpoint) -> Result<IpEndpoint> {
    let route_dst = remote_endpoint.addr;
    let iface = route::lookup_iface(route_dst)?;

    let source = match route_dst {
        IpAddress::Ipv4(_) => iface
            .ipv4_cidr()
            .map(|cidr| IpAddress::Ipv4(cidr.address()))
            .ok_or_else(|| {
                Error::with_message(
                    Errno::EADDRNOTAVAIL,
                    "the route output iface has no IPv4 address",
                )
            })?,
        IpAddress::Ipv6(_) => iface
            .ipv6_cidr()
            .map(|cidr| IpAddress::Ipv6(cidr.address()))
            .ok_or_else(|| {
                Error::with_message(
                    Errno::EADDRNOTAVAIL,
                    "the route output iface has no IPv6 address",
                )
            })?,
    };

    Ok(IpEndpoint::new(source, 0))
}

/// Maps an unspecified `connect` destination to the loopback address.
///
/// Linux treats `connect` to an unspecified address as connecting to localhost.
///
/// Reference: <https://elixir.bootlin.com/linux/v7.0/source/net/ipv4/route.c#L2803-L2812>.
/// Reference: <https://elixir.bootlin.com/linux/v7.0/source/net/ipv6/tcp_ipv6.c#L171-L181>.
pub(super) fn map_unspecified_to_localhost(mut endpoint: IpEndpoint) -> IpEndpoint {
    endpoint.addr = match endpoint.addr {
        IpAddress::Ipv4(address) if address.is_unspecified() => Ipv4Address::LOCALHOST.into(),
        IpAddress::Ipv6(address) if address.is_unspecified() => Ipv6Address::LOCALHOST.into(),
        address => address,
    };
    endpoint
}
