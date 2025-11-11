// SPDX-License-Identifier: MPL-2.0

use core::net::Ipv4Addr;

use aster_bigtcp::wire::{IpAddress, IpEndpoint};
use spin::Once;

use crate::{net::iface::iter_all_ifaces, prelude::*};

/// All known broadcast addresses.
// FIXME: This information should be maintained in the routing table,
// since a broadcast address might change if an interface's IP
// or netmask changes, or if an interface is added/removed.
static BROADCAST_ADDRS: Once<BTreeSet<Ipv4Addr>> = Once::new();

pub(super) fn init() {
    BROADCAST_ADDRS.call_once(|| {
        let mut broadcast_addrs = BTreeSet::new();
        // 255.255.255.255 is always included.
        broadcast_addrs.insert(Ipv4Addr::BROADCAST);

        for iface in iter_all_ifaces() {
            let Some(broadcast_addr) = iface.broadcast_addr() else {
                continue;
            };

            broadcast_addrs.insert(broadcast_addr);
        }
        broadcast_addrs
    });
}

/// Determines if a given IP endpoint's address is a known broadcast address.
pub fn is_broadcast_endpoint(endpoint: &IpEndpoint) -> bool {
    let IpAddress::Ipv4(ipv4_addr) = &endpoint.addr;
    BROADCAST_ADDRS.get().unwrap().contains(ipv4_addr)
}
