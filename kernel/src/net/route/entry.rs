// SPDX-License-Identifier: MPL-2.0

use aster_bigtcp::wire::{IpAddress, IpCidr, Ipv6AddressExt};
use aster_util::ranged_integer::{RangedU8, RangedU32};

use super::RouteLookupKey;
use crate::prelude::*;

/// A route entry stored in the kernel forwarding information base.
///
/// The entry stores only routing state that the current network stack executes:
/// destination matching, table selection, route type, output interface, and
/// optional next-hop gateway. The gateway is part of the route state because two
/// otherwise equivalent routes may forward matching packets through different
/// next hops.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RouteEntry {
    /// Destination network selected by longest-prefix matching.
    dst: IpCidr,
    /// Next-hop gateway, if the route is not directly connected.
    gateway: Option<IpAddress>,
    /// Output interface index. Zero means unspecified.
    oif_index: u32,
    /// Linux route table that owns this entry.
    table: RouteTableId,
    /// Origin of the route.
    protocol: RouteProtocol,
    /// Visibility scope of the route destination.
    scope: RouteScope,
    /// Kernel route type such as unicast, local, or broadcast.
    type_: RouteType,
}

/// A Linux route table identifier.
///
/// Full Linux route table identifiers are `u32` values. The named identifiers
/// below match Linux's reserved table IDs.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.18/source/include/uapi/linux/rtnetlink.h#L354-L364>.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct RouteTableId(RangedU32<0, { u32::MAX }>);

impl RouteTableId {
    /// Identifies an unspecified route table.
    pub const UNSPEC: Self = Self::new(0);
    /// Identifies Linux's default route table.
    pub const DEFAULT: Self = Self::new(253);
    /// Identifies Linux's main route table.
    pub const MAIN: Self = Self::new(254);
    /// Identifies Linux's local route table.
    pub const LOCAL: Self = Self::new(255);

    /// Creates a route table identifier from a raw Linux table ID.
    pub const fn new(id: u32) -> Self {
        Self(RangedU32::new(id))
    }

    /// Returns the raw Linux table ID.
    pub const fn get(self) -> u32 {
        self.0.get()
    }
}

/// Route protocol.
///
/// Linux does not interpret protocol values greater than or equal to
/// `RTPROT_STATIC`, so this type stores the raw value instead of rejecting
/// user-defined routing daemon IDs.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.18/source/include/uapi/linux/rtnetlink.h#L282-L316>.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RouteProtocol(RangedU8<0, { u8::MAX }>);

impl RouteProtocol {
    /// Identifies an unspecified route protocol.
    pub const UNSPEC: Self = Self::new(0);
    /// Identifies routes created by the kernel.
    pub const KERNEL: Self = Self::new(2);
    /// Identifies routes created during boot.
    pub const BOOT: Self = Self::new(3);

    /// Creates a route protocol from a raw Linux protocol value.
    pub const fn new(protocol: u8) -> Self {
        Self(RangedU8::new(protocol))
    }

    /// Returns the raw Linux protocol value.
    pub const fn get(self) -> u8 {
        self.0.get()
    }
}

/// Route scope.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.18/source/include/uapi/linux/rtnetlink.h#L318-L325>.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RouteScope(RangedU8<0, { u8::MAX }>);

impl RouteScope {
    /// Identifies globally reachable route destinations.
    pub const UNIVERSE: Self = Self::new(0);
    /// Identifies link-local route destinations.
    pub const LINK: Self = Self::new(253);
    /// Identifies host-local route destinations.
    pub const HOST: Self = Self::new(254);

    /// Creates a route scope from a raw Linux scope value.
    pub const fn new(scope: u8) -> Self {
        Self(RangedU8::new(scope))
    }

    /// Returns the raw Linux scope value.
    pub const fn get(self) -> u8 {
        self.0.get()
    }
}

/// Route type.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.18/source/include/uapi/linux/rtnetlink.h#L259-L279>.
#[repr(u8)]
#[derive(Clone, Copy, Debug, Eq, PartialEq, TryFromInt)]
pub enum RouteType {
    /// Identifies an unspecified route type.
    Unspec = 0,
    /// Identifies a unicast route.
    Unicast = 1,
    /// Identifies a local address route.
    Local = 2,
    /// Identifies a broadcast address route.
    Broadcast = 3,
    /// Identifies an anycast route.
    Anycast = 4,
    /// Identifies a multicast route.
    Multicast = 5,
    /// Identifies a route that drops packets.
    Blackhole = 6,
    /// Identifies an unreachable destination route.
    Unreachable = 7,
    /// Identifies an administratively prohibited route.
    Prohibit = 8,
    /// Identifies a route that terminates lookup in this table.
    Throw = 9,
    /// Identifies a network-address-translation route.
    Nat = 10,
    /// Identifies a route that uses an external resolver.
    Xresolve = 11,
}

impl RouteEntry {
    /// Creates an executable IP route entry with default selectors.
    pub fn new(
        dst: IpCidr,
        table: RouteTableId,
        protocol: RouteProtocol,
        scope: RouteScope,
        type_: RouteType,
        oif_index: u32,
        gateway: Option<IpAddress>,
    ) -> Result<Self> {
        let network = network_cidr(dst);
        if dst != network {
            return_errno_with_message!(Errno::EINVAL, "the route destination is not canonical");
        }
        if gateway
            .as_ref()
            .is_some_and(|gateway| !same_addr_family(*gateway, dst.address()))
        {
            return_errno_with_message!(
                Errno::EINVAL,
                "the route gateway address family is invalid"
            );
        }
        Ok(Self {
            dst: network,
            gateway,
            oif_index,
            table,
            protocol,
            scope,
            type_,
        })
    }

    /// Returns the destination CIDR.
    pub fn dst(&self) -> IpCidr {
        self.dst
    }

    /// Returns the next-hop gateway.
    pub fn gateway(&self) -> Option<IpAddress> {
        self.gateway
    }

    /// Returns the output interface index.
    pub fn oif_index(&self) -> u32 {
        self.oif_index
    }

    /// Returns the route table ID.
    pub fn table(&self) -> RouteTableId {
        self.table
    }

    /// Returns the route protocol.
    pub fn protocol(&self) -> RouteProtocol {
        self.protocol
    }

    /// Returns the route scope.
    pub fn scope(&self) -> RouteScope {
        self.scope
    }

    /// Returns the route type.
    pub fn type_(&self) -> RouteType {
        self.type_
    }

    pub(super) fn matches_lookup(&self, key: &RouteLookupKey) -> bool {
        matches!(
            self.type_,
            RouteType::Unicast | RouteType::Local | RouteType::Broadcast
        ) && self.dst.contains_addr(&key.dst())
            && key
                .oif_index()
                .is_none_or(|oif_index| self.oif_index == oif_index)
    }
}

fn network_cidr(cidr: IpCidr) -> IpCidr {
    match cidr {
        IpCidr::Ipv4(ipv4_cidr) => ipv4_cidr.network().into(),
        IpCidr::Ipv6(ipv6_cidr) => IpCidr::new(
            IpAddress::Ipv6(ipv6_cidr.address().mask(ipv6_cidr.prefix_len()).into()),
            ipv6_cidr.prefix_len(),
        ),
    }
}

fn same_addr_family(lhs: IpAddress, rhs: IpAddress) -> bool {
    matches!(
        (lhs, rhs),
        (IpAddress::Ipv4(_), IpAddress::Ipv4(_)) | (IpAddress::Ipv6(_), IpAddress::Ipv6(_))
    )
}
