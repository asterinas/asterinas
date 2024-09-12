// SPDX-License-Identifier: MPL-2.0

use alloc::{
    boxed::Box,
    collections::{
        btree_map::{BTreeMap, Entry},
        btree_set::BTreeSet,
    },
    sync::Arc,
    vec::Vec,
};

use keyable_arc::KeyableArc;
use ostd::sync::{LocalIrqDisabled, RwLock, SpinLock, SpinLockGuard};
use smoltcp::{
    iface::{SocketHandle, SocketSet},
    phy::Device,
    wire::Ipv4Address,
};

use super::{port::BindPortConfig, time::get_network_timestamp, Iface};
use crate::{
    errors::BindError,
    socket::{AnyBoundSocket, AnyBoundSocketInner, AnyRawSocket, AnyUnboundSocket, SocketFamily},
};

pub struct IfaceCommon<E> {
    interface: SpinLock<smoltcp::iface::Interface, LocalIrqDisabled>,
    sockets: SpinLock<SocketSet<'static>, LocalIrqDisabled>,
    used_ports: RwLock<BTreeMap<u16, usize>>,
    bound_sockets: RwLock<BTreeSet<KeyableArc<AnyBoundSocketInner<E>>>>,
    closing_sockets: SpinLock<BTreeSet<KeyableArc<AnyBoundSocketInner<E>>>, LocalIrqDisabled>,
    ext: E,
}

impl<E> IfaceCommon<E> {
    pub(super) fn new(interface: smoltcp::iface::Interface, ext: E) -> Self {
        let socket_set = SocketSet::new(Vec::new());
        let used_ports = BTreeMap::new();
        Self {
            interface: SpinLock::new(interface),
            sockets: SpinLock::new(socket_set),
            used_ports: RwLock::new(used_ports),
            bound_sockets: RwLock::new(BTreeSet::new()),
            closing_sockets: SpinLock::new(BTreeSet::new()),
            ext,
        }
    }

    /// Acquires the lock to the interface.
    ///
    /// *Lock ordering:* [`Self::sockets`] first, [`Self::interface`] second.
    pub(crate) fn interface(&self) -> SpinLockGuard<smoltcp::iface::Interface, LocalIrqDisabled> {
        self.interface.lock()
    }

    /// Acuqires the lock to the sockets.
    ///
    /// *Lock ordering:* [`Self::sockets`] first, [`Self::interface`] second.
    pub(crate) fn sockets(
        &self,
    ) -> SpinLockGuard<smoltcp::iface::SocketSet<'static>, LocalIrqDisabled> {
        self.sockets.lock()
    }

    pub(super) fn ipv4_addr(&self) -> Option<Ipv4Address> {
        self.interface.lock().ipv4_addr()
    }

    /// Alloc an unused port range from 49152 ~ 65535 (According to smoltcp docs)
    fn alloc_ephemeral_port(&self) -> Option<u16> {
        let mut used_ports = self.used_ports.write();
        for port in IP_LOCAL_PORT_START..=IP_LOCAL_PORT_END {
            if let Entry::Vacant(e) = used_ports.entry(port) {
                e.insert(0);
                return Some(port);
            }
        }
        None
    }

    #[must_use]
    fn bind_port(&self, port: u16, can_reuse: bool) -> bool {
        let mut used_ports = self.used_ports.write();
        if let Some(used_times) = used_ports.get_mut(&port) {
            if *used_times == 0 || can_reuse {
                // FIXME: Check if the previous socket was bound with SO_REUSEADDR.
                *used_times += 1;
            } else {
                return false;
            }
        } else {
            used_ports.insert(port, 1);
        }
        true
    }

    /// Release port number so the port can be used again. For reused port, the port may still be in use.
    pub(crate) fn release_port(&self, port: u16) {
        let mut used_ports = self.used_ports.write();
        if let Some(used_times) = used_ports.remove(&port) {
            if used_times != 1 {
                used_ports.insert(port, used_times - 1);
            }
        }
    }

    pub(super) fn bind_socket(
        &self,
        iface: Arc<dyn Iface<E>>,
        socket: Box<AnyUnboundSocket>,
        config: BindPortConfig,
    ) -> core::result::Result<AnyBoundSocket<E>, (BindError, Box<AnyUnboundSocket>)> {
        let port = if let Some(port) = config.port() {
            port
        } else {
            match self.alloc_ephemeral_port() {
                Some(port) => port,
                None => return Err((BindError::Exhausted, socket)),
            }
        };
        if !self.bind_port(port, config.can_reuse()) {
            return Err((BindError::InUse, socket));
        }

        let (handle, socket_family, observer) = match socket.into_raw() {
            (AnyRawSocket::Tcp(tcp_socket), observer) => (
                self.sockets.lock().add(tcp_socket),
                SocketFamily::Tcp,
                observer,
            ),
            (AnyRawSocket::Udp(udp_socket), observer) => (
                self.sockets.lock().add(udp_socket),
                SocketFamily::Udp,
                observer,
            ),
        };
        let bound_socket = AnyBoundSocket::new(iface, handle, port, socket_family, observer);
        self.insert_bound_socket(bound_socket.inner());

        Ok(bound_socket)
    }

    /// Remove a socket from the interface
    pub(crate) fn remove_socket(&self, handle: SocketHandle) {
        self.sockets.lock().remove(handle);
    }

    #[must_use]
    pub(super) fn poll<D: Device + ?Sized>(&self, device: &mut D) -> Option<u64> {
        let mut sockets = self.sockets.lock();
        let mut interface = self.interface.lock();

        let timestamp = get_network_timestamp();
        let (has_events, poll_at) = {
            let mut has_events = false;
            let mut poll_at;

            loop {
                // `poll` transmits and receives a bounded number of packets. This loop ensures
                // that all packets are transmitted and received. For details, see
                // <https://github.com/smoltcp-rs/smoltcp/blob/8e3ea5c7f09a76f0a4988fda20cadc74eacdc0d8/src/iface/interface/mod.rs#L400-L405>.
                while interface.poll(timestamp, device, &mut sockets) {
                    has_events = true;
                }

                // `poll_at` can return `Some(Instant::from_millis(0))`, which means `PollAt::Now`.
                // For details, see
                // <https://github.com/smoltcp-rs/smoltcp/blob/8e3ea5c7f09a76f0a4988fda20cadc74eacdc0d8/src/iface/interface/mod.rs#L478>.
                poll_at = interface.poll_at(timestamp, &sockets);
                let Some(instant) = poll_at else {
                    break;
                };
                if instant > timestamp {
                    break;
                }
            }

            (has_events, poll_at)
        };

        // drop sockets here to avoid deadlock
        drop(interface);
        drop(sockets);

        if has_events {
            // We never try to hold the write lock in the IRQ context, and we disable IRQ when
            // holding the write lock. So we don't need to disable IRQ when holding the read lock.
            self.bound_sockets.read().iter().for_each(|bound_socket| {
                bound_socket.on_iface_events();
            });

            let closed_sockets = self
                .closing_sockets
                .lock()
                .extract_if(|closing_socket| closing_socket.is_closed())
                .collect::<Vec<_>>();
            drop(closed_sockets);
        }

        poll_at.map(|at| smoltcp::time::Instant::total_millis(&at) as u64)
    }

    pub(super) fn ext(&self) -> &E {
        &self.ext
    }

    fn insert_bound_socket(&self, socket: &Arc<AnyBoundSocketInner<E>>) {
        let keyable_socket = KeyableArc::from(socket.clone());

        let inserted = self
            .bound_sockets
            .write_irq_disabled()
            .insert(keyable_socket);
        assert!(inserted);
    }

    pub(crate) fn remove_bound_socket_now(&self, socket: &Arc<AnyBoundSocketInner<E>>) {
        let keyable_socket = KeyableArc::from(socket.clone());

        let removed = self
            .bound_sockets
            .write_irq_disabled()
            .remove(&keyable_socket);
        assert!(removed);
    }

    pub(crate) fn remove_bound_socket_when_closed(&self, socket: &Arc<AnyBoundSocketInner<E>>) {
        let keyable_socket = KeyableArc::from(socket.clone());

        let removed = self
            .bound_sockets
            .write_irq_disabled()
            .remove(&keyable_socket);
        assert!(removed);

        let mut closing_sockets = self.closing_sockets.lock();

        // Check `is_closed` after holding the lock to avoid race conditions.
        if keyable_socket.is_closed() {
            return;
        }

        let inserted = closing_sockets.insert(keyable_socket);
        assert!(inserted);
    }
}

const IP_LOCAL_PORT_START: u16 = 49152;
const IP_LOCAL_PORT_END: u16 = 65535;
