// SPDX-License-Identifier: MPL-2.0

use aster_bigtcp::wire::{Ipv4Address, Ipv4Cidr, Ipv6Address, Ipv6AddressExt, Ipv6Cidr};

use super::RouteLookupKey;
use crate::{net::iface::Iface, prelude::*};

/// A route that describes how to handle packets for a destination network.
#[derive(Clone)]
pub(super) struct RouteEntry<A: RouteAddressFamily> {
    dst: A::Cidr,
    #[expect(dead_code)]
    gateway: Option<A>,
    output_iface: Arc<Iface>,
    table_id: RouteTableId,
    metric: RouteMetric,
    type_: RouteType,
}

impl<A: RouteAddressFamily> RouteEntry<A> {
    /// Creates a route entry.
    pub(super) fn new(
        dst: A::Cidr,
        gateway: Option<A>,
        output_iface: Arc<Iface>,
        table_id: RouteTableId,
        metric: RouteMetric,
        type_: RouteType,
    ) -> Result<Self> {
        if dst != A::network(&dst) {
            return_errno_with_message!(Errno::EINVAL, "the route destination is not canonical");
        }

        Ok(Self {
            dst,
            gateway,
            output_iface,
            table_id,
            metric,
            type_,
        })
    }

    /// Returns the destination network.
    pub(super) const fn dst(&self) -> &A::Cidr {
        &self.dst
    }

    /// Returns the next-hop gateway.
    #[expect(dead_code)]
    const fn gateway(&self) -> Option<A> {
        self.gateway
    }

    /// Returns the output interface.
    pub(super) const fn output_iface(&self) -> &Arc<Iface> {
        &self.output_iface
    }

    /// Returns the ID of the table containing the route.
    pub(super) const fn table_id(&self) -> RouteTableId {
        self.table_id
    }

    /// Returns the route metric.
    pub(super) const fn metric(&self) -> RouteMetric {
        self.metric
    }

    /// Returns the route type.
    pub(super) const fn type_(&self) -> RouteType {
        self.type_
    }

    pub(super) fn matches_lookup(&self, key: &RouteLookupKey<A>) -> bool {
        A::contains(&self.dst, key.dst())
    }
}

/// An IP address family supported by the routing manager.
pub(super) trait RouteAddressFamily: Copy + Eq + Ord {
    /// The CIDR type associated with this address family.
    type Cidr: Copy + Eq + Ord;

    /// Returns whether `cidr` contains `address`.
    fn contains(cidr: &Self::Cidr, address: &Self) -> bool;

    /// Returns the canonical network represented by `cidr`.
    fn network(cidr: &Self::Cidr) -> Self::Cidr;

    /// Returns the prefix length of `cidr`.
    fn prefix_len(cidr: &Self::Cidr) -> u8;
}

impl RouteAddressFamily for Ipv4Address {
    type Cidr = Ipv4Cidr;

    fn contains(cidr: &Self::Cidr, address: &Self) -> bool {
        cidr.contains_addr(address)
    }

    fn network(cidr: &Self::Cidr) -> Self::Cidr {
        cidr.network()
    }

    fn prefix_len(cidr: &Self::Cidr) -> u8 {
        cidr.prefix_len()
    }
}

impl RouteAddressFamily for Ipv6Address {
    type Cidr = Ipv6Cidr;

    fn contains(cidr: &Self::Cidr, address: &Self) -> bool {
        cidr.contains_addr(address)
    }

    fn network(cidr: &Self::Cidr) -> Self::Cidr {
        Ipv6Cidr::new(
            cidr.address().mask(cidr.prefix_len()).into(),
            cidr.prefix_len(),
        )
    }

    fn prefix_len(cidr: &Self::Cidr) -> u8 {
        cidr.prefix_len()
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) struct RouteTableId(u32);

impl RouteTableId {
    /// The table ID of the default table,
    /// which is reserved for routes used after earlier rules do not match.
    pub(super) const DEFAULT: Self = Self(253);
    /// The table ID of the main table.
    pub(super) const MAIN: Self = Self(254);
    /// The table ID of the local table,
    /// which manages routes for addresses assigned locally.
    pub(super) const LOCAL: Self = Self(255);
}

/// A route metric used to prefer routes with equal prefix lengths;
/// lower values have higher priority.
#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub(super) struct RouteMetric(u32);

/// The forwarding behavior of a route.
///
/// Reference: <https://elixir.bootlin.com/linux/v7.1/source/include/uapi/linux/rtnetlink.h#L261>.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum RouteType {
    /// Forwards packets to a destination.
    Unicast,
    /// Delivers packets locally.
    Local,
    /// Delivers IPv4 broadcasts.
    Broadcast,
}
