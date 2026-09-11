// SPDX-License-Identifier: MPL-2.0

use aster_softirq::BottomHalfDisabled;
use aster_virtio::device::socket::{
    device::SocketDevice,
    header::{VirtioVsockHdr, VirtioVsockOp, VirtioVsockShutdownFlags, VirtioVsockType},
    packet::TxPacket,
};
use ostd::sync::PreemptDisabled;
use spin::Once;

use crate::{
    events::IoEvents,
    net::socket::vsock::{
        addr::{VMADDR_CID_HOST, VsockSocketAddr},
        transport::{
            BoundPort, Connection, Listener,
            conn_id::ConnId,
            connection::{ConnectionInner, RxPayload},
            listener::ListenerInner,
            port::PortTable,
            timer::TimerEvent,
        },
    },
    prelude::*,
    process::signal::Pollee,
};

// A machine uses either the guest frontend or the host backend. Supporting both requires
// selecting a transport per connection and separating their local port namespaces.
pub(super) struct VsockSpace {
    backend: VsockBackend,
    ports: SpinLock<PortTable>,
    sockets: SpinLock<SocketTable, BottomHalfDisabled>,
}

enum VsockBackend {
    Virtio(Arc<SocketDevice>),
    Vhost,
}

struct SocketTable {
    connections: BTreeMap<ConnId, Arc<ConnectionInner>>,
    listeners: BTreeMap<u32, Arc<ListenerInner>>,
}

impl VsockSpace {
    fn new(backend: VsockBackend) -> Self {
        Self {
            backend,
            ports: SpinLock::new(PortTable::new()),
            sockets: SpinLock::new(SocketTable {
                connections: BTreeMap::new(),
                listeners: BTreeMap::new(),
            }),
        }
    }

    pub(super) fn virtio_device(&self) -> Option<&SocketDevice> {
        match &self.backend {
            VsockBackend::Virtio(device) => Some(device),
            VsockBackend::Vhost => None,
        }
    }

    pub(super) fn is_vhost_backend(&self) -> bool {
        matches!(self.backend, VsockBackend::Vhost)
    }

    pub(super) fn guest_cid(&self) -> u64 {
        match &self.backend {
            VsockBackend::Virtio(device) => device.guest_cid(),
            VsockBackend::Vhost => VMADDR_CID_HOST as u64,
        }
    }

    pub(super) fn can_connect_remote_cid(&self, cid: u32) -> bool {
        match &self.backend {
            VsockBackend::Virtio(_) => cid == VMADDR_CID_HOST,
            VsockBackend::Vhost => crate::device::misc::vhost_vsock::can_connect_remote_cid(cid),
        }
    }

    pub(super) fn lock_ports(&self) -> SpinLockGuard<'_, PortTable, PreemptDisabled> {
        self.ports.lock()
    }
}

// Connection and listener management.
impl VsockSpace {
    pub(super) fn new_connection(
        &self,
        bound_port: BoundPort,
        remote_addr: VsockSocketAddr,
        pollee: &Pollee,
    ) -> Result<Connection, (Error, BoundPort)> {
        use alloc::collections::btree_map::Entry;

        let mut sockets = self.sockets.lock();

        // Recheck under the same lock used by backend teardown, so a concurrent stop cannot
        // leave a newly inserted connection behind after clearing this CID's connections.
        if !self.can_connect_remote_cid(remote_addr.cid) {
            return Err((
                Error::with_message(Errno::ENETUNREACH, "the vsock CID is not reachable"),
                bound_port,
            ));
        }

        // Note that we should query the guest CID (part of `from_port_and_remote`) after locking
        // `sockets` to avoid race conditions with `process_transport_event`.
        let conn_id = ConnId::from_port_and_remote(&bound_port, remote_addr);
        let Entry::Vacant(entry) = sockets.connections.entry(conn_id) else {
            return Err((
                Error::with_message(Errno::EADDRINUSE, "the vsock connection already exists"),
                bound_port,
            ));
        };

        let inner = ConnectionInner::new_connecting(bound_port, &conn_id, pollee.clone());
        entry.insert(inner.clone());

        Ok(Connection::new(inner))
    }

    /// Removes a connection.
    ///
    /// This method should not be called with the socket state lock held. We need to lock the
    /// sockets before locking the socket state.
    pub(super) fn remove_connection(&self, connection: &Arc<ConnectionInner>) {
        use alloc::collections::btree_map::Entry;

        let mut sockets = self.sockets.lock();

        let conn_id = connection.conn_id();

        // The following early returns may reach due to some race conditions between user
        // operations and incoming packets. But we can safely ignore this case if it happens.
        let Entry::Occupied(occupied) = sockets.connections.entry(conn_id) else {
            return;
        };
        if !Arc::ptr_eq(connection, occupied.get()) {
            return;
        }

        occupied.remove();
    }

    pub(super) fn new_listener(
        &self,
        bound_port: BoundPort,
        backlog: usize,
        pollee: &Pollee,
    ) -> Result<Listener, (Error, BoundPort)> {
        use alloc::collections::btree_map::Entry;

        let mut sockets = self.sockets.lock();

        let port = bound_port.port();
        let Entry::Vacant(entry) = sockets.listeners.entry(port) else {
            return Err((
                Error::with_message(Errno::EADDRINUSE, "the vsock listener already exists"),
                bound_port,
            ));
        };

        let inner = ListenerInner::new(bound_port, backlog, pollee.clone());
        entry.insert(inner.clone());

        Ok(Listener::new(inner))
    }

    /// Removes a listener.
    ///
    /// This method should not be called with the socket state lock held. We need to lock the
    /// sockets before locking the socket state.
    pub(super) fn remove_listener(&self, listener: &Arc<ListenerInner>) {
        use alloc::collections::btree_map::Entry;

        let mut sockets = self.sockets.lock();

        let port = listener.bound_port().port();
        let removed = sockets.listeners.remove(&port);
        debug_assert!(
            removed
                .as_ref()
                .is_some_and(|removed| Arc::ptr_eq(removed, listener))
        );

        let connections = listener.take_incoming_on_removal();
        for connection in connections.into_iter() {
            let conn_id = connection.conn_id();

            // The following `continue`s may reach due to some race conditions between user
            // operations and incoming packets. But we can safely ignore this case if it happens.
            let Entry::Occupied(occupied) = sockets.connections.entry(conn_id) else {
                continue;
            };
            if !Arc::ptr_eq(&connection, occupied.get()) {
                continue;
            }

            occupied.remove();

            connection.active_rst();
            // No need to notify the pollee since the connection isn't even accepted.
        }
    }
}

// RX packet and transport event processing.
impl VsockSpace {
    pub(super) fn process_rx(&self) {
        // Lock order: device RX -> sockets -> socket state -> device TX

        let device = self.virtio_device().unwrap();
        let mut rx = device.lock_rx();
        let mut sockets = self.sockets.lock();

        while let Some(packet) = rx.recv() {
            let header = packet.header();
            self.process_rx_packet(&mut sockets, &header, RxPayload::Virtio(packet));
        }
    }

    pub(super) fn process_vhost_packet(&self, header: VirtioVsockHdr, payload: &[u8]) {
        let packet = RxPayload::Vhost(payload.to_vec());
        let mut sockets = self.sockets.lock();
        self.process_rx_packet(&mut sockets, &header, packet);
    }

    fn process_rx_packet(
        &self,
        sockets: &mut SocketTable,
        header: &VirtioVsockHdr,
        packet: RxPayload,
    ) {
        use alloc::collections::btree_map::Entry;

        let conn_id = ConnId::from_incoming_header(header);
        let entry = sockets.connections.entry(conn_id);

        match entry {
            Entry::Vacant(vacant) => {
                self.process_rx_with_listener(&sockets.listeners, vacant, header, packet)
            }
            Entry::Occupied(occupied) => self.process_rx_with_connection(occupied, header, packet),
        }
    }

    fn process_rx_with_listener(
        &self,
        listeners: &BTreeMap<u32, Arc<ListenerInner>>,
        vacant_conn: alloc::collections::btree_map::VacantEntry<'_, ConnId, Arc<ConnectionInner>>,
        header: &VirtioVsockHdr,
        packet: RxPayload,
    ) {
        if header.op() != Some(VirtioVsockOp::Request)
            || !self.validate_rx_header(VirtioVsockOp::Request, header, &packet)
        {
            self.send_raw_rst(header);
            return;
        }

        let dst_port = header.dst_port;
        let listener = if let Some(listener) = listeners.get(&dst_port)
            && !listener.is_full()
        {
            listener
        } else {
            self.send_raw_rst(header);
            return;
        };

        let bound_port = BoundPort::new_shared(listener.bound_port());
        let conn_id = vacant_conn.key();

        let inner = ConnectionInner::new_connected(bound_port, conn_id, header);
        vacant_conn.insert(inner.clone());

        listener.push_incoming(inner.clone());
    }

    fn process_rx_with_connection(
        &self,
        occupied_conn: alloc::collections::btree_map::OccupiedEntry<
            '_,
            ConnId,
            Arc<ConnectionInner>,
        >,
        header: &VirtioVsockHdr,
        packet: RxPayload,
    ) {
        let op = if let Some(op) = header.op()
            && self.validate_rx_header(op, header, &packet)
        {
            op
        } else {
            Self::reset_removed_connection(occupied_conn.remove());
            return;
        };

        let connection = occupied_conn.get();

        let should_remove = match op {
            VirtioVsockOp::Request => {
                connection.active_rst();
                true
            }
            VirtioVsockOp::Response => connection.on_response(header).is_err(),
            VirtioVsockOp::Rst => {
                connection.on_rst();
                true
            }
            VirtioVsockOp::Shutdown => connection.on_shutdown(header),
            VirtioVsockOp::Rw => connection.on_rw(header, packet).is_err(),
            VirtioVsockOp::CreditUpdate => connection.on_credit_update(header).is_err(),
            VirtioVsockOp::CreditRequest => connection.on_credit_request(header).is_err(),
        };

        if should_remove {
            Self::notify_removed_connection(occupied_conn.remove());
        }
    }

    fn send_raw_rst(&self, header: &VirtioVsockHdr) {
        if header.op == VirtioVsockOp::Rst as u16 {
            // Do not send an RST packet in response to an RST packet. Otherwise, we may loop.
            return;
        }

        // We do not use `VirtioVsockHdr::new` here because we want to specify the `type_` field. It
        // may not be `VirtioVsockType::Stream`.
        let rst_header = VirtioVsockHdr {
            src_cid: header.dst_cid,
            dst_cid: header.src_cid,
            src_port: header.dst_port,
            dst_port: header.src_port,
            len: 0,
            type_: header.type_,
            op: VirtioVsockOp::Rst as u16,
            flags: 0,
            buf_alloc: 0,
            fwd_cnt: 0,
        };
        let _ = self.send_packet(&rst_header);
    }

    pub(super) fn process_transport_event(&self) {
        // Lock order: sockets -> socket state

        let mut sockets = self.sockets.lock();

        // As stated in the specification, we only need to deal with the connections:
        // "The driver shuts down established connections and the guest_cid configuration field is
        // fetched again. Existing listen sockets remain but their CID is updated to reflect the
        // current guest_cid."

        let connections = core::mem::take(&mut sockets.connections);
        for connection in connections.into_values() {
            connection.on_rst();
            Self::notify_removed_connection(connection);
        }

        // The reload of the guest CID is protectd by the `sockets` lock.
        self.virtio_device().unwrap().reload_guest_id();
    }

    pub(super) fn reset_vhost_connections(&self, cid: u32) {
        // The device has stopped routing this CID before entering here. Lock order remains
        // sockets -> socket state; callers must not hold a device or backend registry lock.
        let mut sockets = self.sockets.lock();
        let mut pollees = Vec::new();
        sockets.connections.retain(|conn_id, connection| {
            if conn_id.peer_cid != cid as u64 {
                return true;
            }

            connection.on_transport_reset();
            pollees.push(connection.pollee().clone());
            false
        });

        // Connecting sockets can finish only after the table drops its Arc. In particular,
        // notifying from inside retain would let a waiter observe a result that is not ready.
        for pollee in pollees {
            pollee.notify(
                IoEvents::IN | IoEvents::OUT | IoEvents::RDHUP | IoEvents::HUP | IoEvents::ERR,
            );
        }
    }

    pub(super) fn notify_vhost_writable(&self, cid: u32) {
        let sockets = self.sockets.lock();
        for (conn_id, connection) in &sockets.connections {
            if conn_id.peer_cid == cid as u64 {
                connection.pollee().notify(IoEvents::OUT);
            }
        }
    }

    pub(super) fn process_timer_events(&self, events: Vec<TimerEvent>) {
        use alloc::collections::btree_map::Entry;

        // Lock order: sockets -> socket state

        let mut sockets = self.sockets.lock();

        for event in events.into_iter() {
            let Entry::Occupied(entry) = sockets.connections.entry(event.conn_id) else {
                continue;
            };

            if !entry.get().on_timeout(event.generation) {
                continue;
            }

            Self::notify_removed_connection(entry.remove());
        }
    }

    fn reset_removed_connection(connection: Arc<ConnectionInner>) {
        connection.active_rst();
        Self::notify_removed_connection(connection);
    }

    fn notify_removed_connection(connection: Arc<ConnectionInner>) {
        // A reset connection may still be in the listener's accept queue. This is a deliberate
        // design choice, as we currently lack the means to efficiently locate the listener. This
        // should be harmless to the user space because a connection can always be reset just after
        // being accepted.
        //
        // FIXME: This may not be consistent with Linux behavior.

        let pollee = connection.pollee().clone();
        drop(connection);

        // Notify the pollee after dropping the connection. This ensures the connection's reference
        // count is one, allowing the socket layer to use the `Connection::into_result` method.
        pollee
            .notify(IoEvents::IN | IoEvents::OUT | IoEvents::RDHUP | IoEvents::HUP | IoEvents::ERR);
    }

    /// Sends a control packet which does not carry payload.
    ///
    /// This method may be called while holding the socket state lock.
    //
    // TODO: This method may fail if memory allocation fails. For now, we will ignore the error in
    // most cases. If possible, we should find better ways to handle the error.
    #[must_use]
    pub(super) fn send_packet(&self, header: &VirtioVsockHdr) -> bool {
        let Some(device) = self.virtio_device() else {
            return match crate::device::misc::vhost_vsock::send_packet(header, &[]) {
                Ok(sent) => sent,
                Err(err) => {
                    warn!("failed to send vhost-vsock control packet: {:?}", err);
                    false
                }
            };
        };

        let Ok(builder) = TxPacket::new_builder() else {
            warn!("failed to allocate vsock packet: {:?}", header);
            return false;
        };
        let packet = builder.build(header);

        // Lock order: socket state -> device TX

        let mut tx = device.lock_tx();
        match tx.try_send(packet) {
            Ok(()) => (),
            Err(pending) => {
                // TODO: Ideally, we should limit the number of in-flight packets. This will prevent
                // the peer endpoint from exhausting a large amount of memory by triggering too many
                // packets but never processing them.
                pending.push_pending(None);
            }
        }

        true
    }

    fn validate_rx_header(
        &self,
        op: VirtioVsockOp,
        header: &VirtioVsockHdr,
        packet: &RxPayload,
    ) -> bool {
        if header.type_ != VirtioVsockType::Stream as u16 {
            return false;
        }

        if header.dst_cid != self.guest_cid() {
            return false;
        }

        let payload_len = packet.payload_len();
        if payload_len != header.len as usize {
            return false;
        }

        match op {
            VirtioVsockOp::Request
            | VirtioVsockOp::Response
            | VirtioVsockOp::Rst
            | VirtioVsockOp::CreditUpdate
            | VirtioVsockOp::CreditRequest => payload_len == 0 && header.flags == 0,
            VirtioVsockOp::Shutdown => {
                payload_len == 0 && VirtioVsockShutdownFlags::from_bits(header.flags).is_some()
            }
            VirtioVsockOp::Rw => header.flags == 0,
        }
    }
}

static VSOCK_SPACE: Once<VsockSpace> = Once::new();

pub(super) fn vsock_space() -> Result<&'static VsockSpace> {
    VSOCK_SPACE
        .get()
        .ok_or_else(|| Error::with_message(Errno::ENODEV, "no vsock transport is available"))
}

pub(super) fn init(device: Arc<SocketDevice>) {
    VSOCK_SPACE.call_once(move || VsockSpace::new(VsockBackend::Virtio(device)));
}

pub(super) fn init_host_vhost() {
    VSOCK_SPACE.call_once(|| VsockSpace::new(VsockBackend::Vhost));
}
