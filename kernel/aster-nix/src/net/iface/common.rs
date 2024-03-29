// SPDX-License-Identifier: MPL-2.0

use alloc::collections::btree_map::Entry;
use core::sync::atomic::{AtomicU64, Ordering};

use aster_frame::sync::WaitQueue;
use keyable_arc::KeyableWeak;
use smoltcp::{
    iface::{SocketHandle, SocketSet},
    phy::Device,
    wire::IpCidr,
};

use super::{
    any_socket::{AnyBoundSocket, AnyRawSocket, AnyUnboundSocket, SocketFamily},
    time::get_network_timestamp,
    util::BindPortConfig,
    Iface, Ipv4Address,
};
use crate::prelude::*;

pub struct IfaceCommon {
    interface: SpinLock<smoltcp::iface::Interface>,
    sockets: SpinLock<SocketSet<'static>>,
    used_ports: RwLock<BTreeMap<u16, usize>>,
    /// The time should do next poll. We stores the total milliseconds since system boots up.
    next_poll_at_ms: AtomicU64,
    bound_sockets: RwLock<BTreeSet<KeyableWeak<AnyBoundSocket>>>,
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
            polling_wait_queue: WaitQueue::new(),
        }
    }

    pub(super) fn interface(&self) -> SpinLockGuard<smoltcp::iface::Interface> {
        self.interface.lock_irq_disabled()
    }

    pub(super) fn sockets(&self) -> SpinLockGuard<smoltcp::iface::SocketSet<'static>> {
        self.sockets.lock_irq_disabled()
    }

    pub(super) fn ipv4_addr(&self) -> Option<Ipv4Address> {
        self.interface.lock_irq_disabled().ipv4_addr()
    }

    pub(super) fn netmask(&self) -> Option<Ipv4Address> {
        let interface = self.interface.lock_irq_disabled();
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
    ) -> core::result::Result<Arc<AnyBoundSocket>, (Error, Box<AnyUnboundSocket>)> {
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
                self.sockets.lock_irq_disabled().add(tcp_socket),
                SocketFamily::Tcp,
                observer,
            ),
            (AnyRawSocket::Udp(udp_socket), observer) => (
                self.sockets.lock_irq_disabled().add(udp_socket),
                SocketFamily::Udp,
                observer,
            ),
        };
        let bound_socket = AnyBoundSocket::new(iface, handle, port, socket_family, observer);
        self.insert_bound_socket(&bound_socket).unwrap();

        Ok(bound_socket)
    }

    /// Remove a socket from the interface
    pub(super) fn remove_socket(&self, handle: SocketHandle) {
        self.sockets.lock_irq_disabled().remove(handle);
    }

    pub(super) fn poll<D: Device + ?Sized>(&self, device: &mut D) {
        let mut interface = self.interface.lock_irq_disabled();
        let timestamp = get_network_timestamp();
        let has_events = {
            let mut sockets = self.sockets.lock_irq_disabled();
            interface.poll(timestamp, device, &mut sockets)
            // drop sockets here to avoid deadlock
        };
        if has_events {
            self.bound_sockets.read().iter().for_each(|bound_socket| {
                if let Some(bound_socket) = bound_socket.upgrade() {
                    bound_socket.on_iface_events();
                }
            });
        }

        let sockets = self.sockets.lock_irq_disabled();
        if let Some(instant) = interface.poll_at(timestamp, &sockets) {
            let old_instant = self.next_poll_at_ms.load(Ordering::Acquire);
            let new_instant = instant.total_millis() as u64;
            self.next_poll_at_ms
                .store(instant.total_millis() as u64, Ordering::Relaxed);

            if new_instant < old_instant {
                self.polling_wait_queue.wake_all();
            }
        } else {
            self.next_poll_at_ms.store(0, Ordering::Relaxed);
        }
    }

    pub(super) fn next_poll_at_ms(&self) -> Option<u64> {
        let millis = self.next_poll_at_ms.load(Ordering::SeqCst);
        if millis == 0 {
            None
        } else {
            Some(millis)
        }
    }

    fn insert_bound_socket(&self, socket: &Arc<AnyBoundSocket>) -> Result<()> {
        let weak_ref = KeyableWeak::from(Arc::downgrade(socket));
        let mut bound_sockets = self.bound_sockets.write();
        if bound_sockets.contains(&weak_ref) {
            return_errno_with_message!(Errno::EINVAL, "the socket is already bound");
        }
        bound_sockets.insert(weak_ref);
        Ok(())
    }

    pub(super) fn remove_bound_socket(&self, socket: Weak<AnyBoundSocket>) {
        let weak_ref = KeyableWeak::from(socket);
        self.bound_sockets.write().remove(&weak_ref);
    }
}

const IP_LOCAL_PORT_START: u16 = 49152;
const IP_LOCAL_PORT_END: u16 = 65535;
