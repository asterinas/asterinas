// SPDX-License-Identifier: MPL-2.0

use aster_bigtcp::{
    errors::BindError,
    iface::BindPortConfig,
    wire::{IpAddress, IpEndpoint, Ipv4Address},
};

use crate::{
    net::{
        iface::{self, Iface},
        route::{self, RouteLookupKey, RouteType},
        socket::util::check_port_privilege,
    },
    prelude::*,
};

fn get_ephemeral_ipv4_endpoint(
    remote_ipv4_addr: Ipv4Address,
    can_broadcast: bool,
) -> Result<IpEndpoint> {
    let route_entry = route::lookup(RouteLookupKey::new_dst(remote_ipv4_addr.into()))?;
    if !can_broadcast && route_entry.type_() == RouteType::Broadcast {
        return_errno_with_message!(
            Errno::EACCES,
            "sending to a broadcast address without SO_BROADCAST is not allowed"
        );
    }

    let iface = route::iface_by_index(route_entry.oif_index()).ok_or_else(|| {
        Error::with_message(Errno::ENODEV, "the route output iface does not exist")
    })?;
    let source = iface.ipv4_addr().ok_or_else(|| {
        Error::with_message(
            Errno::EADDRNOTAVAIL,
            "the route output iface has no IPv4 address",
        )
    })?;
    Ok(IpEndpoint::new(IpAddress::Ipv4(source), 0))
}

fn get_ephemeral_ipv6_iface(remote_ipv6_addr: &aster_bigtcp::wire::Ipv6Address) -> Arc<Iface> {
    if let Some(iface) = iface::iter_all_ifaces().find(|iface| {
        iface
            .ipv6_addr()
            .is_some_and(|addr| addr == *remote_ipv6_addr)
    }) {
        return iface.clone();
    }

    if let Some(virtio_iface) = iface::virtio_iface()
        && virtio_iface.ipv6_addr().is_some()
    {
        return virtio_iface.clone();
    }

    iface::loopback_iface().clone()
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

pub(super) fn get_ephemeral_endpoint(
    remote_endpoint: &IpEndpoint,
    can_broadcast: bool,
) -> Result<IpEndpoint> {
    match remote_endpoint.addr {
        IpAddress::Ipv4(remote_ipv4_addr) => {
            get_ephemeral_ipv4_endpoint(remote_ipv4_addr, can_broadcast)
        }
        IpAddress::Ipv6(remote_ipv6_addr) => {
            let iface = get_ephemeral_ipv6_iface(&remote_ipv6_addr);
            let ipv6_addr = iface.ipv6_addr().ok_or_else(|| {
                Error::with_message(
                    Errno::EADDRNOTAVAIL,
                    "no interface has an address for the specified family",
                )
            })?;
            Ok(IpEndpoint::new(IpAddress::Ipv6(ipv6_addr), 0))
        }
    }
}
