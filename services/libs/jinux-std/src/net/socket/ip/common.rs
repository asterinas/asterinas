use crate::net::iface::BindPortConfig;
use crate::net::iface::Iface;
use crate::net::iface::{AnyBoundSocket, AnyUnboundSocket};
use crate::net::iface::{IpAddress, IpEndpoint};
use crate::net::IFACES;
use crate::prelude::*;

pub fn get_iface_to_bind(ip_addr: &IpAddress) -> Option<Arc<dyn Iface>> {
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
fn get_ephemeral_iface(remote_ip_addr: &IpAddress) -> Arc<dyn Iface> {
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

pub(super) fn bind_socket(
    unbound_socket: Box<AnyUnboundSocket>,
    endpoint: IpEndpoint,
    can_reuse: bool,
) -> core::result::Result<Arc<AnyBoundSocket>, (Error, Box<AnyUnboundSocket>)> {
    let iface = match get_iface_to_bind(&endpoint.addr) {
        Some(iface) => iface,
        None => {
            let err = Error::with_message(Errno::EADDRNOTAVAIL, "Request iface is not available");
            return Err((err, unbound_socket));
        }
    };
    let bind_port_config = match BindPortConfig::new(endpoint.port, can_reuse) {
        Ok(config) => config,
        Err(e) => return Err((e, unbound_socket)),
    };
    iface.bind_socket(unbound_socket, bind_port_config)
}

pub fn get_ephemeral_endpoint(remote_endpoint: &IpEndpoint) -> IpEndpoint {
    let iface = get_ephemeral_iface(&remote_endpoint.addr);
    let ip_addr = iface.ipv4_addr().unwrap();
    IpEndpoint::new(IpAddress::Ipv4(ip_addr), 0)
}
