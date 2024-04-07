// SPDX-License-Identifier: MPL-2.0

use alloc::collections::BTreeSet;

use aster_virtio::device::socket::{
    connect::{VsockEvent, VsockEventType},
    device::SocketDevice,
    error::SocketError,
    get_device, DEVICE_NAME,
};

use super::{
    addr::VsockSocketAddr,
    stream::{
        connected::{Connected, ConnectionID},
        listen::Listen,
    },
};
use crate::{events::IoEvents, prelude::*, return_errno_with_message};

/// Manage all active sockets
pub struct VsockSpace {
    pub driver: Arc<SpinLock<SocketDevice>>,
    // (key, value) = (local_addr, connecting)
    pub connecting_sockets: SpinLock<BTreeMap<VsockSocketAddr, Arc<Connected>>>,
    // (key, value) = (local_addr, listen)
    pub listen_sockets: SpinLock<BTreeMap<VsockSocketAddr, Arc<Listen>>>,
    // (key, value) = (id(local_addr,peer_addr), connected)
    pub connected_sockets: SpinLock<BTreeMap<ConnectionID, Arc<Connected>>>,
    // Used ports
    pub used_ports: SpinLock<BTreeSet<u32>>,
}

impl VsockSpace {
    /// Create a new global VsockSpace
    pub fn new() -> Self {
        let driver = get_device(DEVICE_NAME).unwrap();
        Self {
            driver,
            connecting_sockets: SpinLock::new(BTreeMap::new()),
            listen_sockets: SpinLock::new(BTreeMap::new()),
            connected_sockets: SpinLock::new(BTreeMap::new()),
            used_ports: SpinLock::new(BTreeSet::new()),
        }
    }
    /// Poll for each event from the driver
    pub fn poll(&self) -> Result<Option<VsockEvent>> {
        let mut driver = self.driver.lock_irq_disabled();
        let guest_cid: u32 = driver.guest_cid() as u32;

        // match the socket and store the buffer body (if valid)
        let result = driver
            .poll(|event, body| {
                if !self.is_event_for_socket(&event) {
                    return Ok(None);
                }

                // Deal with Received before the buffer are recycled.
                if let VsockEventType::Received { length } = event.event_type {
                    // Only consider the connected socket and copy body to buffer
                    if let Some(connected) = self
                        .connected_sockets
                        .lock_irq_disabled()
                        .get(&event.into())
                    {
                        debug!("Rw matches a connection with id {:?}", connected.id());
                        if !connected.connection_buffer_add(body) {
                            return Err(SocketError::BufferTooShort);
                        }
                    } else {
                        return Ok(None);
                    }
                }
                Ok(Some(event))
            })
            .map_err(|e| {
                Error::with_message(Errno::EAGAIN, "driver poll failed, please try again")
            })?;

        let Some(event) = result else {
            return Ok(None);
        };

        // The socket must be stored in the VsockSpace.
        if let Some(connected) = self
            .connected_sockets
            .lock_irq_disabled()
            .get(&event.into())
        {
            connected.update_for_event(&event);
        }

        // Response to the event
        match event.event_type {
            VsockEventType::ConnectionRequest => {
                // Preparation for listen socket `accept`
                if let Some(listen) = self
                    .listen_sockets
                    .lock_irq_disabled()
                    .get(&event.destination.into())
                {
                    let peer = event.source;
                    let connected = Arc::new(Connected::new(peer.into(), listen.addr()));
                    connected.update_for_event(&event);
                    listen.push_incoming(connected).unwrap();
                } else {
                    return_errno_with_message!(
                        Errno::EINVAL,
                        "Connecion request can only be handled by listening socket"
                    )
                }
            }
            VsockEventType::Connected => {
                if let Some(connecting) = self
                    .connecting_sockets
                    .lock_irq_disabled()
                    .get(&event.destination.into())
                {
                    // debug!("match a connecting socket. Peer{:?}; local{:?}",connecting.peer_addr(),connecting.local_addr());
                    connecting.update_for_event(&event);
                    connecting.add_events(IoEvents::IN);
                }
            }
            VsockEventType::Disconnected { reason } => {
                if let Some(connected) = self
                    .connected_sockets
                    .lock_irq_disabled()
                    .get(&event.into())
                {
                    connected.peer_requested_shutdown();
                } else {
                    return_errno_with_message!(Errno::ENOTCONN, "The socket hasn't connected");
                }
            }
            VsockEventType::Received { length } => {
                if let Some(connected) = self
                    .connected_sockets
                    .lock_irq_disabled()
                    .get(&event.into())
                {
                    connected.add_events(IoEvents::IN);
                } else {
                    return_errno_with_message!(Errno::ENOTCONN, "The socket hasn't connected");
                }
            }
            VsockEventType::CreditRequest => {
                if let Some(connected) = self
                    .connected_sockets
                    .lock_irq_disabled()
                    .get(&event.into())
                {
                    driver.credit_update(&connected.get_info()).map_err(|_| {
                        Error::with_message(Errno::EINVAL, "can not send credit update")
                    })?;
                }
            }
            VsockEventType::CreditUpdate => {
                if let Some(connected) = self
                    .connected_sockets
                    .lock_irq_disabled()
                    .get(&event.into())
                {
                    connected.update_for_event(&event);
                } else {
                    return_errno_with_message!(
                        Errno::EINVAL,
                        "CreditUpdate is only valid in connected sockets"
                    )
                }
            }
        }
        Ok(Some(event))
    }
    /// Check whether the event is for this socket space
    fn is_event_for_socket(&self, event: &VsockEvent) -> bool {
        // debug!("The event is for connection with id {:?}",ConnectionID::from(*event));
        self.connecting_sockets
            .lock_irq_disabled()
            .contains_key(&event.destination.into())
            || self
                .listen_sockets
                .lock_irq_disabled()
                .contains_key(&event.destination.into())
            || self
                .connected_sockets
                .lock_irq_disabled()
                .contains_key(&(*event).into())
    }
    /// Alloc an unused port range
    pub fn alloc_ephemeral_port(&self) -> Result<u32> {
        let mut used_ports = self.used_ports.lock_irq_disabled();
        for port in 1024..=u32::MAX {
            if !used_ports.contains(&port) {
                used_ports.insert(port);
                return Ok(port);
            }
        }
        return_errno_with_message!(Errno::EAGAIN, "cannot find unused high port");
    }
}

impl Default for VsockSpace {
    fn default() -> Self {
        Self::new()
    }
}
