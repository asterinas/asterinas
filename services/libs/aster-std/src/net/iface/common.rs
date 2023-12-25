use core::sync::atomic::{AtomicU64, Ordering};

use super::Ipv4Address;
use crate::prelude::*;
use alloc::collections::btree_map::Entry;
use keyable_arc::KeyableWeak;
use smoltcp::{
    iface::{SocketHandle, SocketSet},
    phy::Device,
    wire::IpCidr,
};

use super::{
    any_socket::{AnyBoundSocket, AnyRawSocket, AnyUnboundSocket},
    time::get_network_timestamp,
    util::BindPortConfig,
    Iface,
};

pub struct IfaceCommon {
    interface: SpinLock<smoltcp::iface::Interface>,
    sockets: SpinLock<SocketSet<'static>>,
    used_ports: RwLock<BTreeMap<u16, usize>>,
    /// The time should do next poll. We stores the total microseconds since system boots up.
    next_poll_at_ms: AtomicU64,
    bound_sockets: RwLock<BTreeSet<KeyableWeak<AnyBoundSocket>>>,
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

    /// Alloc an unused port range from 49152 ~ 65535 (According to smoltcp docs)
    fn alloc_ephemeral_port(&self) -> Result<u16> {
        let mut used_ports = self.used_ports.write();
        for port in IP_LOCAL_PORT_START..=IP_LOCAL_PORT_END {
            if let Entry::Vacant(e) = used_ports.entry(port) {
                e.insert(0);
                return Ok(port);
            }
        }
        return_errno_with_message!(Errno::EAGAIN, "cannot find unused high port");
    }

    fn bind_port(&self, port: u16, can_reuse: bool) -> Result<()> {
        let mut used_ports = self.used_ports.write();
        if let Some(used_times) = used_ports.get_mut(&port) {
            if *used_times == 0 || can_reuse {
                *used_times += 1;
            } else {
                return_errno_with_message!(Errno::EADDRINUSE, "cannot bind port");
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
                Err(e) => return Err((e, socket)),
            }
        };
        if let Some(e) = self.bind_port(port, config.can_reuse()).err() {
            return Err((e, socket));
        }
        let socket_family = socket.socket_family();
        let mut sockets = self.sockets.lock_irq_disabled();
        let handle = match socket.raw_socket_family() {
            AnyRawSocket::Tcp(tcp_socket) => sockets.add(tcp_socket),
            AnyRawSocket::Udp(udp_socket) => sockets.add(udp_socket),
        };
        let bound_socket = AnyBoundSocket::new(iface, handle, port, socket_family);
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
            self.next_poll_at_ms
                .store(instant.total_millis() as u64, Ordering::SeqCst);
        } else {
            self.next_poll_at_ms.store(0, Ordering::SeqCst);
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
