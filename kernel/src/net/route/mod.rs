// SPDX-License-Identifier: MPL-2.0

//! Routing table support.
//!
//! The top-level route module owns the kernel routing table and exposes the
//! operations used by socket lookup and rtnetlink.

use aster_bigtcp::{
    iface::InterfaceType,
    wire::{IpAddress, Ipv4Address, Ipv4Cidr, Ipv6Cidr},
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
pub use manager::RouteLookupKey;

static ROUTE_MANAGER: Once<RwMutex<RouteManager>> = Once::new();
const LIMITED_BROADCAST_ADDR: Ipv4Address = Ipv4Address::new(255, 255, 255, 255);

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

        RwMutex::new(RouteManager::new(routes))
    });
}

fn bootstrap_routes_for_iface(iface: &Arc<Iface>) -> Result<Vec<RouteEntry>> {
    let mut routes = Vec::new();

    if let Some(ipv4_addr) = iface.ipv4_addr()
        && let Some(prefix_len) = iface.prefix_len()
    {
        let iface_cidr = Ipv4Cidr::new(ipv4_addr, prefix_len);
        routes.push(RouteEntry::new(
            iface_cidr.network().into(),
            RouteTableId::MAIN,
            RouteProtocol::KERNEL,
            RouteScope::LINK,
            RouteType::Unicast,
            iface.index(),
            None,
        )?);

        let local_dst = if iface.type_() == InterfaceType::LOOPBACK {
            iface_cidr.network()
        } else {
            Ipv4Cidr::new(ipv4_addr, 32)
        };
        routes.push(RouteEntry::new(
            local_dst.into(),
            RouteTableId::LOCAL,
            RouteProtocol::KERNEL,
            RouteScope::HOST,
            RouteType::Local,
            iface.index(),
            None,
        )?);

        routes.push(RouteEntry::new(
            Ipv4Cidr::new(LIMITED_BROADCAST_ADDR, 32).into(),
            RouteTableId::LOCAL,
            RouteProtocol::KERNEL,
            RouteScope::LINK,
            RouteType::Broadcast,
            iface.index(),
            None,
        )?);

        if let Some(broadcast_addr) = iface.broadcast_addr() {
            routes.push(RouteEntry::new(
                Ipv4Cidr::new(broadcast_addr, 32).into(),
                RouteTableId::LOCAL,
                RouteProtocol::KERNEL,
                RouteScope::LINK,
                RouteType::Broadcast,
                iface.index(),
                None,
            )?);
        }

        for (dst, gateway) in iface.ipv4_routes() {
            routes.push(RouteEntry::new(
                dst.into(),
                RouteTableId::MAIN,
                RouteProtocol::BOOT,
                RouteScope::UNIVERSE,
                RouteType::Unicast,
                iface.index(),
                Some(gateway.into()),
            )?);
        }
    }

    if let Some(ipv6_addr) = iface.ipv6_addr()
        && let Some(prefix_len) = iface.ipv6_prefix_len()
    {
        let ipv6_cidr = Ipv6Cidr::new(ipv6_addr, prefix_len);

        routes.push(RouteEntry::new(
            ipv6_cidr.into(),
            RouteTableId::MAIN,
            RouteProtocol::KERNEL,
            RouteScope::LINK,
            RouteType::Unicast,
            iface.index(),
            None,
        )?);

        let local_dst = if iface.type_() == InterfaceType::LOOPBACK {
            ipv6_cidr
        } else {
            Ipv6Cidr::new(ipv6_cidr.address(), 128)
        };
        routes.push(RouteEntry::new(
            local_dst.into(),
            RouteTableId::LOCAL,
            RouteProtocol::KERNEL,
            RouteScope::HOST,
            RouteType::Local,
            iface.index(),
            None,
        )?);
    }

    Ok(routes)
}

/// Dumps IP routes.
pub fn dump(table_filter: Option<RouteTableId>) -> Vec<RouteEntry> {
    ROUTE_MANAGER.get().unwrap().read().dump(table_filter)
}

/// Looks up an IP route.
pub fn lookup(key: RouteLookupKey) -> Result<RouteEntry> {
    ROUTE_MANAGER.get().unwrap().read().lookup_entry(&key)
}

/// Looks up the interface that owns a local IP address.
pub fn lookup_local_iface(ip_addr: &IpAddress) -> Result<Arc<Iface>> {
    let manager = ROUTE_MANAGER.get().unwrap().read();
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

    iface_by_index(route.oif_index()).ok_or_else(|| {
        Error::with_message(Errno::ENODEV, "the local route output iface does not exist")
    })
}

/// Returns an interface by index.
pub fn iface_by_index(index: u32) -> Option<Arc<Iface>> {
    iface::iter_all_ifaces()
        .find(|iface| iface.index() == index)
        .map(Clone::clone)
}
