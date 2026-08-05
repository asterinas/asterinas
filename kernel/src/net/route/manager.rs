// SPDX-License-Identifier: MPL-2.0

use core::num::NonZeroU32;

use aster_bigtcp::wire::IpAddress;

use super::{
    entry::{RouteEntry, RouteTableId},
    rule::{RuleAction, RuleList},
    table::RouteTable,
};
use crate::prelude::*;

/// Maintains routing policy rules and route tables.
///
/// `RouteManager` owns the kernel's in-memory routing state. It currently
/// manages the immutable IP FIB bootstrapped from the initial interface
/// configuration.
#[derive(Clone, Debug)]
pub(super) struct RouteManager {
    rules: RuleList,
    tables: BTreeMap<RouteTableId, RouteTable>,
}

/// A route together with the table that contains it.
///
/// The table ID belongs to the route table, not to the stored [`RouteEntry`].
/// This type carries that ownership context across manager boundaries such as
/// route lookup and enumeration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RouteTableEntry {
    table: RouteTableId,
    route: RouteEntry,
}

impl RouteTableEntry {
    pub(super) fn new(table: RouteTableId, route: RouteEntry) -> Self {
        Self { table, route }
    }

    /// Returns the ID of the table containing the route.
    pub fn table(&self) -> RouteTableId {
        self.table
    }

    /// Returns the route stored in the table.
    pub fn route(&self) -> &RouteEntry {
        &self.route
    }

    fn into_parts(self) -> (RouteTableId, RouteEntry) {
        (self.table, self.route)
    }
}

/// A route lookup key.
///
/// The key currently supports destination and optional output-interface
/// selection. The constructor rejects source address, input interface, packet
/// mark, and route protocol selectors because those values require routing
/// policy rule matching that Asterinas does not implement yet.
#[derive(Clone, Copy, Debug)]
pub struct RouteLookupKey {
    dst: IpAddress,
    oif_index: Option<NonZeroU32>,
}

impl RouteLookupKey {
    /// Creates a lookup key for `dst`.
    pub fn new_dst(dst: IpAddress) -> Self {
        Self {
            dst,
            oif_index: None,
        }
    }

    /// Creates a lookup key from all parsed Linux lookup selectors.
    pub(in crate::net) fn new(
        dst: IpAddress,
        oif_index: Option<NonZeroU32>,
        src: Option<IpAddress>,
        iif_index: Option<u32>,
        mark: u32,
        protocol: Option<u8>,
    ) -> Result<Self> {
        // Asterinas parses unsupported policy-routing selectors so requests
        // fail explicitly instead of being silently ignored.
        if src.is_some() || iif_index.is_some() || mark != 0 || protocol.is_some() {
            return_errno_with_message!(Errno::EOPNOTSUPP, "the route lookup key is not supported");
        }

        Ok(Self { dst, oif_index })
    }

    pub(super) fn dst(&self) -> IpAddress {
        self.dst
    }

    pub(in crate::net) fn oif_index(&self) -> Option<NonZeroU32> {
        self.oif_index
    }
}

impl RouteManager {
    pub(super) fn new(bootstrap_routes: Vec<RouteTableEntry>) -> Self {
        let mut tables = BTreeMap::new();
        for table_id in [RouteTableId::LOCAL, RouteTableId::MAIN] {
            tables.insert(table_id, RouteTable::new());
        }

        let mut manager = Self {
            rules: RuleList::default(),
            tables,
        };
        for route in bootstrap_routes {
            let (table_id, route) = route.into_parts();
            let table = manager
                .tables
                .entry(table_id)
                .or_insert_with(RouteTable::new);
            table.insert(route);
        }
        manager
    }

    pub(super) fn dump(&self) -> Vec<RouteTableEntry> {
        self.tables
            .iter()
            .flat_map(|(table_id, table)| {
                table
                    .entries()
                    .iter()
                    .cloned()
                    .map(|route| RouteTableEntry::new(*table_id, route))
            })
            .collect()
    }

    pub(super) fn lookup_entry(&self, key: &RouteLookupKey) -> Result<RouteTableEntry> {
        for rule in self.rules.iter() {
            match rule.action() {
                RuleAction::Lookup => {
                    let Some(table_id) = rule.table() else {
                        continue;
                    };
                    let Some(table) = self.tables.get(&table_id) else {
                        continue;
                    };
                    if let Some(route) = table.lookup_with_key(key) {
                        let route = if key.is_limited_broadcast() {
                            route.into_limited_broadcast()
                        } else {
                            route
                        };
                        return Ok(RouteTableEntry::new(table_id, route));
                    }
                }
                RuleAction::Unreachable | RuleAction::Prohibit | RuleAction::Blackhole => {
                    return_errno_with_message!(Errno::ENETUNREACH, "the route rule rejects lookup");
                }
            }
        }

        return_errno_with_message!(Errno::ENETUNREACH, "no route to the destination")
    }

    pub(super) fn get_local_table(&self) -> &RouteTable {
        self.tables.get(&RouteTableId::LOCAL).unwrap()
    }
}

impl RouteLookupKey {
    fn is_limited_broadcast(&self) -> bool {
        matches!(self.dst, IpAddress::Ipv4(addr) if addr.is_broadcast())
    }
}
