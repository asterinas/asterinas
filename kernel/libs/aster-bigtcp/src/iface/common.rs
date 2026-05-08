// SPDX-License-Identifier: MPL-2.0

use alloc::{
    collections::btree_map::{BTreeMap, Entry},
    string::String,
    sync::Arc,
    vec::Vec,
};
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use aster_softirq::BottomHalfDisabled;
use bitflags::bitflags;
use int_to_c_enum::TryFromInt;
use ostd::sync::{SpinLock, SpinLockGuard};
use smoltcp::{
    iface::{Context, packet::Packet},
    phy::Device,
    wire::{IpAddress, IpEndpoint, Ipv4Address, Ipv4Packet, Ipv6Address, Ipv6Packet},
};

/// An enum representing either an IPv4 or IPv6 packet.
pub(super) enum IpPacket<'a> {
    Ipv4(Ipv4Packet<&'a [u8]>),
    Ipv6(Ipv6Packet<&'a [u8]>),
}

/// Normalizes IPv4-mapped IPv6 addresses to IPv4 addresses.
///
/// IPv4-mapped IPv6 addresses (::ffff:x.x.x.x) should be treated as equivalent
/// to their IPv4 counterparts for binding purposes. This ensures that binding
/// to 192.0.2.1:80 and ::ffff:192.0.2.1:80 are treated as the same.
//
// TODO: This function currently only handles port binding conflict detection.
// Full dual-stack support is not yet implemented, including:
// - Accepting IPv4 connections on IPv6 wildcard socket (binding to `::`)
// - Returning IPv4-mapped addresses in `accept()` for IPv4 clients
// - Proper handling of `IPV6_V6ONLY` socket option
fn normalize_ip_address(addr: IpAddress) -> IpAddress {
    if let IpAddress::Ipv6(ipv6) = addr
        && let Some(ipv4) = ipv6.to_ipv4_mapped()
    {
        return IpAddress::Ipv4(ipv4);
    }

    addr
}

use super::{
    Iface,
    poll::{FnHelper, PollContext, SocketTableAction},
    poll_iface::PollableIface,
    port::BindPortConfig,
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
    name: String,
    type_: InterfaceType,
    flags: InterfaceFlags,

    interface: SpinLock<PollableIface<E>, BottomHalfDisabled>,
    used_ports: SpinLock<BTreeMap<IpAddress, BTreeMap<u16, PortState>>, BottomHalfDisabled>,
    sockets: SpinLock<SocketTable<E>, BottomHalfDisabled>,
    sched_poll: E::ScheduleNextPoll,
}

impl<E: Ext> IfaceCommon<E> {
    pub(super) fn new(
        name: String,
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
            used_ports: SpinLock::new(BTreeMap::new()),
            sockets: SpinLock::new(SocketTable::new()),
            sched_poll,
        }
    }

    pub(super) fn index(&self) -> u32 {
        self.index
    }

    pub(super) fn name(&self) -> &str {
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

    pub(super) fn prefix_len(&self) -> Option<u8> {
        self.interface.lock().prefix_len()
    }

    pub(super) fn sched_poll(&self) -> &E::ScheduleNextPoll {
        &self.sched_poll
    }
}

/// An allocator that allocates a unique index for each interface.
//
// FIXME: This allocator is specific to each network namespace.
pub static INTERFACE_INDEX_ALLOCATOR: AtomicU32 = AtomicU32::new(1);

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
    pub(super) fn bind(
        &self,
        iface: Arc<dyn Iface<E>>,
        config: BindPortConfig,
    ) -> Result<BoundPort<E>, BindError> {
        let addr = config.addr();
        let (port, can_reuse) = self.bind_port(config)?;
        Ok(BoundPort {
            iface,
            addr,
            port,
            can_reuse: AtomicBool::new(can_reuse),
        })
    }

    /// Allocates an ephemeral port.
    ///
    /// We follow the port range that many Linux kernels use by default, which is 32768-60999.
    ///
    /// See <https://en.wikipedia.org/wiki/Ephemeral_port>.
    fn alloc_ephemeral_port(
        used_ports: &mut BTreeMap<IpAddress, BTreeMap<u16, PortState>>,
        addr: IpAddress,
        _can_reuse: bool,
    ) -> Option<u16> {
        let address_ports = used_ports.entry(addr).or_default();
        for port in IP_LOCAL_PORT_START..=IP_LOCAL_PORT_END {
            if let Entry::Vacant(..) = address_ports.entry(port) {
                return Some(port);
            }
        }

        // FIXME: If `can_reuse` is `true`, we should also check all in-use ephemeral ports
        // to see if any can be reused instead of directly returning `None`.

        None
    }

    fn bind_port(&self, config: BindPortConfig) -> Result<(u16, bool), BindError> {
        let mut used_ports = self.used_ports.lock();
        let config_can_reuse = config.can_reuse();
        let addr = normalize_ip_address(config.addr());

        let port = if let Some(port) = config.port() {
            port
        } else {
            match Self::alloc_ephemeral_port(&mut used_ports, addr, config_can_reuse) {
                Some(port) => port,
                None => return Err(BindError::Exhausted),
            }
        };

        let address_ports = used_ports.entry(addr).or_default();
        if let Some(port_state) = address_ports.get_mut(&port) {
            // FIXME: If the socket is not a backlog socket,
            // we should check whether there is a listening socket on the port.
            // If there is, the socket cannot be bound to that port.
            let can_reuse = config.is_backlog() || (port_state.can_reuse() & config_can_reuse);
            if can_reuse {
                port_state.nsocket += 1;
                if config_can_reuse {
                    port_state.nreuse += 1;
                }
            } else {
                return Err(BindError::InUse);
            }
        } else {
            let port_state = PortState::new(config_can_reuse);
            address_ports.insert(port, port_state);
        };

        Ok((port, config_can_reuse))
    }

    /// Releases the port so that it can be used again.
    fn release_port(&self, addr: IpAddress, port: u16, can_reuse: bool) {
        let addr = normalize_ip_address(addr);
        let mut used_ports = self.used_ports.lock();
        if let Some(address_ports) = used_ports.get_mut(&addr)
            && let Some(port_state) = address_ports.get_mut(&port)
        {
            port_state.nsocket -= 1;
            if can_reuse {
                port_state.nreuse -= 1;
            }
            if port_state.nsocket == 0 {
                address_ports.remove(&port);
                if address_ports.is_empty() {
                    used_ports.remove(&addr);
                }
            }
        }
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
//
// FIXME: TCP and UDP ports are independent. Find a way to track the protocol here.
pub struct BoundPort<E: Ext> {
    iface: Arc<dyn Iface<E>>,
    addr: IpAddress,
    port: u16,
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
    pub fn addr(&self) -> &IpAddress {
        &self.addr
    }

    /// Returns the bound endpoint.
    pub fn endpoint(&self) -> IpEndpoint {
        IpEndpoint::new(self.addr, self.port)
    }

    /// Sets whether the port can be reused.
    pub fn set_can_reuse(&self, can_reuse: bool) {
        let iface_common = self.iface.common();
        let mut used_ports = iface_common.used_ports.lock();

        if self.can_reuse.load(Ordering::Relaxed) == can_reuse {
            return;
        }

        let normalized_addr = normalize_ip_address(self.addr);
        if let Some(port_state) = used_ports
            .get_mut(&normalized_addr)
            .and_then(|address_ports| address_ports.get_mut(&self.port))
        {
            if can_reuse {
                port_state.nreuse += 1;
            } else {
                port_state.nreuse -= 1;
            }
        }

        self.can_reuse.store(can_reuse, Ordering::Relaxed);
    }
}

impl<E: Ext> Drop for BoundPort<E> {
    fn drop(&mut self) {
        self.iface
            .common()
            .release_port(self.addr, self.port, *self.can_reuse.get_mut());
    }
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
