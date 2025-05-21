// SPDX-License-Identifier: MPL-2.0

use alloc::{
    collections::btree_map::{BTreeMap, Entry},
    string::String,
    sync::Arc,
    vec::Vec,
};
use core::sync::atomic::{AtomicU32, Ordering};

use aster_softirq::BottomHalfDisabled;
use bitflags::bitflags;
use int_to_c_enum::TryFromInt;
use ostd::sync::{SpinLock, SpinLockGuard};
use smoltcp::{
    iface::{packet::Packet, Context},
    phy::Device,
    wire::{IpAddress, IpEndpoint, Ipv4Address, Ipv4Packet},
};

use super::{
    poll::{FnHelper, PollContext, SocketTableAction},
    poll_iface::PollableIface,
    port::BindPortConfig,
    time::get_network_timestamp,
    Iface,
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
    used_ports: SpinLock<BTreeMap<u16, usize>, BottomHalfDisabled>,
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
    ) -> core::result::Result<BoundPort<E>, BindError> {
        let port = self.bind_port(config)?;
        Ok(BoundPort { iface, port })
    }

    /// Allocates an unused ephemeral port.
    ///
    /// We follow the port range that many Linux kernels use by default, which is 32768-60999.
    ///
    /// See <https://en.wikipedia.org/wiki/Ephemeral_port>.
    fn alloc_ephemeral_port(&self) -> Option<u16> {
        let mut used_ports = self.used_ports.lock();
        for port in IP_LOCAL_PORT_START..=IP_LOCAL_PORT_END {
            if let Entry::Vacant(e) = used_ports.entry(port) {
                e.insert(0);
                return Some(port);
            }
        }
        None
    }

    fn bind_port(&self, config: BindPortConfig) -> Result<u16, BindError> {
        let port = if let Some(port) = config.port() {
            port
        } else {
            match self.alloc_ephemeral_port() {
                Some(port) => port,
                None => return Err(BindError::Exhausted),
            }
        };

        let mut used_ports = self.used_ports.lock();

        if let Some(used_times) = used_ports.get_mut(&port) {
            if *used_times == 0 || config.can_reuse() {
                // FIXME: Check if the previous socket was bound with SO_REUSEADDR.
                *used_times += 1;
            } else {
                return Err(BindError::InUse);
            }
        } else {
            used_ports.insert(port, 1);
        }

        Ok(port)
    }

    /// Releases the port so that it can be used again (if it is not being reused).
    fn release_port(&self, port: u16) {
        let mut used_ports = self.used_ports.lock();
        if let Some(used_times) = used_ports.remove(&port) {
            if used_times != 1 {
                used_ports.insert(port, used_times - 1);
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
            Option<(Ipv4Packet<&'pkt [u8]>, D::TxToken<'tx>)>,
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
    port: u16,
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

    /// Returns the bound endpoint.
    pub fn endpoint(&self) -> Option<IpEndpoint> {
        let ip_addr = {
            let ipv4_addr = self.iface().ipv4_addr()?;
            IpAddress::Ipv4(ipv4_addr)
        };
        Some(IpEndpoint::new(ip_addr, self.port))
    }
}

impl<E: Ext> Drop for BoundPort<E> {
    fn drop(&mut self) {
        self.iface.common().release_port(self.port);
    }
}

/// Interface type.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.0.18/source/include/uapi/linux/if_arp.h#L30>
#[repr(u16)]
#[derive(Debug, Clone, Copy, TryFromInt, PartialEq, Eq)]
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
