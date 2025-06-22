// SPDX-License-Identifier: MPL-2.0

use aster_bigtcp::{
    errors::BindError,
    iface::BindPortConfig,
    wire::{IpAddress, IpEndpoint},
};

use crate::{
    net::iface::{iter_all_ifaces, loopback_iface, virtio_iface, BoundPort, Iface},
    prelude::*,
};

pub(super) fn get_iface_to_bind(ip_addr: &IpAddress) -> Option<Arc<Iface>> {
    let IpAddress::Ipv4(ipv4_addr) = ip_addr;
    iter_all_ifaces()
        .find(|iface| {
            if let Some(iface_ipv4_addr) = iface.ipv4_addr() {
                iface_ipv4_addr == *ipv4_addr
            } else {
                false
            }
        })
        .map(Clone::clone)
}

/// Get a suitable iface to deal with sendto/connect request if the socket is not bound to an iface.
/// If the remote address is the same as that of some iface, we will use the iface.
/// Otherwise, we will use a default interface.
fn get_ephemeral_iface(remote_ip_addr: &IpAddress) -> Arc<Iface> {
    let IpAddress::Ipv4(remote_ipv4_addr) = remote_ip_addr;
    if let Some(iface) = iter_all_ifaces().find(|iface| {
        if let Some(iface_ipv4_addr) = iface.ipv4_addr() {
            iface_ipv4_addr == *remote_ipv4_addr
        } else {
            false
        }
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

pub(super) fn bind_port(endpoint: &IpEndpoint, can_reuse: bool) -> Result<BoundPort> {
    let iface = match get_iface_to_bind(&endpoint.addr) {
        Some(iface) => iface,
        None => {
            return_errno_with_message!(
                Errno::EADDRNOTAVAIL,
                "the address is not available from the local machine"
            );
        }
    };

    let bind_port_config = BindPortConfig::new(endpoint.port, can_reuse);

    Ok(iface.bind(bind_port_config)?)
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

pub(super) fn get_ephemeral_endpoint(remote_endpoint: &IpEndpoint) -> IpEndpoint {
    let iface = get_ephemeral_iface(&remote_endpoint.addr);
    let ip_addr = iface.ipv4_addr().unwrap();
    IpEndpoint::new(IpAddress::Ipv4(ip_addr), 0)
}
