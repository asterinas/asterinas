// SPDX-License-Identifier: MPL-2.0

use aster_bigtcp::{
    errors::BindError,
    iface::BindPortConfig,
    wire::{IpAddress, IpEndpoint},
};

use crate::{
    net::{
        iface::{Iface, iter_all_ifaces, loopback_iface, virtio_iface},
        socket::util::check_port_privilege,
    },
    prelude::*,
};

fn get_iface_to_bind(ip_addr: &IpAddress) -> Option<Arc<Iface>> {
    match *ip_addr {
        IpAddress::Ipv4(ipv4_addr) => iter_all_ifaces()
            .find(|iface| iface.ipv4_addr().is_some_and(|addr| addr == ipv4_addr))
            .map(Clone::clone),
        IpAddress::Ipv6(ipv6_addr) => iter_all_ifaces()
            .find(|iface| iface.ipv6_addr().is_some_and(|addr| addr == ipv6_addr))
            .map(Clone::clone),
    }
}

/// Get a suitable iface to deal with sendto/connect request if the socket is not bound to an iface.
/// If the remote address is the same as that of some iface, we will use the iface.
/// Otherwise, we will use a default interface.
fn get_ephemeral_iface(remote_ip_addr: &IpAddress) -> Arc<Iface> {
    match remote_ip_addr {
        IpAddress::Ipv4(remote_ipv4_addr) => {
            if let Some(iface) = iter_all_ifaces().find(|iface| {
                iface
                    .ipv4_addr()
                    .is_some_and(|addr| addr == *remote_ipv4_addr)
            }) {
                return iface.clone();
            }

            // FIXME: Instead of hardcoding the rules here, we should choose the
            // default interface according to the routing table.
            if let Some(virtio_iface) = virtio_iface() {
                virtio_iface.clone()
            } else {
                loopback_iface().clone()
            }
        }
        IpAddress::Ipv6(remote_ipv6_addr) => {
            if let Some(iface) = iter_all_ifaces().find(|iface| {
                iface
                    .ipv6_addr()
                    .is_some_and(|addr| addr == *remote_ipv6_addr)
            }) {
                return iface.clone();
            }

            // Fall back to an interface with an IPv6 address.
            // Prefer virtio over loopback for external traffic.
            if let Some(virtio_iface) = virtio_iface()
                && virtio_iface.ipv6_addr().is_some()
            {
                return virtio_iface.clone();
            }

            loopback_iface().clone()
        }
    }
}

pub(super) fn resolve_bind_iface_and_config(
    endpoint: &IpEndpoint,
    can_reuse: bool,
) -> Result<(Arc<Iface>, BindPortConfig)> {
    check_port_privilege(endpoint.port)?;

    let iface = match get_iface_to_bind(&endpoint.addr) {
        Some(iface) => iface,
        None => {
            return_errno_with_message!(
                Errno::EADDRNOTAVAIL,
                "the address is not available from the local machine"
            );
        }
    };

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

pub(super) fn get_ephemeral_endpoint(remote_endpoint: &IpEndpoint) -> Option<IpEndpoint> {
    let iface = get_ephemeral_iface(&remote_endpoint.addr);
    match remote_endpoint.addr {
        IpAddress::Ipv4(_) => {
            let ip_addr = iface.ipv4_addr()?;
            Some(IpEndpoint::new(IpAddress::Ipv4(ip_addr), 0))
        }
        IpAddress::Ipv6(_) => {
            let ipv6_addr = iface.ipv6_addr()?;
            Some(IpEndpoint::new(IpAddress::Ipv6(ipv6_addr), 0))
        }
    }
}
