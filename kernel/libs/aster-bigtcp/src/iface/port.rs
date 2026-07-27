// SPDX-License-Identifier: MPL-2.0

use smoltcp::wire::{IpAddress, IpEndpoint};

/// The local address scope that a socket is bound to.
///
/// The scope determines both which incoming packets the socket accepts and how
/// the binding conflicts with other bindings on the same port.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum BindPortScope {
    /// A specific local address.
    Address(IpAddress),
    /// The IPv4 wildcard address (`0.0.0.0`).
    Ipv4Wildcard,
    /// The IPv6 wildcard address (`::`) of an `IPV6_V6ONLY` socket.
    Ipv6OnlyWildcard,
    /// The IPv6 wildcard address (`::`) of a dual-stack socket, which also
    /// accepts IPv4 traffic.
    Ipv6DualStackWildcard,
}

impl BindPortScope {
    /// Returns the representative local address of the scope.
    pub(crate) fn addr(&self) -> IpAddress {
        match self {
            Self::Address(addr) => *addr,
            Self::Ipv4Wildcard => IpAddress::Ipv4(core::net::Ipv4Addr::UNSPECIFIED),
            Self::Ipv6OnlyWildcard | Self::Ipv6DualStackWildcard => {
                IpAddress::Ipv6(core::net::Ipv6Addr::UNSPECIFIED)
            }
        }
    }

    /// Returns whether a packet destined to `addr` should be delivered to a
    /// socket bound with this scope.
    pub(crate) fn matches_addr(&self, addr: IpAddress) -> bool {
        match (*self, addr) {
            (Self::Address(bound), addr) => bound == addr,
            (Self::Ipv4Wildcard, IpAddress::Ipv4(_)) => true,
            (Self::Ipv6OnlyWildcard, IpAddress::Ipv6(addr)) => !is_ipv4_mapped(addr),
            (Self::Ipv6DualStackWildcard, IpAddress::Ipv4(_)) => true,
            (Self::Ipv6DualStackWildcard, IpAddress::Ipv6(addr)) => !is_ipv4_mapped(addr),
            _ => false,
        }
    }
}

fn is_ipv4_mapped(addr: core::net::Ipv6Addr) -> bool {
    let octets = addr.octets();
    octets[..10] == [0; 10] && octets[10] == 0xff && octets[11] == 0xff
}

/// The configuration using for bind to a TCP/UDP port.
pub struct BindPortConfig {
    scope: BindPortScope,
    kind: PortKind,
}

enum PortKind {
    /// Binds to the specified reusable port.
    CanReuse(u16),
    /// Binds to the specified non-reusable port.
    Specified(u16),
    /// Allocates an ephemeral port to bind.
    Ephemeral(bool),
    /// Reuses the port of the listening socket.
    Backlog(u16),
}

impl BindPortConfig {
    /// Creates new configuration using for bind to a TCP/UDP port.
    pub fn new(endpoint: IpEndpoint, can_reuse: bool) -> Self {
        Self::new_with_scope(
            BindPortScope::Address(endpoint.addr),
            endpoint.port,
            can_reuse,
        )
    }

    /// Creates a new configuration that binds to a port within the given scope.
    pub fn new_with_scope(scope: BindPortScope, port: u16, can_reuse: bool) -> Self {
        let kind = match (port, can_reuse) {
            (0, can_reuse) => PortKind::Ephemeral(can_reuse),
            (_, true) => PortKind::CanReuse(port),
            (_, false) => PortKind::Specified(port),
        };
        Self { scope, kind }
    }

    /// Creates a new configuration for reusing the port of a listening socket.
    pub fn new_backlog(endpoint: IpEndpoint) -> Self {
        Self {
            scope: BindPortScope::Address(endpoint.addr),
            kind: PortKind::Backlog(endpoint.port),
        }
    }

    /// Creates a new configuration that reuses a listening socket's port within
    /// the given scope.
    pub(crate) fn new_backlog_with_scope(scope: BindPortScope, port: u16) -> Self {
        Self {
            scope,
            kind: PortKind::Backlog(port),
        }
    }

    pub(super) fn is_backlog(&self) -> bool {
        matches!(self.kind, PortKind::Backlog(..))
    }

    pub(super) fn can_reuse(&self) -> bool {
        matches!(
            self.kind,
            PortKind::CanReuse(..) | PortKind::Ephemeral(true)
        )
    }

    pub(super) fn port(&self) -> Option<u16> {
        match &self.kind {
            PortKind::CanReuse(port) | PortKind::Specified(port) | PortKind::Backlog(port) => {
                Some(*port)
            }
            PortKind::Ephemeral(_) => None,
        }
    }

    pub(super) fn scope(&self) -> BindPortScope {
        self.scope
    }
}
