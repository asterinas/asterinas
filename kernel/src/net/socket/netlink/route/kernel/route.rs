// SPDX-License-Identifier: MPL-2.0

//! Handle route-related requests.

use crate::{
    net::{
        iface::iter_all_ifaces,
        socket::netlink::{
            message::ErrorSegment,
            route::message::{RouteAttr, RouteSegment, RtnlSegment},
        },
    },
    prelude::*,
    util::net::CSocketAddrFamily,
};

/// Handles RTM_NEWROUTE: adds or replaces a route entry.
///
/// For now we only handle the common case of setting a default IPv4 gateway
/// (`ip route add default via <gw> dev <iface>`).
pub(super) fn do_new_route(request_segment: &RouteSegment) -> Result<Vec<RtnlSegment>> {
    use aster_bigtcp::wire::Ipv4Address;

    let body = request_segment.body();

    if body.family != CSocketAddrFamily::AF_INET as u8 {
        return_errno_with_message!(Errno::EAFNOSUPPORT, "only AF_INET routes are supported");
    }

    let mut gateway: Option<[u8; 4]> = None;
    let mut oif: Option<u32> = None;

    for attr in request_segment.attrs() {
        match attr {
            RouteAttr::Gateway(gw) => gateway = Some(*gw),
            RouteAttr::Oif(idx)    => oif = Some(*idx),
            RouteAttr::Dst(_)      => {}
        }
    }

    let gw_octets = gateway.ok_or_else(|| {
        Error::with_message(Errno::EINVAL, "no gateway in RTM_NEWROUTE")
    })?;
    let gateway_addr = Ipv4Address::new(
        gw_octets[0], gw_octets[1], gw_octets[2], gw_octets[3],
    );

    // Find the target interface: prefer RTA_OIF match, otherwise use the first
    // non-loopback interface.
    let iface = if let Some(idx) = oif {
        iter_all_ifaces()
            .find(|i| i.index() == idx)
            .ok_or_else(|| Error::with_message(Errno::ENODEV, "interface not found"))?
    } else {
        iter_all_ifaces()
            .find(|i| i.ipv4_addr().is_some())
            .ok_or_else(|| Error::with_message(Errno::ENODEV, "no suitable interface found"))?
    };

    iface.set_ipv4_gateway(gateway_addr);

    let ack = ErrorSegment::new_from_request(request_segment.header(), None);
    Ok(vec![RtnlSegment::Error(ack)])
}
