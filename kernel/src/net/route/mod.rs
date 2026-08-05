// SPDX-License-Identifier: MPL-2.0

//! Routing table support.
//!
//! The top-level route module owns the kernel routing table and exposes the
//! operations used by socket lookup and rtnetlink.

use core::num::NonZeroU32;

use aster_bigtcp::{
    iface::InterfaceType,
    wire::{IpAddress, IpEndpoint, Ipv4Cidr, Ipv6Cidr},
};
use spin::Once;

use self::manager::RouteManager;
use super::iface::{self, Iface};
use crate::prelude::*;

mod entry;
mod manager;
mod rule;
mod table;

pub use entry::{RouteEntry, RouteProtocol, RouteScope, RouteTableId, RouteType};
pub use manager::{RouteLookupKey, RouteTableEntry};

static ROUTE_MANAGER: Once<RouteManager> = Once::new();

/// Initializes routes from the currently configured interfaces.
pub fn init() {
    ROUTE_MANAGER.call_once(|| {
        let routes = iface::iter_all_ifaces()
            .filter_map(|iface| match bootstrap_routes_for_iface(iface) {
                Ok(routes) => Some(routes),
                Err(err) => {
                    warn!(
                        "failed to collect bootstrap routes for iface {}: {:?}",
                        iface.index(),
                        err
                    );
                    None
                }
            })
            .flatten()
            .collect();

        RouteManager::new(routes)
    });
}

fn bootstrap_routes_for_iface(iface: &Arc<Iface>) -> Result<Vec<RouteTableEntry>> {
    let mut routes = Vec::new();
    let oif_index = NonZeroU32::new(iface.index())
        .ok_or_else(|| Error::with_message(Errno::EINVAL, "the interface index is zero"))?;

    if let Some(iface_cidr) = iface.ipv4_cidr() {
        let ipv4_addr = iface_cidr.address();
        routes.push(RouteTableEntry::new(
            RouteTableId::LOCAL,
            RouteEntry::new(
                Ipv4Cidr::new(ipv4_addr, 32).into(),
                RouteProtocol::KERNEL,
                RouteScope::HOST,
                RouteType::Local,
                Some(oif_index),
                None,
            )?,
        ));

        if iface.type_() == InterfaceType::LOOPBACK {
            routes.push(RouteTableEntry::new(
                RouteTableId::LOCAL,
                RouteEntry::new(
                    iface_cidr.network().into(),
                    RouteProtocol::KERNEL,
                    RouteScope::HOST,
                    RouteType::Local,
                    Some(oif_index),
                    None,
                )?,
            ));
        } else {
            routes.push(RouteTableEntry::new(
                RouteTableId::MAIN,
                RouteEntry::new(
                    iface_cidr.network().into(),
                    RouteProtocol::KERNEL,
                    RouteScope::LINK,
                    RouteType::Unicast,
                    Some(oif_index),
                    None,
                )?,
            ));
        }

        if let Some(broadcast_addr) = iface.broadcast_addr() {
            routes.push(RouteTableEntry::new(
                RouteTableId::LOCAL,
                RouteEntry::new(
                    Ipv4Cidr::new(broadcast_addr, 32).into(),
                    RouteProtocol::KERNEL,
                    RouteScope::LINK,
                    RouteType::Broadcast,
                    Some(oif_index),
                    None,
                )?,
            ));
        }
    }

    for (dst, gateway) in iface.routes() {
        routes.push(RouteTableEntry::new(
            RouteTableId::MAIN,
            RouteEntry::new(
                dst,
                RouteProtocol::BOOT,
                RouteScope::UNIVERSE,
                RouteType::Unicast,
                Some(oif_index),
                Some(gateway),
            )?,
        ));
    }

    if let Some(ipv6_cidr) = iface.ipv6_cidr() {
        if iface.type_() != InterfaceType::LOOPBACK {
            routes.push(RouteTableEntry::new(
                RouteTableId::MAIN,
                RouteEntry::new(
                    ipv6_cidr.into(),
                    RouteProtocol::KERNEL,
                    RouteScope::UNIVERSE,
                    RouteType::Unicast,
                    Some(oif_index),
                    None,
                )?,
            ));
        }

        routes.push(RouteTableEntry::new(
            RouteTableId::LOCAL,
            RouteEntry::new(
                Ipv6Cidr::new(ipv6_cidr.address(), 128).into(),
                RouteProtocol::KERNEL,
                RouteScope::UNIVERSE,
                RouteType::Local,
                Some(oif_index),
                None,
            )?,
        ));
    }

    Ok(routes)
}

/// Dumps IP routes.
pub fn dump() -> Vec<RouteTableEntry> {
    ROUTE_MANAGER.get().unwrap().dump()
}

/// Looks up an IP route.
pub fn lookup(key: RouteLookupKey) -> Result<RouteTableEntry> {
    ROUTE_MANAGER.get().unwrap().lookup_entry(&key)
}

/// Looks up the interface that owns a local IP address.
pub fn lookup_local_iface(ip_addr: &IpAddress) -> Result<Arc<Iface>> {
    let manager = ROUTE_MANAGER.get().unwrap();
    let route = manager
        .get_local_table()
        .lookup_with_key(&RouteLookupKey::new_dst(*ip_addr))
        .ok_or_else(|| {
            Error::with_message(
                Errno::EADDRNOTAVAIL,
                "the address is not available from the local machine",
            )
        })?;

    if route.type_() != RouteType::Local {
        return_errno_with_message!(
            Errno::EADDRNOTAVAIL,
            "the address is not available from the local machine"
        );
    }

    let oif_index = route
        .oif_index()
        .ok_or_else(|| Error::with_message(Errno::ENODEV, "the local route has no output iface"))?;
    iface_by_index(oif_index).ok_or_else(|| {
        Error::with_message(Errno::ENODEV, "the local route output iface does not exist")
    })
}

/// Determines if the endpoint is routed to an IPv4 broadcast address.
///
/// Limited broadcast is an address-level special case. Directed broadcasts
/// are represented as `RTN_BROADCAST` entries in the local route table.
pub fn is_broadcast_endpoint(endpoint: &IpEndpoint) -> bool {
    let IpAddress::Ipv4(ipv4_addr) = endpoint.addr else {
        return false;
    };

    if ipv4_addr.is_broadcast() {
        return true;
    }

    ROUTE_MANAGER
        .get()
        .unwrap()
        .lookup_entry(&RouteLookupKey::new_dst(ipv4_addr.into()))
        .is_ok_and(|route| route.route().type_() == RouteType::Broadcast)
}

/// Returns an interface by index.
pub fn iface_by_index(index: NonZeroU32) -> Option<Arc<Iface>> {
    iface::iter_all_ifaces()
        .find(|iface| iface.index() == index.get())
        .map(Clone::clone)
}
