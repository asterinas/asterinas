// SPDX-License-Identifier: MPL-2.0

use alloc::collections::btree_map::Entry;
use core::sync::atomic::{AtomicU64, Ordering};

use keyable_arc::KeyableArc;
use ostd::sync::{LocalIrqDisabled, WaitQueue};
use smoltcp::{
    iface::{SocketHandle, SocketSet},
    phy::Device,
    wire::IpCidr,
};

use super::{
    any_socket::{AnyBoundSocketInner, AnyRawSocket, AnyUnboundSocket, SocketFamily},
    time::get_network_timestamp,
    util::BindPortConfig,
    AnyBoundSocket, Iface,
};
use crate::{net::socket::ip::Ipv4Address, prelude::*};

pub struct IfaceCommon {
    interface: SpinLock<smoltcp::iface::Interface>,
    sockets: SpinLock<SocketSet<'static>>,
    used_ports: RwLock<BTreeMap<u16, usize>>,
    /// The time should do next poll. We stores the total milliseconds since system boots up.
    next_poll_at_ms: AtomicU64,
    bound_sockets: RwLock<BTreeSet<KeyableArc<AnyBoundSocketInner>>>,
    closing_sockets: SpinLock<BTreeSet<KeyableArc<AnyBoundSocketInner>>>,
    /// The wait queue that background polling thread will sleep on
    polling_wait_queue: WaitQueue,
}

impl IfaceCommon {
    pub(super) fn new(interface: smoltcp::iface::Interface) -> Self {
        let socket_set = SocketSet::new(Vec::new());
        let used_ports = BTreeMap::new();
        Self {
            interface: SpinLock::new(interface),
            sockets: SpinLock::new(socket_set),
            used_ports: RwLock::new(used_ports),
            next_poll_at_ms: AtomicU64::new(0),
            bound_sockets: RwLock::new(BTreeSet::new()),
            closing_sockets: SpinLock::new(BTreeSet::new()),
            polling_wait_queue: WaitQueue::new(),
        }
    }

    /// Acquires the lock to the interface.
    ///
    /// *Lock ordering:* [`Self::sockets`] first, [`Self::interface`] second.
    pub(super) fn interface(&self) -> SpinLockGuard<smoltcp::iface::Interface, LocalIrqDisabled> {
        self.interface.disable_irq().lock()
    }

    /// Acuqires the lock to the sockets.
    ///
    /// *Lock ordering:* [`Self::sockets`] first, [`Self::interface`] second.
    pub(super) fn sockets(
        &self,
    ) -> SpinLockGuard<smoltcp::iface::SocketSet<'static>, LocalIrqDisabled> {
        self.sockets.disable_irq().lock()
    }

    pub(super) fn ipv4_addr(&self) -> Option<Ipv4Address> {
        self.interface.disable_irq().lock().ipv4_addr()
    }

    pub(super) fn netmask(&self) -> Option<Ipv4Address> {
        let interface = self.interface.disable_irq().lock();
        let ip_addrs = interface.ip_addrs();
        ip_addrs.first().map(|cidr| match cidr {
            IpCidr::Ipv4(ipv4_cidr) => ipv4_cidr.netmask(),
        })
    }

    pub(super) fn polling_wait_queue(&self) -> &WaitQueue {
        &self.polling_wait_queue
    }

    /// Alloc an unused port range from 49152 ~ 65535 (According to smoltcp docs)
    fn alloc_ephemeral_port(&self) -> Result<u16> {
        let mut used_ports = self.used_ports.write();
        for port in IP_LOCAL_PORT_START..=IP_LOCAL_PORT_END {
            if let Entry::Vacant(e) = used_ports.entry(port) {
                e.insert(0);
                return Ok(port);
            }
        }
        return_errno_with_message!(Errno::EAGAIN, "no ephemeral port is available");
    }

    fn bind_port(&self, port: u16, can_reuse: bool) -> Result<()> {
        let mut used_ports = self.used_ports.write();
        if let Some(used_times) = used_ports.get_mut(&port) {
            if *used_times == 0 || can_reuse {
                *used_times += 1;
            } else {
                return_errno_with_message!(Errno::EADDRINUSE, "the address is already in use");
            }
        } else {
            used_ports.insert(port, 1);
        }
        Ok(())
    }

    /// Release port number so the port can be used again. For reused port, the port may still be in use.
    pub(super) fn release_port(&self, port: u16) {
        let mut used_ports = self.used_ports.write();
        if let Some(used_times) = used_ports.remove(&port) {
            if used_times != 1 {
                used_ports.insert(port, used_times - 1);
            }
        }
    }

    pub(super) fn bind_socket(
        &self,
        iface: Arc<dyn Iface>,
        socket: Box<AnyUnboundSocket>,
        config: BindPortConfig,
    ) -> core::result::Result<AnyBoundSocket, (Error, Box<AnyUnboundSocket>)> {
        let port = if let Some(port) = config.port() {
            port
        } else {
            match self.alloc_ephemeral_port() {
                Ok(port) => port,
                Err(err) => return Err((err, socket)),
            }
        };
        if let Some(err) = self.bind_port(port, config.can_reuse()).err() {
            return Err((err, socket));
        }

        let (handle, socket_family, observer) = match socket.into_raw() {
            (AnyRawSocket::Tcp(tcp_socket), observer) => (
                self.sockets.disable_irq().lock().add(tcp_socket),
                SocketFamily::Tcp,
                observer,
            ),
            (AnyRawSocket::Udp(udp_socket), observer) => (
                self.sockets.disable_irq().lock().add(udp_socket),
                SocketFamily::Udp,
                observer,
            ),
        };
        let bound_socket = AnyBoundSocket::new(iface, handle, port, socket_family, observer);
        self.insert_bound_socket(bound_socket.inner());

        Ok(bound_socket)
    }

    /// Remove a socket from the interface
    pub(super) fn remove_socket(&self, handle: SocketHandle) {
        self.sockets.disable_irq().lock().remove(handle);
    }

    pub(super) fn poll<D: Device + ?Sized>(&self, device: &mut D) {
        let mut sockets = self.sockets.disable_irq().lock();
        let mut interface = self.interface.disable_irq().lock();

        let timestamp = get_network_timestamp();
        let (has_events, poll_at) = {
            let mut has_events = false;
            let mut poll_at;
            loop {
                while interface.poll(timestamp, device, &mut sockets) {
                    has_events = true;
                }
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

        if let Some(instant) = poll_at {
            let old_instant = self.next_poll_at_ms.load(Ordering::Relaxed);
            let new_instant = instant.total_millis() as u64;
            self.next_poll_at_ms.store(new_instant, Ordering::Relaxed);

            if old_instant == 0 || new_instant < old_instant {
                self.polling_wait_queue.wake_all();
            }
        } else {
            self.next_poll_at_ms.store(0, Ordering::Relaxed);
        }

        if has_events {
            // We never try to hold the write lock in the IRQ context, and we disable IRQ when
            // holding the write lock. So we don't need to disable IRQ when holding the read lock.
            self.bound_sockets.read().iter().for_each(|bound_socket| {
                bound_socket.on_iface_events();
            });

            let closed_sockets = self
                .closing_sockets
                .disable_irq()
                .lock()
                .extract_if(|closing_socket| closing_socket.is_closed())
                .collect::<Vec<_>>();
            drop(closed_sockets);
        }
    }

    pub(super) fn next_poll_at_ms(&self) -> Option<u64> {
        let millis = self.next_poll_at_ms.load(Ordering::Relaxed);
        if millis == 0 {
            None
        } else {
            Some(millis)
        }
    }

    fn insert_bound_socket(&self, socket: &Arc<AnyBoundSocketInner>) {
        let keyable_socket = KeyableArc::from(socket.clone());

        let inserted = self
            .bound_sockets
            .write_irq_disabled()
            .insert(keyable_socket);
        assert!(inserted);
    }

    pub(super) fn remove_bound_socket_now(&self, socket: &Arc<AnyBoundSocketInner>) {
        let keyable_socket = KeyableArc::from(socket.clone());

        let removed = self
            .bound_sockets
            .write_irq_disabled()
            .remove(&keyable_socket);
        assert!(removed);
    }

    pub(super) fn remove_bound_socket_when_closed(&self, socket: &Arc<AnyBoundSocketInner>) {
        let keyable_socket = KeyableArc::from(socket.clone());

        let removed = self
            .bound_sockets
            .write_irq_disabled()
            .remove(&keyable_socket);
        assert!(removed);

        let mut closing_sockets = self.closing_sockets.disable_irq().lock();

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
