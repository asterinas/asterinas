// SPDX-License-Identifier: MPL-2.0

use alloc::{
    collections::{
        btree_map::{BTreeMap, Entry},
        btree_set::BTreeSet,
    },
    string::String,
    sync::Arc,
    vec::Vec,
};

use keyable_arc::KeyableArc;
use ostd::sync::{LocalIrqDisabled, SpinLock, SpinLockGuard};
use smoltcp::{
    iface::{packet::Packet, Context},
    phy::Device,
    wire::{IpAddress, IpEndpoint, Ipv4Address, Ipv4Packet},
};

use super::{
    poll::{FnHelper, PollContext},
    port::BindPortConfig,
    time::get_network_timestamp,
    Iface,
};
use crate::{
    errors::BindError,
    ext::Ext,
    socket::{TcpConnectionBg, TcpListenerBg, UdpSocketBg},
};

pub struct IfaceCommon<E: Ext> {
    name: String,
    interface: SpinLock<smoltcp::iface::Interface, LocalIrqDisabled>,
    used_ports: SpinLock<BTreeMap<u16, usize>, LocalIrqDisabled>,
    sockets: SpinLock<SocketSet<E>, LocalIrqDisabled>,
    sched_poll: E::ScheduleNextPoll,
}

pub(super) struct SocketSet<E: Ext> {
    pub(super) tcp_conn: BTreeSet<KeyableArc<TcpConnectionBg<E>>>,
    pub(super) tcp_listen: BTreeSet<KeyableArc<TcpListenerBg<E>>>,
    pub(super) udp: BTreeSet<KeyableArc<UdpSocketBg<E>>>,
}

impl<E: Ext> IfaceCommon<E> {
    pub(super) fn new(
        name: String,
        interface: smoltcp::iface::Interface,
        sched_poll: E::ScheduleNextPoll,
    ) -> Self {
        let sockets = SocketSet {
            tcp_conn: BTreeSet::new(),
            tcp_listen: BTreeSet::new(),
            udp: BTreeSet::new(),
        };

        Self {
            name,
            interface: SpinLock::new(interface),
            used_ports: SpinLock::new(BTreeMap::new()),
            sockets: SpinLock::new(sockets),
            sched_poll,
        }
    }

    pub(super) fn name(&self) -> &str {
        &self.name
    }

    pub(super) fn ipv4_addr(&self) -> Option<Ipv4Address> {
        self.interface.lock().ipv4_addr()
    }

    pub(super) fn sched_poll(&self) -> &E::ScheduleNextPoll {
        &self.sched_poll
    }
}

impl<E: Ext> IfaceCommon<E> {
    /// Acquires the lock to the interface.
    pub(crate) fn interface(&self) -> SpinLockGuard<smoltcp::iface::Interface, LocalIrqDisabled> {
        self.interface.lock()
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
    pub(crate) fn register_tcp_connection(&self, socket: KeyableArc<TcpConnectionBg<E>>) {
        let mut sockets = self.sockets.lock();
        let inserted = sockets.tcp_conn.insert(socket);
        debug_assert!(inserted);
    }

    pub(crate) fn register_tcp_listener(&self, socket: KeyableArc<TcpListenerBg<E>>) {
        let mut sockets = self.sockets.lock();
        let inserted = sockets.tcp_listen.insert(socket);
        debug_assert!(inserted);
    }

    pub(crate) fn register_udp_socket(&self, socket: KeyableArc<UdpSocketBg<E>>) {
        let mut sockets = self.sockets.lock();
        let inserted = sockets.udp.insert(socket);
        debug_assert!(inserted);
    }

    #[allow(clippy::mutable_key_type)]
    fn remove_dead_tcp_connections(sockets: &mut BTreeSet<KeyableArc<TcpConnectionBg<E>>>) {
        for socket in sockets.extract_if(|socket| socket.is_dead()) {
            TcpConnectionBg::on_dead_events(socket);
        }
    }

    pub(crate) fn remove_tcp_listener(&self, socket: &KeyableArc<TcpListenerBg<E>>) {
        let mut sockets = self.sockets.lock();
        let removed = sockets.tcp_listen.remove(socket);
        debug_assert!(removed);
    }

    pub(crate) fn remove_udp_socket(&self, socket: &KeyableArc<UdpSocketBg<E>>) {
        let mut sockets = self.sockets.lock();
        let removed = sockets.udp.remove(socket);
        debug_assert!(removed);
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
        interface.context().now = get_network_timestamp();

        let mut sockets = self.sockets.lock();

        loop {
            let mut new_tcp_conns = Vec::new();

            let mut context = PollContext::new(interface.context(), &sockets, &mut new_tcp_conns);
            context.poll_ingress(device, &mut process_phy, &mut dispatch_phy);
            context.poll_egress(device, &mut dispatch_phy);

            // New packets sent by new connections are not handled. So if there are new
            // connections, try again.
            if new_tcp_conns.is_empty() {
                break;
            } else {
                sockets.tcp_conn.extend(new_tcp_conns);
            }
        }

        Self::remove_dead_tcp_connections(&mut sockets.tcp_conn);

        sockets.tcp_conn.iter().for_each(|socket| {
            if socket.has_events() {
                socket.on_events();
            }
        });
        sockets.tcp_listen.iter().for_each(|socket| {
            if socket.has_events() {
                socket.on_events();
            }
        });
        sockets.udp.iter().for_each(|socket| {
            if socket.has_events() {
                socket.on_events();
            }
        });

        // Note that only TCP connections can have timers set, so as far as the time to poll is
        // concerned, we only need to consider TCP connections.
        sockets
            .tcp_conn
            .iter()
            .map(|socket| socket.next_poll_at_ms())
            .min()
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
