// SPDX-License-Identifier: MPL-2.0

use alloc::{collections::btree_set::BTreeSet, sync::Arc, vec::Vec};
use core::{
    borrow::Borrow,
    sync::atomic::{AtomicU64, Ordering},
};

use smoltcp::{
    iface::Route,
    socket::{
        PollAt,
        dhcpv4::{Event as DhcpEvent, Socket as Dhcpv4Socket},
    },
    wire::{Ipv4Address, Ipv4Repr, UdpRepr},
};

use crate::{
    ext::Ext,
    socket::{NeedIfacePoll, TcpConnectionBg},
};

/// An interface with auxiliary data that makes it pollable.
///
/// This is used, for example, when updating a socket's next poll time and finding a socket to
/// poll.
pub(crate) struct PollableIface<E: Ext> {
    interface: smoltcp::iface::Interface,
    pending_conns: PendingConnSet<E>,
    /// The DHCPv4 client, present when the interface is configured by DHCP.
    dhcp: Option<Dhcpv4Socket<'static>>,
    /// DNS servers announced by the last DHCP lease.
    dns_servers: Vec<Ipv4Address>,
}

/// Bumped every time an interface's address or routes change at runtime.
///
/// Consumers that cache routing information (the kernel's route tables) compare
/// this with the value they last saw and rebuild when it differs.
pub static CONFIG_GENERATION: AtomicU64 = AtomicU64::new(1);

impl<E: Ext> PollableIface<E> {
    pub(super) fn new(interface: smoltcp::iface::Interface) -> Self {
        Self {
            interface,
            pending_conns: PendingConnSet::new(),
            dhcp: None,
            dns_servers: Vec::new(),
        }
    }

    pub(super) fn new_with_dhcp(interface: smoltcp::iface::Interface) -> Self {
        let mut this = Self::new(interface);
        this.dhcp = Some(Dhcpv4Socket::new());
        this
    }

    /// Returns the DNS servers announced by DHCP (empty for static configuration).
    pub(super) fn dns_servers(&self) -> &[Ipv4Address] {
        &self.dns_servers
    }

    /// Whether DHCP is active and no lease has been obtained yet.
    pub(super) fn is_dhcp_pending(&self) -> bool {
        self.dhcp.is_some() && self.ipv4_cidr().is_none()
    }

    /// Stops the DHCP client. Called when user space configures the address itself.
    pub(super) fn stop_dhcp(&mut self) {
        self.dhcp = None;
        self.dns_servers.clear();
    }

    /// Applies lease events reported by the DHCP client. Returns whether the
    /// configuration changed.
    pub(super) fn poll_dhcp(&mut self) -> bool {
        let Some(dhcp) = self.dhcp.as_mut() else {
            return false;
        };
        let Some(event) = dhcp.poll() else {
            return false;
        };
        match event {
            DhcpEvent::Configured(config) => {
                let address = config.address;
                let router = config.router;
                let dns: Vec<Ipv4Address> = config.dns_servers.iter().copied().collect();
                log::info!("dhcp: lease {} gateway {:?} dns {:?}", address, router, dns);
                self.set_ipv4_cidr(address);
                self.set_ipv4_gateway(router);
                self.dns_servers = dns;
            }
            DhcpEvent::Deconfigured => {
                if self.ipv4_cidr().is_none() {
                    // smoltcp reports `Deconfigured` once at start-up; nothing to undo.
                    return false;
                }
                log::info!("dhcp: lease lost");
                self.interface.update_ip_addrs(|addrs| {
                    addrs.retain(|a| !matches!(a, smoltcp::wire::IpCidr::Ipv4(_)));
                });
                self.set_ipv4_gateway(None);
                self.dns_servers.clear();
            }
        }
        CONFIG_GENERATION.fetch_add(1, Ordering::Release);
        true
    }

    pub(super) fn as_mut(&mut self) -> PollableIfaceMut<'_, E> {
        PollableIfaceMut {
            context: self.interface.context(),
            pending_conns: &mut self.pending_conns,
            dhcp: self.dhcp.as_mut(),
        }
    }

    pub(super) fn ipv4_cidr(&self) -> Option<smoltcp::wire::Ipv4Cidr> {
        self.interface.ip_addrs().iter().find_map(|cidr| {
            if let smoltcp::wire::IpCidr::Ipv4(ipv4_cidr) = cidr {
                Some(*ipv4_cidr)
            } else {
                None
            }
        })
    }

    pub(super) fn ipv6_cidr(&self) -> Option<smoltcp::wire::Ipv6Cidr> {
        self.interface.ip_addrs().iter().find_map(|cidr| {
            if let smoltcp::wire::IpCidr::Ipv6(ipv6_cidr) = cidr {
                Some(*ipv6_cidr)
            } else {
                None
            }
        })
    }

    /// Replaces the IPv4 address of the interface (adding one if there was none).
    pub(super) fn set_ipv4_cidr(&mut self, cidr: smoltcp::wire::Ipv4Cidr) {
        self.interface.update_ip_addrs(|addrs| {
            addrs.retain(|a| !matches!(a, smoltcp::wire::IpCidr::Ipv4(_)));
            addrs.push(smoltcp::wire::IpCidr::Ipv4(cidr)).unwrap();
        });
    }

    /// Replaces the default IPv4 route.
    pub(super) fn set_ipv4_gateway(&mut self, gateway: Option<Ipv4Address>) {
        let routes = self.interface.routes_mut();
        match gateway {
            Some(gw) => {
                routes.add_default_ipv4_route(gw).unwrap();
            }
            None => {
                routes.remove_default_ipv4_route();
            }
        }
    }

    pub(super) fn routes(&mut self) -> Vec<Route> {
        let mut routes = Vec::new();
        self.interface.routes_mut().update(|route_entries| {
            routes.extend(route_entries.iter().copied());
        });
        routes
    }

    /// Returns the next poll time.
    pub(super) fn next_poll_at_ms(&mut self) -> Option<u64> {
        let conns = self.pending_conns.next_poll_at_ms();
        let Some(dhcp) = self.dhcp.as_ref() else {
            return conns;
        };
        let dhcp_at = match dhcp.poll_at(self.interface.context()) {
            PollAt::Now => Some(0),
            PollAt::Time(t) => Some(t.total_millis() as u64),
            PollAt::Ingress => None,
        };
        match (conns, dhcp_at) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }
}

impl<E: Ext> PollableIface<E> {
    /// Returns the `smoltcp` context for passing to the `smoltcp` APIs.
    pub(crate) fn context_mut(&mut self) -> &mut smoltcp::iface::Context {
        self.interface.context()
    }

    /// Updates the next poll time of `socket` to `poll_at`.
    ///
    /// This method (or [`PollableIfaceMut::update_next_poll_at_ms`]) should be called after network or
    /// user events that change the poll time occur.
    pub(crate) fn update_next_poll_at_ms(
        &mut self,
        socket: &Arc<TcpConnectionBg<E>>,
        poll_at: PollAt,
    ) -> NeedIfacePoll {
        self.pending_conns.update_next_poll_at_ms(socket, poll_at)
    }

    /// Maps an address to the local unicast address if it is a broadcast address.
    ///
    /// For example, if the interface is configured with the address `10.0.2.15/24`, this method
    ///  - will return `10.0.2.15` for `10.0.2.255`, and
    ///  - will return the original address for `10.0.2.15` and `10.0.2.16`.
    ///
    /// Note: "local" means that the IP address belongs to the local interface, not to be confused
    /// with the localhost IP (`127.0.0.1`).
    pub(crate) fn map_broadcast_to_local(
        &self,
        addr: smoltcp::wire::IpAddress,
    ) -> smoltcp::wire::IpAddress {
        use smoltcp::wire::IpAddress;

        if let IpAddress::Ipv4(addr_v4) = addr
            && let Some(cidr_v4) = self.ipv4_cidr()
            && cidr_v4.broadcast() == Some(addr_v4)
        {
            return IpAddress::Ipv4(cidr_v4.address());
        }

        addr
    }
}

/// A mutable reference to a [`PollableIface`].
///
/// This type is reconstructed from mutable references to fields in [`PollableIface`], since the fields
/// must be broken into individual fields during interface polling due to limitations of the
/// [`smoltcp`] APIs.
pub(crate) struct PollableIfaceMut<'a, E: Ext> {
    context: &'a mut smoltcp::iface::Context,
    pending_conns: &'a mut PendingConnSet<E>,
    dhcp: Option<&'a mut Dhcpv4Socket<'static>>,
}

// FIXME: We provide `new()` and `inner_mut()` as `pub(crate)` methods because it's necessary to
// allow the Rust compiler to check the lifetime for separate fields. We should find better ways to
// avoid these `pub(crate)` methods in the future.
impl<'a, E: Ext> PollableIfaceMut<'a, E> {
    pub(crate) fn new(
        context: &'a mut smoltcp::iface::Context,
        pending_conns: &'a mut PendingConnSet<E>,
    ) -> Self {
        Self {
            context,
            pending_conns,
            dhcp: None,
        }
    }

    pub(crate) fn inner_mut(&mut self) -> (&mut smoltcp::iface::Context, &mut PendingConnSet<E>) {
        (self.context, self.pending_conns)
    }

    /// Whether a DHCP client is attached and still waiting for a lease.
    pub(super) fn is_dhcp_pending(&self) -> bool {
        self.dhcp.is_some() && self.context.ipv4_addr().is_none()
    }

    /// Feeds an incoming UDP datagram to the DHCP client. Returns `true` if it
    /// was a DHCP packet (whether or not the client accepted it).
    pub(super) fn process_dhcp(
        &mut self,
        ip_repr: &Ipv4Repr,
        udp_repr: &UdpRepr,
        payload: &[u8],
    ) -> bool {
        let Some(dhcp) = self.dhcp.as_deref_mut() else {
            return false;
        };
        if udp_repr.src_port != 67 || udp_repr.dst_port != 68 {
            return false;
        }
        dhcp.process(self.context, ip_repr, udp_repr, payload);
        true
    }

    /// Lets the DHCP client emit a packet if one is due. The packet is
    /// returned as (IP header, UDP header, DHCP payload bytes).
    pub(super) fn dispatch_dhcp(&mut self) -> Option<(Ipv4Repr, UdpRepr, Vec<u8>)> {
        let dhcp = self.dhcp.as_deref_mut()?;
        let mut out = None;
        let _ = dhcp.dispatch(self.context, |_cx, (ip_repr, udp_repr, dhcp_repr)| {
            let mut buf = alloc::vec![0u8; dhcp_repr.buffer_len()];
            let mut packet = smoltcp::wire::DhcpPacket::new_unchecked(&mut buf[..]);
            dhcp_repr.emit(&mut packet).map_err(|_| ())?;
            out = Some((ip_repr, udp_repr, buf));
            Ok::<(), ()>(())
        });
        out
    }
}

impl<E: Ext> PollableIfaceMut<'_, E> {
    pub(super) fn pop_pending_tcp(&mut self) -> Option<Arc<TcpConnectionBg<E>>> {
        let now = self.context.now.total_millis() as u64;
        self.pending_conns.pop_tcp_before_now(now)
    }
}

impl<E: Ext> PollableIfaceMut<'_, E> {
    /// Returns an immutable reference to the `smoltcp` context.
    pub(crate) fn context(&self) -> &smoltcp::iface::Context {
        self.context
    }

    /// Returns the `smoltcp` context for passing to the `smoltcp` APIs.
    pub(crate) fn context_mut(&mut self) -> &mut smoltcp::iface::Context {
        self.context
    }

    /// Updates the next poll time of `socket` to `poll_at`.
    ///
    /// This method (or [`PollableIface::update_next_poll_at_ms`]) should be called after network
    /// or user events that change the poll time occur.
    pub(crate) fn update_next_poll_at_ms(
        &mut self,
        socket: &Arc<TcpConnectionBg<E>>,
        poll_at: PollAt,
    ) -> NeedIfacePoll {
        self.pending_conns.update_next_poll_at_ms(socket, poll_at)
    }
}

/// A key to sort sockets by their next poll time.
pub(crate) struct PollKey {
    next_poll_at_ms: AtomicU64,
    id: usize,
}

impl PartialEq for PollKey {
    fn eq(&self, other: &Self) -> bool {
        self.next_poll_at_ms.load(Ordering::Relaxed)
            == other.next_poll_at_ms.load(Ordering::Relaxed)
            && self.id == other.id
    }
}
impl Eq for PollKey {}
impl PartialOrd for PollKey {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for PollKey {
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        self.next_poll_at_ms
            .load(Ordering::Relaxed)
            .cmp(&other.next_poll_at_ms.load(Ordering::Relaxed))
            .then_with(|| self.id.cmp(&other.id))
    }
}

impl PollKey {
    /// A value indicating that an immediate poll is required.
    const IMMEDIATE_VAL: u64 = 0;
    /// A value indicating that no poll is required.
    const INACTIVE_VAL: u64 = u64::MAX;

    /// Creates a new [`PollKey`].
    ///
    /// `id` must be a unique identifier for the associated socket, as it will be used to locate
    /// the socket to update its next poll time. This is usually done using the address of the
    /// [`Arc`] socket (see [`Arc::as_ptr`]).
    ///
    /// [`Arc`]: alloc::sync::Arc
    /// [`Arc::as_ptr`]: alloc::sync::Arc::as_ptr
    pub(crate) fn new(id: usize) -> Self {
        Self {
            next_poll_at_ms: AtomicU64::new(Self::INACTIVE_VAL),
            id,
        }
    }

    /// Returns whether the next poll is active.
    ///
    /// The next poll is active if there are packets to send or a timer is set, in which case the
    /// socket will live in the pending queue.
    pub(crate) fn is_active(&self) -> bool {
        self.next_poll_at_ms.load(Ordering::Relaxed) != Self::INACTIVE_VAL
    }
}

/// Sockets to poll in the future, sorted by poll time.
pub(crate) struct PendingConnSet<E: Ext>(BTreeSet<PendingTcpConn<E>>);

/// A TCP socket to poll in the future.
///
/// Note that currently only TCP sockets can set a timer to fire in the future, so a
/// [`PendingConnSet`] contains only [`PendingTcpConn`]s.
struct PendingTcpConn<E: Ext>(Arc<TcpConnectionBg<E>>);

impl<E: Ext> PartialEq for PendingTcpConn<E> {
    fn eq(&self, other: &Self) -> bool {
        self.0.poll_key() == other.0.poll_key()
    }
}
impl<E: Ext> Eq for PendingTcpConn<E> {}
impl<E: Ext> PartialOrd for PendingTcpConn<E> {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl<E: Ext> Ord for PendingTcpConn<E> {
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        self.0.poll_key().cmp(other.0.poll_key())
    }
}

impl<E: Ext> Borrow<PollKey> for PendingTcpConn<E> {
    fn borrow(&self) -> &PollKey {
        self.0.poll_key()
    }
}

impl<E: Ext> PendingConnSet<E> {
    fn new() -> Self {
        Self(BTreeSet::new())
    }

    fn update_next_poll_at_ms(
        &mut self,
        socket: &Arc<TcpConnectionBg<E>>,
        poll_at: PollAt,
    ) -> NeedIfacePoll {
        let key = socket.poll_key();
        let old_poll_at_ms = key.next_poll_at_ms.load(Ordering::Relaxed);

        let new_poll_at_ms = match poll_at {
            PollAt::Now => PollKey::IMMEDIATE_VAL,
            PollAt::Time(instant) => instant.total_millis() as u64,
            PollAt::Ingress => PollKey::INACTIVE_VAL,
        };

        // Fast path: There is nothing to update.
        if old_poll_at_ms == new_poll_at_ms {
            return NeedIfacePoll::FALSE;
        }

        // Remove the socket from the pending queue if it is in the queue.
        let owned_socket = if old_poll_at_ms != PollKey::INACTIVE_VAL {
            self.0.take(key).unwrap()
        } else {
            PendingTcpConn(socket.clone())
        };

        // Update the poll time _after_ it is removed from the queue.
        key.next_poll_at_ms.store(new_poll_at_ms, Ordering::Relaxed);

        // If no new poll is required, do not add the socket to the pending queue.
        if new_poll_at_ms == PollKey::INACTIVE_VAL {
            return NeedIfacePoll::FALSE;
        }

        // Add the socket back to the queue.
        let inserted = self.0.insert(owned_socket);
        debug_assert!(inserted);

        if new_poll_at_ms < old_poll_at_ms {
            NeedIfacePoll::TRUE
        } else {
            NeedIfacePoll::FALSE
        }
    }

    fn pop_tcp_before_now(&mut self, now_at_ms: u64) -> Option<Arc<TcpConnectionBg<E>>> {
        if self.0.first().is_some_and(|first| {
            first.0.poll_key().next_poll_at_ms.load(Ordering::Relaxed) <= now_at_ms
        }) {
            self.0.pop_first().map(|first| {
                // Reset `next_poll_at_ms` since the socket is no longer in the queue.
                first
                    .0
                    .poll_key()
                    .next_poll_at_ms
                    .store(PollKey::INACTIVE_VAL, Ordering::Relaxed);
                first.0
            })
        } else {
            None
        }
    }

    fn next_poll_at_ms(&self) -> Option<u64> {
        self.0
            .first()
            .map(|first| first.0.poll_key().next_poll_at_ms.load(Ordering::Relaxed))
    }
}

/// An extension trait for an interface context.
pub(super) trait IsUnicast {
    /// Returns whether the destination address is a unicast address of an interface.
    ///
    /// For example, if the interface is configured with the address `10.0.2.15/24`, this method
    ///  - will return true for `10.0.2.15` and `10.0.2.254`, and
    ///  - will return false for `10.0.2.255` and `255.255.255.255`.
    ///
    /// Note: This excludes broadcast addresses, link-local broadcast addresses, and multicast
    /// addresses.
    fn is_unicast(&self, dst_addr: smoltcp::wire::IpAddress) -> bool;

    /// Returns whether the destination address is a local unicast address of an interface.
    ///
    /// For example, if the interface is configured with the address `10.0.2.15/24`, this method
    ///  - will return true for `10.0.2.15`, and
    ///  - will return false for `10.0.2.14`, `10.0.2.255`, and `255.255.255.255`.
    ///
    /// Note: "local" means that the IP address belongs to the local interface, not to be confused
    /// with the localhost IP (`127.0.0.1`).
    fn is_unicast_local(&self, dst_addr: smoltcp::wire::IpAddress) -> bool;
}

impl IsUnicast for smoltcp::iface::Context {
    fn is_unicast(&self, dst_addr: smoltcp::wire::IpAddress) -> bool {
        !self.is_broadcast(&dst_addr) && !dst_addr.is_multicast()
    }

    fn is_unicast_local(&self, dst_addr: smoltcp::wire::IpAddress) -> bool {
        use smoltcp::wire::IpAddress;

        match dst_addr {
            IpAddress::Ipv4(dst_addr) => self.ipv4_addr().is_some_and(|addr| {
                // All IPv4 loopback addresses are handled by the same loopback interface.
                // Treating them as local allows direct socket delivery without traversing the
                // device queues.
                addr == dst_addr || (addr.is_loopback() && dst_addr.is_loopback())
            }),
            IpAddress::Ipv6(dst_addr) => self.ipv6_addr().is_some_and(|addr| addr == dst_addr),
        }
    }
}
