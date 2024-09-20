// SPDX-License-Identifier: MPL-2.0

use aster_bigtcp::{
    errors::BindError,
    iface::BindPortConfig,
    wire::{IpAddress, IpEndpoint},
};

use crate::{
    net::iface::{Iface, IFACES},
    prelude::*,
};

pub(super) fn get_iface_to_bind(ip_addr: &IpAddress) -> Option<Arc<Iface>> {
    let ifaces = IFACES.get().unwrap();
    let IpAddress::Ipv4(ipv4_addr) = ip_addr;
    ifaces
        .iter()
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
    let ifaces = IFACES.get().unwrap();
    let IpAddress::Ipv4(remote_ipv4_addr) = remote_ip_addr;
    if let Some(iface) = ifaces.iter().find(|iface| {
        if let Some(iface_ipv4_addr) = iface.ipv4_addr() {
            iface_ipv4_addr == *remote_ipv4_addr
        } else {
            false
        }
    }) {
        return iface.clone();
    }
    // FIXME: use the virtio-net as the default interface
    ifaces[0].clone()
}

pub(super) fn bind_socket<S, T>(
    unbound_socket: Box<S>,
    endpoint: &IpEndpoint,
    can_reuse: bool,
    bind: impl FnOnce(
        Arc<Iface>,
        Box<S>,
        BindPortConfig,
    ) -> core::result::Result<T, (BindError, Box<S>)>,
) -> core::result::Result<T, (Error, Box<S>)> {
    let iface = match get_iface_to_bind(&endpoint.addr) {
        Some(iface) => iface,
        None => {
            let err = Error::with_message(
                Errno::EADDRNOTAVAIL,
                "the address is not available from the local machine",
            );
            return Err((err, unbound_socket));
        }
    };

    let bind_port_config = BindPortConfig::new(endpoint.port, can_reuse);

    bind(iface, unbound_socket, bind_port_config).map_err(|(err, unbound)| (err.into(), unbound))
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
