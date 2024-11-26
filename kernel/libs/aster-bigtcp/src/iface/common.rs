// SPDX-License-Identifier: MPL-2.0

use alloc::{
    boxed::Box,
    collections::{
        btree_map::{BTreeMap, Entry},
        btree_set::BTreeSet,
    },
    sync::Arc,
};

use keyable_arc::KeyableArc;
use ostd::sync::{LocalIrqDisabled, PreemptDisabled, SpinLock, SpinLockGuard};
use smoltcp::{
    iface::{packet::Packet, Context},
    phy::Device,
    wire::{Ipv4Address, Ipv4Packet},
};

use super::{
    poll::{FnHelper, PollContext},
    port::BindPortConfig,
    time::get_network_timestamp,
    Iface,
};
use crate::{
    errors::BindError,
    socket::{
        BoundTcpSocket, BoundTcpSocketInner, BoundUdpSocket, BoundUdpSocketInner, UnboundTcpSocket,
        UnboundUdpSocket,
    },
};

pub struct IfaceCommon<E> {
    interface: SpinLock<smoltcp::iface::Interface, LocalIrqDisabled>,
    used_ports: SpinLock<BTreeMap<u16, usize>, PreemptDisabled>,
    tcp_sockets: SpinLock<BTreeSet<KeyableArc<BoundTcpSocketInner<E>>>, LocalIrqDisabled>,
    udp_sockets: SpinLock<BTreeSet<KeyableArc<BoundUdpSocketInner<E>>>, LocalIrqDisabled>,
    ext: E,
}

impl<E> IfaceCommon<E> {
    pub(super) fn new(interface: smoltcp::iface::Interface, ext: E) -> Self {
        Self {
            interface: SpinLock::new(interface),
            used_ports: SpinLock::new(BTreeMap::new()),
            tcp_sockets: SpinLock::new(BTreeSet::new()),
            udp_sockets: SpinLock::new(BTreeSet::new()),
            ext,
        }
    }

    pub(super) fn ipv4_addr(&self) -> Option<Ipv4Address> {
        self.interface.lock().ipv4_addr()
    }

    pub(super) fn ext(&self) -> &E {
        &self.ext
    }
}

impl<E> IfaceCommon<E> {
    /// Acquires the lock to the interface.
    pub(crate) fn interface(&self) -> SpinLockGuard<smoltcp::iface::Interface, LocalIrqDisabled> {
        self.interface.lock()
    }
}

const IP_LOCAL_PORT_START: u16 = 32768;
const IP_LOCAL_PORT_END: u16 = 60999;

impl<E> IfaceCommon<E> {
    pub(super) fn bind_tcp(
        &self,
        iface: Arc<dyn Iface<E>>,
        socket: Box<UnboundTcpSocket>,
        config: BindPortConfig,
    ) -> core::result::Result<BoundTcpSocket<E>, (BindError, Box<UnboundTcpSocket>)> {
        let port = match self.bind_port(config) {
            Ok(port) => port,
            Err(err) => return Err((err, socket)),
        };

        let (raw_socket, observer) = socket.into_raw();
        let bound_socket = BoundTcpSocket::new(iface, port, raw_socket, observer);

        let inserted = self
            .tcp_sockets
            .lock()
            .insert(KeyableArc::from(bound_socket.inner().clone()));
        assert!(inserted);

        Ok(bound_socket)
    }

    pub(super) fn bind_udp(
        &self,
        iface: Arc<dyn Iface<E>>,
        socket: Box<UnboundUdpSocket>,
        config: BindPortConfig,
    ) -> core::result::Result<BoundUdpSocket<E>, (BindError, Box<UnboundUdpSocket>)> {
        let port = match self.bind_port(config) {
            Ok(port) => port,
            Err(err) => return Err((err, socket)),
        };

        let (raw_socket, observer) = socket.into_raw();
        let bound_socket = BoundUdpSocket::new(iface, port, raw_socket, observer);

        let inserted = self
            .udp_sockets
            .lock()
            .insert(KeyableArc::from(bound_socket.inner().clone()));
        assert!(inserted);

        Ok(bound_socket)
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
}

impl<E> IfaceCommon<E> {
    #[allow(clippy::mutable_key_type)]
    fn remove_dead_tcp_sockets(&self, sockets: &mut BTreeSet<KeyableArc<BoundTcpSocketInner<E>>>) {
        sockets.retain(|socket| {
            if socket.is_dead() {
                self.release_port(socket.port());
                false
            } else {
                true
            }
        });
    }

    pub(crate) fn remove_udp_socket(&self, socket: &Arc<BoundUdpSocketInner<E>>) {
        let keyable_socket = KeyableArc::from(socket.clone());

        let removed = self.udp_sockets.lock().remove(&keyable_socket);
        assert!(removed);

        self.release_port(keyable_socket.port());
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

impl<E> IfaceCommon<E> {
    pub(super) fn poll<D, P, Q>(
        &self,
        device: &mut D,
        process_phy: P,
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

        let mut tcp_sockets = self.tcp_sockets.lock();
        let udp_sockets = self.udp_sockets.lock();

        let mut context = PollContext::new(interface.context(), &tcp_sockets, &udp_sockets);
        context.poll_ingress(device, process_phy, &mut dispatch_phy);
        context.poll_egress(device, dispatch_phy);

        tcp_sockets.iter().for_each(|socket| {
            if socket.has_events() {
                socket.on_events();
            }
        });
        udp_sockets.iter().for_each(|socket| {
            if socket.has_events() {
                socket.on_events();
            }
        });

        self.remove_dead_tcp_sockets(&mut tcp_sockets);

        match (
            tcp_sockets
                .iter()
                .map(|socket| socket.next_poll_at_ms())
                .min(),
            udp_sockets
                .iter()
                .map(|socket| socket.next_poll_at_ms())
                .min(),
        ) {
            (Some(tcp_poll_at), Some(udp_poll_at)) if tcp_poll_at <= udp_poll_at => {
                Some(tcp_poll_at)
            }
            (tcp_poll_at, None) => tcp_poll_at,
            (_, udp_poll_at) => udp_poll_at,
        }
    }
}
