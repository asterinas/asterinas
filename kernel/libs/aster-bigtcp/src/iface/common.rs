// SPDX-License-Identifier: MPL-2.0

use alloc::{
    collections::btree_map::{BTreeMap, Entry},
    ffi::CString,
    sync::Arc,
    vec::Vec,
};
use core::{
    ffi::CStr,
    ops::Deref,
    sync::atomic::{AtomicBool, AtomicU32, Ordering},
};

use aster_softirq::BottomHalfDisabled;
use bitflags::bitflags;
use int_to_c_enum::TryFromInt;
use ostd::sync::{SpinLock, SpinLockGuard};
use smoltcp::{
    iface::{Context, packet::Packet},
    phy::Device,
    wire::{IpAddress, IpEndpoint, Ipv4Address, Ipv4Packet, Ipv6Address, Ipv6Packet},
};

use super::{
    Iface,
    poll::{FnHelper, PollContext, SocketTableAction},
    poll_iface::PollableIface,
    port::{BindPortConfig, BindPortScope},
    time::get_network_timestamp,
};
use crate::{
    errors::BindError,
    ext::Ext,
    socket::{TcpListenerBg, UdpSocketBg},
    socket_table::SocketTable,
};

pub struct IfaceCommon<E: Ext> {
    index: u32,
    name: CString,
    type_: InterfaceType,
    flags: InterfaceFlags,

    interface: SpinLock<PollableIface<E>, BottomHalfDisabled>,
    used_ports: SpinLock<PortTable, BottomHalfDisabled>,
    sockets: SpinLock<SocketTable<E>, BottomHalfDisabled>,
    sched_poll: E::ScheduleNextPoll,
}

/// An enum representing either an IPv4 or IPv6 packet.
pub(super) enum IpPacket<'a> {
    Ipv4(Ipv4Packet<&'a [u8]>),
    Ipv6(Ipv6Packet<&'a [u8]>),
}

impl<E: Ext> IfaceCommon<E> {
    pub(super) fn new(
        name: CString,
        type_: InterfaceType,
        flags: InterfaceFlags,
        interface: smoltcp::iface::Interface,
        sched_poll: E::ScheduleNextPoll,
    ) -> Self {
        let index = INTERFACE_INDEX_ALLOCATOR.fetch_add(1, Ordering::Relaxed);

        Self {
            index,
            name,
            type_,
            flags,
            interface: SpinLock::new(PollableIface::new(interface)),
            used_ports: SpinLock::new(PortTable::new()),
            sockets: SpinLock::new(SocketTable::new()),
            sched_poll,
        }
    }

    pub(super) fn index(&self) -> u32 {
        self.index
    }

    pub(super) fn name(&self) -> &CStr {
        &self.name
    }

    pub(super) fn type_(&self) -> InterfaceType {
        self.type_
    }

    pub(super) fn flags(&self) -> InterfaceFlags {
        self.flags
    }

    pub(super) fn ipv4_addr(&self) -> Option<Ipv4Address> {
        self.interface.lock().ipv4_addr()
    }

    pub(super) fn ipv6_addr(&self) -> Option<Ipv6Address> {
        self.interface.lock().ipv6_addr()
    }

    pub(super) fn ipv4_prefix_len(&self) -> Option<u8> {
        self.interface.lock().ipv4_prefix_len()
    }

    pub(super) fn ipv6_prefix_len(&self) -> Option<u8> {
        self.interface.lock().ipv6_prefix_len()
    }

    pub(super) fn sched_poll(&self) -> &E::ScheduleNextPoll {
        &self.sched_poll
    }
}

/// An allocator that allocates a unique index for each interface.
//
// FIXME: This allocator is specific to each network namespace.
static INTERFACE_INDEX_ALLOCATOR: AtomicU32 = AtomicU32::new(1);

// Lock order: `interface` -> `sockets`
impl<E: Ext> IfaceCommon<E> {
    /// Acquires the lock to the interface.
    pub(crate) fn interface(&self) -> SpinLockGuard<'_, PollableIface<E>, BottomHalfDisabled> {
        self.interface.lock()
    }

    /// Acquires the lock to the socket table.
    pub(crate) fn sockets(&self) -> SpinLockGuard<'_, SocketTable<E>, BottomHalfDisabled> {
        self.sockets.lock()
    }
}

const IP_LOCAL_PORT_START: u16 = 32768;
const IP_LOCAL_PORT_END: u16 = 60999;

impl<E: Ext> IfaceCommon<E> {
    pub(super) fn bind_tcp(
        &self,
        iface: Arc<dyn Iface<E>>,
        config: BindPortConfig,
    ) -> Result<BoundTcpPort<E>, BindError> {
        self.bind(iface, config, PortProtocol::Tcp)
            .map(BoundTcpPort)
    }

    pub(super) fn bind_udp(
        &self,
        iface: Arc<dyn Iface<E>>,
        config: BindPortConfig,
    ) -> Result<BoundUdpPort<E>, BindError> {
        self.bind(iface, config, PortProtocol::Udp)
            .map(BoundUdpPort)
    }

    fn bind(
        &self,
        iface: Arc<dyn Iface<E>>,
        config: BindPortConfig,
        protocol: PortProtocol,
    ) -> Result<BoundPort<E>, BindError> {
        let scope = config.scope();
        let (port, can_reuse) = self.used_ports.lock().bind(config, protocol)?;
        Ok(BoundPort {
            iface,
            scope,
            port,
            protocol,
            can_reuse: AtomicBool::new(can_reuse),
        })
    }

    /// Releases the port so that it can be used again.
    fn release_port(
        &self,
        scope: BindPortScope,
        port: u16,
        can_reuse: bool,
        protocol: PortProtocol,
    ) {
        self.used_ports
            .lock()
            .release(scope, port, can_reuse, protocol);
    }
}

impl<E: Ext> IfaceCommon<E> {
    pub(crate) fn register_udp_socket(&self, socket: Arc<UdpSocketBg<E>>) {
        let mut sockets = self.sockets.lock();
        sockets.insert_udp_socket(socket);
    }

    pub(crate) fn remove_tcp_listener(&self, socket: &Arc<TcpListenerBg<E>>) {
        let mut sockets = self.sockets.lock();
        let removed = sockets.remove_listener(socket.listener_key());
        debug_assert!(removed.is_some());
    }

    pub(crate) fn remove_udp_socket(&self, socket: &Arc<UdpSocketBg<E>>) {
        let mut sockets = self.sockets.lock();
        let removed = sockets.remove_udp_socket(socket);
        debug_assert!(removed.is_some());
    }
}

impl<E: Ext> IfaceCommon<E> {
    pub(super) fn poll<D, P, Q>(
        &self,
        device: &mut D,
        mut process_phy: P,
        mut dispatch_phy: Q,
    ) -> Option<u64>
    where
        D: Device + ?Sized,
        P: for<'pkt, 'cx, 'tx> FnHelper<
                &'pkt [u8],
                &'cx mut Context,
                D::TxToken<'tx>,
                Option<(IpPacket<'pkt>, D::TxToken<'tx>)>,
            >,
        Q: FnMut(&Packet, &mut Context, D::TxToken<'_>),
    {
        let mut interface = self.interface();
        interface.context_mut().now = get_network_timestamp();

        let mut sockets = self.sockets.lock();
        let mut socket_actions = Vec::new();

        let mut context = PollContext::new(interface.as_mut(), &sockets, &mut socket_actions);
        context.poll_ingress(device, &mut process_phy, &mut dispatch_phy);
        context.poll_egress(device, &mut dispatch_phy);

        // Insert new connections and remove dead connections.
        for action in socket_actions.into_iter() {
            match action {
                SocketTableAction::AddTcpConn(new_tcp_conn) => {
                    let res = sockets.insert_connection(new_tcp_conn);
                    debug_assert!(res.is_ok());
                }
                SocketTableAction::DelTcpConn(dead_conn_key) => {
                    sockets.remove_dead_tcp_connection(&dead_conn_key);
                }
            }
        }

        // Note that only TCP connections can have timers set, so as far as the time to poll is
        // concerned, we only need to consider TCP connections.
        interface.next_poll_at_ms()
    }
}

/// A port bound to an iface.
///
/// When dropped, the port is automatically released.
pub struct BoundPort<E: Ext> {
    iface: Arc<dyn Iface<E>>,
    scope: BindPortScope,
    port: u16,
    protocol: PortProtocol,
    can_reuse: AtomicBool,
}

impl<E: Ext> BoundPort<E> {
    /// Returns a reference to the iface.
    pub fn iface(&self) -> &Arc<dyn Iface<E>> {
        &self.iface
    }

    /// Returns the port number.
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Returns the bound IP address.
    pub fn addr(&self) -> IpAddress {
        self.scope.addr()
    }

    /// Returns the bound endpoint.
    pub fn endpoint(&self) -> IpEndpoint {
        IpEndpoint::new(self.scope.addr(), self.port)
    }

    /// Returns the scope that the port is bound to.
    pub(crate) fn scope(&self) -> BindPortScope {
        self.scope
    }

    /// Sets whether the port can be reused.
    pub fn set_can_reuse(&self, can_reuse: bool) {
        let iface_common = self.iface.common();
        let mut used_ports = iface_common.used_ports.lock();

        // Check after locking `used_ports` to avoid race conditions.
        if self.can_reuse.load(Ordering::Relaxed) == can_reuse {
            return;
        }

        let key = PortKey {
            protocol: self.protocol,
            port: self.port,
            scope: self.scope,
        };
        used_ports.set_can_reuse(key, can_reuse);

        self.can_reuse.store(can_reuse, Ordering::Relaxed);
    }
}

impl<E: Ext> Drop for BoundPort<E> {
    fn drop(&mut self) {
        self.iface.common().release_port(
            self.scope,
            self.port,
            *self.can_reuse.get_mut(),
            self.protocol,
        );
    }
}

/// A TCP port bound to an iface.
pub struct BoundTcpPort<E: Ext>(BoundPort<E>);
/// A UDP port bound to an iface.
pub struct BoundUdpPort<E: Ext>(BoundPort<E>);

impl<E: Ext> Deref for BoundTcpPort<E> {
    type Target = BoundPort<E>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl<E: Ext> Deref for BoundUdpPort<E> {
    type Target = BoundPort<E>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

// The field order matters: keying by `(protocol, port, scope)` keeps all
// bindings on the same port contiguous in `PortTable::used_ports`, so conflict
// checks can scan just that port instead of the whole table.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct PortKey {
    protocol: PortProtocol,
    port: u16,
    scope: BindPortScope,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum PortProtocol {
    Tcp,
    Udp,
}

struct PortState {
    nsocket: usize,
    /// The number of sockets that have enabled address reuse on this port.
    nreuse: usize,
}

impl PortState {
    pub(self) fn new(can_reuse: bool) -> Self {
        let nreuse = if can_reuse { 1 } else { 0 };
        Self { nsocket: 1, nreuse }
    }

    pub(self) fn can_reuse(&self) -> bool {
        self.nsocket == self.nreuse
    }
}

struct PortTable {
    used_ports: BTreeMap<PortKey, PortState>,
    next_ephemeral_port: u16,
}

impl PortTable {
    fn new() -> Self {
        Self {
            used_ports: BTreeMap::new(),
            next_ephemeral_port: IP_LOCAL_PORT_START,
        }
    }

    /// Iterates over the bindings on the given `(protocol, port)`.
    ///
    /// The entries for a single port are contiguous in `used_ports` (see the
    /// note on [`PortKey`]), so this scans only those entries rather than the
    /// whole table.
    fn keys_on_port(
        &self,
        protocol: PortProtocol,
        port: u16,
    ) -> impl Iterator<Item = (&PortKey, &PortState)> {
        let lower_bound = PortKey {
            protocol,
            port,
            scope: BindPortScope::Address(IpAddress::Ipv4(core::net::Ipv4Addr::UNSPECIFIED)),
        };
        self.used_ports
            .range(lower_bound..)
            .take_while(move |(key, _)| key.protocol == protocol && key.port == port)
    }

    fn bind(
        &mut self,
        config: BindPortConfig,
        protocol: PortProtocol,
    ) -> Result<(u16, bool), BindError> {
        let config_can_reuse = config.can_reuse();
        let scope = config.scope();

        let port = if let Some(port) = config.port() {
            port
        } else {
            match self.alloc_ephemeral_port(scope, protocol, config_can_reuse) {
                Some(port) => port,
                None => return Err(BindError::Exhausted),
            }
        };

        let key = PortKey {
            protocol,
            port,
            scope,
        };

        let conflicting_keys = self
            .keys_on_port(protocol, port)
            .filter(|(used_key, _)| scopes_conflict(used_key.scope, scope))
            .map(|(used_key, _)| *used_key)
            .collect::<Vec<_>>();

        if !conflicting_keys.is_empty() {
            // FIXME: If the socket is not a backlog socket,
            // we should check whether there is a listening socket on the port.
            // If there is, the socket cannot be bound to that port.
            let can_reuse = config.is_backlog()
                || (config_can_reuse
                    && conflicting_keys
                        .iter()
                        .all(|key| self.used_ports.get(key).unwrap().can_reuse()));

            if !can_reuse {
                return Err(BindError::InUse);
            }
        };

        let entry = self.used_ports.entry(key);
        match entry {
            Entry::Occupied(mut occupied) => {
                let port_state = occupied.get_mut();
                port_state.nsocket += 1;
                if config_can_reuse {
                    port_state.nreuse += 1;
                }
            }
            Entry::Vacant(vacant) => {
                vacant.insert(PortState::new(config_can_reuse));
            }
        }

        Ok((port, config_can_reuse))
    }

    /// Allocates an ephemeral port.
    ///
    /// We follow the port range that many Linux kernels use by default, which is 32768-60999.
    ///
    /// Each allocation starts scanning from the port immediately
    /// after the last successfully allocated ephemeral port.
    /// This ensures that a recently released port is not immediately
    /// reused by a new socket, which avoids potential port conflicts.
    ///
    /// See <https://en.wikipedia.org/wiki/Ephemeral_port>.
    fn alloc_ephemeral_port(
        &mut self,
        scope: BindPortScope,
        protocol: PortProtocol,
        _can_reuse: bool,
    ) -> Option<u16> {
        const fn next_ephemeral_port_after(port: u16) -> u16 {
            if port >= IP_LOCAL_PORT_END {
                IP_LOCAL_PORT_START
            } else {
                port + 1
            }
        }

        let start_port = self.next_ephemeral_port;
        let mut port = start_port;
        loop {
            let in_use = self
                .keys_on_port(protocol, port)
                .any(|(used_key, _)| scopes_conflict(used_key.scope, scope));
            if !in_use {
                self.next_ephemeral_port = next_ephemeral_port_after(port);
                return Some(port);
            }

            port = next_ephemeral_port_after(port);
            if port == start_port {
                break;
            }
        }

        // FIXME: If `can_reuse` is `true`, we should also check all in-use ephemeral ports
        // to see if any can be reused instead of directly returning `None`.

        None
    }

    fn release(
        &mut self,
        scope: BindPortScope,
        port: u16,
        can_reuse: bool,
        protocol: PortProtocol,
    ) {
        let key = PortKey {
            protocol,
            port,
            scope,
        };
        let Entry::Occupied(mut occupied) = self.used_ports.entry(key) else {
            return;
        };

        let port_state = occupied.get_mut();
        port_state.nsocket -= 1;
        if can_reuse {
            port_state.nreuse -= 1;
        }
        if port_state.nsocket == 0 {
            occupied.remove();
        }
    }

    fn set_can_reuse(&mut self, key: PortKey, can_reuse: bool) {
        let Some(port_state) = self.used_ports.get_mut(&key) else {
            return;
        };

        if can_reuse {
            port_state.nreuse += 1;
        } else {
            port_state.nreuse -= 1;
        }
    }
}

fn scopes_conflict(lhs: BindPortScope, rhs: BindPortScope) -> bool {
    use BindPortScope::*;

    match (lhs, rhs) {
        (Ipv6DualStackWildcard, Address(IpAddress::Ipv4(_)))
        | (Address(IpAddress::Ipv4(_)), Ipv6DualStackWildcard)
        | (Ipv6DualStackWildcard, Ipv4Wildcard)
        | (Ipv4Wildcard, Ipv6DualStackWildcard)
        | (Ipv6DualStackWildcard, Ipv6DualStackWildcard)
        | (Ipv6DualStackWildcard, Ipv6OnlyWildcard)
        | (Ipv6OnlyWildcard, Ipv6DualStackWildcard)
        | (Ipv4Wildcard, Address(IpAddress::Ipv4(_)))
        | (Address(IpAddress::Ipv4(_)), Ipv4Wildcard)
        | (Ipv4Wildcard, Ipv4Wildcard)
        | (Ipv6OnlyWildcard, Address(IpAddress::Ipv6(_)))
        | (Address(IpAddress::Ipv6(_)), Ipv6OnlyWildcard)
        | (Ipv6OnlyWildcard, Ipv6OnlyWildcard) => true,
        (Address(left), Address(right)) => left == right,
        (Ipv6DualStackWildcard, Address(IpAddress::Ipv6(_)))
        | (Address(IpAddress::Ipv6(_)), Ipv6DualStackWildcard) => true,
        _ => false,
    }
}

/// Interface type.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.0.18/source/include/uapi/linux/if_arp.h#L30>
#[repr(u16)]
#[derive(Clone, Copy, Debug, Eq, PartialEq, TryFromInt)]
pub enum InterfaceType {
    // Arp protocol hardware identifiers
    /// from KA9Q: NET/ROM pseudo
    NETROM = 0,
    /// Ethernet 10Mbps
    ETHER = 1,
    /// Experimental Ethernet
    EETHER = 2,

    // Dummy types for non ARP hardware
    /// IPIP tunnel
    TUNNEL = 768,
    /// IP6IP6 tunnel
    TUNNEL6 = 769,
    /// Frame Relay Access Device
    FRAD = 770,
    /// SKIP vif
    SKIP = 771,
    /// Loopback device
    LOOPBACK = 772,
    /// Localtalk device
    LOCALTALK = 773,
    // TODO: This enum is not exhaustive
}

bitflags! {
    /// Interface flags.
    ///
    /// Reference: <https://elixir.bootlin.com/linux/v6.0.18/source/include/uapi/linux/if.h#L82>
    pub struct InterfaceFlags: u32 {
        /// Interface is up
        const UP				= 1<<0;
        /// Broadcast address valid
        const BROADCAST			= 1<<1;
        /// Turn on debugging
        const DEBUG			    = 1<<2;
        /// Loopback net
        const LOOPBACK			= 1<<3;
        /// Interface is has p-p link
        const POINTOPOINT		= 1<<4;
        /// Avoid use of trailers
        const NOTRAILERS		= 1<<5;
        /// Interface RFC2863 OPER_UP
        const RUNNING			= 1<<6;
        /// No ARP protocol
        const NOARP			    = 1<<7;
        /// Receive all packets
        const PROMISC			= 1<<8;
        /// Receive all multicast packets
        const ALLMULTI			= 1<<9;
        /// Master of a load balancer
        const MASTER			= 1<<10;
        /// Slave of a load balancer
        const SLAVE			    = 1<<11;
        /// Supports multicast
        const MULTICAST			= 1<<12;
        /// Can set media type
        const PORTSEL			= 1<<13;
        /// Auto media select active
        const AUTOMEDIA			= 1<<14;
        /// Dialup device with changing addresses
        const DYNAMIC			= 1<<15;
        /// Driver signals L1 up
        const LOWER_UP			= 1<<16;
        /// Driver signals dormant
        const DORMANT			= 1<<17;
        /// Echo sent packets
        const ECHO			    = 1<<18;
    }
}
