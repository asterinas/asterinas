// SPDX-License-Identifier: MPL-2.0

use aster_bigtcp::{
    errors::BindError,
    iface::BindPortConfig,
    wire::{IpAddress, IpEndpoint, Ipv4Address, Ipv6Address},
};

use crate::{
    net::{
        iface::Iface,
        route::{self, RouteLookupKey, RouteType},
        socket::util::check_port_privilege,
    },
    prelude::*,
};

pub(super) fn get_ephemeral_endpoint(
    remote_endpoint: &IpEndpoint,
    can_broadcast: bool,
) -> Result<IpEndpoint> {
    if !can_broadcast
        && matches!(remote_endpoint.addr, IpAddress::Ipv4(addr) if addr.is_broadcast())
    {
        return_errno_with_message!(
            Errno::EACCES,
            "sending to a broadcast address without SO_BROADCAST is not allowed"
        );
    }

    // Linux treats an unspecified remote address as the loopback address of
    // the same family when selecting a route. IPv4-mapped IPv6 destinations
    // are routed through the IPv4 connect path.
    // References:
    // - <https://elixir.bootlin.com/linux/v7.0/source/net/ipv4/route.c#L2803-L2811>
    // - <https://elixir.bootlin.com/linux/v7.0/source/net/ipv6/tcp_ipv6.c#L171-L181>
    // - <https://elixir.bootlin.com/linux/v7.0/source/net/ipv6/tcp_ipv6.c#L215-L240>
    let route_dst = match remote_endpoint.addr {
        IpAddress::Ipv4(addr) if addr.is_unspecified() => Ipv4Address::LOCALHOST.into(),
        IpAddress::Ipv6(addr) if addr.is_unspecified() => Ipv6Address::LOCALHOST.into(),
        IpAddress::Ipv6(addr)
            if addr
                .to_ipv4_mapped()
                .is_some_and(|addr| addr.is_unspecified()) =>
        {
            Ipv4Address::LOCALHOST.into()
        }
        addr => addr,
    };
    let route_entry = route::lookup(RouteLookupKey::new_dst(route_dst))?;
    if !can_broadcast && route_entry.route().type_() == RouteType::Broadcast {
        return_errno_with_message!(
            Errno::EACCES,
            "sending to a broadcast address without SO_BROADCAST is not allowed"
        );
    }

    let oif_index = route_entry
        .route()
        .oif_index()
        .ok_or_else(|| Error::with_message(Errno::ENODEV, "the route has no output iface"))?;
    let iface = route::iface_by_index(oif_index).ok_or_else(|| {
        Error::with_message(Errno::ENODEV, "the route output iface does not exist")
    })?;
    let (source, error_message) = match remote_endpoint.addr {
        IpAddress::Ipv4(_) => (
            iface
                .ipv4_cidr()
                .map(|cidr| IpAddress::Ipv4(cidr.address())),
            "the route output iface has no IPv4 address",
        ),
        IpAddress::Ipv6(_) => (
            iface
                .ipv6_cidr()
                .map(|cidr| IpAddress::Ipv6(cidr.address())),
            "the route output iface has no IPv6 address",
        ),
    };
    let source = source.ok_or_else(|| Error::with_message(Errno::EADDRNOTAVAIL, error_message))?;
    Ok(IpEndpoint::new(source, 0))
}

pub(super) fn resolve_bind_iface_and_config(
    endpoint: &IpEndpoint,
    can_reuse: bool,
) -> Result<(Arc<Iface>, BindPortConfig)> {
    check_port_privilege(endpoint.port)?;

    let iface = route::lookup_local_iface(&endpoint.addr)?;

    let bind_port_config = BindPortConfig::new(*endpoint, can_reuse);

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
