// SPDX-License-Identifier: MPL-2.0

use alloc::collections::BTreeSet;

use aster_virtio::device::socket::{
    connect::{ConnectionInfo, VsockEvent, VsockEventType},
    device::SocketDevice,
    error::SocketError,
    get_device, DEVICE_NAME,
};

use super::{
    addr::VsockSocketAddr,
    stream::{
        connected::{Connected, ConnectionID},
        connecting::Connecting,
        listen::Listen,
    },
};
use crate::{events::IoEvents, prelude::*, return_errno_with_message};

/// Manage all active sockets
pub struct VsockSpace {
    driver: Arc<SpinLock<SocketDevice>>,
    // (key, value) = (local_addr, connecting)
    connecting_sockets: SpinLock<BTreeMap<VsockSocketAddr, Arc<Connecting>>>,
    // (key, value) = (local_addr, listen)
    listen_sockets: SpinLock<BTreeMap<VsockSocketAddr, Arc<Listen>>>,
    // (key, value) = (id(local_addr,peer_addr), connected)
    connected_sockets: RwLock<BTreeMap<ConnectionID, Arc<Connected>>>,
    // Used ports
    used_ports: SpinLock<BTreeSet<u32>>,
}

impl VsockSpace {
    /// Create a new global VsockSpace
    pub fn new() -> Self {
        let driver = get_device(DEVICE_NAME).unwrap();
        Self {
            driver,
            connecting_sockets: SpinLock::new(BTreeMap::new()),
            listen_sockets: SpinLock::new(BTreeMap::new()),
            connected_sockets: RwLock::new(BTreeMap::new()),
            used_ports: SpinLock::new(BTreeSet::new()),
        }
    }
    /// Check whether the event is for this socket space
    fn is_event_for_socket(&self, event: &VsockEvent) -> bool {
        self.connecting_sockets
            .lock_irq_disabled()
            .contains_key(&event.destination.into())
            || self
                .listen_sockets
                .lock_irq_disabled()
                .contains_key(&event.destination.into())
            || self
                .connected_sockets
                .read_irq_disabled()
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
    pub fn insert_port(&self, port: u32) -> bool {
        let mut used_ports = self.used_ports.lock_irq_disabled();
        used_ports.insert(port)
    }
    pub fn recycle_port(&self, port: &u32) -> bool {
        let mut used_ports = self.used_ports.lock_irq_disabled();
        used_ports.remove(port)
    }

    pub fn insert_connected_socket(
        &self,
        id: ConnectionID,
        connected: Arc<Connected>,
    ) -> Option<Arc<Connected>> {
        let mut connected_sockets = self.connected_sockets.write_irq_disabled();
        connected_sockets.insert(id, connected)
    }
    pub fn remove_connected_socket(&self, id: &ConnectionID) -> Option<Arc<Connected>> {
        let mut connected_sockets = self.connected_sockets.write_irq_disabled();
        connected_sockets.remove(id)
    }
    pub fn insert_connecting_socket(
        &self,
        addr: VsockSocketAddr,
        connecting: Arc<Connecting>,
    ) -> Option<Arc<Connecting>> {
        let mut connecting_sockets = self.connecting_sockets.lock_irq_disabled();
        connecting_sockets.insert(addr, connecting)
    }
    pub fn remove_connecting_socket(&self, addr: &VsockSocketAddr) -> Option<Arc<Connecting>> {
        let mut connecting_sockets = self.connecting_sockets.lock_irq_disabled();
        connecting_sockets.remove(addr)
    }
    pub fn insert_listen_socket(
        &self,
        addr: VsockSocketAddr,
        listen: Arc<Listen>,
    ) -> Option<Arc<Listen>> {
        let mut listen_sockets = self.listen_sockets.lock_irq_disabled();
        listen_sockets.insert(addr, listen)
    }
    pub fn remove_listen_socket(&self, addr: &VsockSocketAddr) -> Option<Arc<Listen>> {
        let mut listen_sockets = self.listen_sockets.lock_irq_disabled();
        listen_sockets.remove(addr)
    }
}

impl VsockSpace {
    pub fn guest_cid(&self) -> u32 {
        let driver = self.driver.lock_irq_disabled();
        driver.guest_cid() as u32
    }

    pub fn request(&self, info: &ConnectionInfo) -> Result<()> {
        let mut driver = self.driver.lock_irq_disabled();
        driver
            .request(info)
            .map_err(|_| Error::with_message(Errno::EIO, "can not send connect packet"))
    }

    pub fn response(&self, info: &ConnectionInfo) -> Result<()> {
        let mut driver = self.driver.lock_irq_disabled();
        driver
            .response(info)
            .map_err(|_| Error::with_message(Errno::EIO, "can not send response packet"))
    }

    pub fn shutdown(&self, info: &ConnectionInfo) -> Result<()> {
        let mut driver = self.driver.lock_irq_disabled();
        driver
            .shutdown(info)
            .map_err(|_| Error::with_message(Errno::EIO, "can not send shutdown packet"))
    }

    pub fn reset(&self, info: &ConnectionInfo) -> Result<()> {
        let mut driver = self.driver.lock_irq_disabled();
        driver
            .reset(info)
            .map_err(|_| Error::with_message(Errno::EIO, "can not send reset packet"))
    }

    pub fn request_credit(&self, info: &ConnectionInfo) -> Result<()> {
        let mut driver = self.driver.lock_irq_disabled();
        driver
            .credit_request(info)
            .map_err(|_| Error::with_message(Errno::EIO, "can not send credit request packet"))
    }

    pub fn update_credit(&self, info: &ConnectionInfo) -> Result<()> {
        let mut driver = self.driver.lock_irq_disabled();
        driver
            .credit_update(info)
            .map_err(|_| Error::with_message(Errno::EIO, "can not send credit update packet"))
    }

    pub fn send(&self, buffer: &[u8], info: &mut ConnectionInfo) -> Result<()> {
        let mut driver = self.driver.lock_irq_disabled();
        driver
            .send(buffer, info)
            .map_err(|_| Error::with_message(Errno::EIO, "can not send data packet"))
    }

    /// Poll for each event from the driver
    pub fn poll(&self) -> Result<Option<VsockEvent>> {
        let mut driver = self.driver.lock_irq_disabled();
        let guest_cid = driver.guest_cid() as u32;
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
                        .read_irq_disabled()
                        .get(&event.into())
                    {
                        debug!("Rw matches a connection with id {:?}", connected.id());
                        if !connected.add_connection_buffer(body) {
                            return Err(SocketError::BufferTooShort);
                        }
                        connected.update_io_events();
                    } else {
                        return Ok(None);
                    }
                }
                Ok(Some(event))
            })
            .map_err(|e| Error::with_message(Errno::EIO, "driver poll failed, please try again"))?;

        let Some(event) = result else {
            return Ok(None);
        };
        debug!("vsock receive event: {:?}", event);
        // The socket must be stored in the VsockSpace.
        if let Some(connected) = self
            .connected_sockets
            .read_irq_disabled()
            .get(&event.into())
        {
            connected.update_info(&event);
        }

        // Response to the event
        match event.event_type {
            VsockEventType::ConnectionRequest => {
                // Preparation for listen socket `accept`
                let listen_sockets = self.listen_sockets.lock_irq_disabled();
                let Some(listen) = listen_sockets.get(&event.destination.into()) else {
                    return_errno_with_message!(
                        Errno::EINVAL,
                        "connecion request can only be handled by listening socket"
                    );
                };
                let peer = event.source;
                let connected = Arc::new(Connected::new(peer.into(), listen.addr()));
                connected.update_info(&event);
                listen.push_incoming(connected).unwrap();
                listen.update_io_events();
            }
            VsockEventType::Connected => {
                let connecting_sockets = self.connecting_sockets.lock_irq_disabled();
                let Some(connecting) = connecting_sockets.get(&event.destination.into()) else {
                    return_errno_with_message!(
                        Errno::EINVAL,
                        "connected event can only be handled by connecting socket"
                    );
                };
                debug!(
                    "match a connecting socket. Peer{:?}; local{:?}",
                    connecting.peer_addr(),
                    connecting.local_addr()
                );
                connecting.update_info(&event);
                connecting.add_events(IoEvents::IN);
            }
            VsockEventType::Disconnected { reason } => {
                let connected_sockets = self.connected_sockets.read_irq_disabled();
                let Some(connected) = connected_sockets.get(&event.into()) else {
                    return_errno_with_message!(Errno::ENOTCONN, "the socket hasn't connected");
                };
                connected.peer_requested_shutdown();
            }
            VsockEventType::Received { length } => {}
            VsockEventType::CreditRequest => {
                let connected_sockets = self.connected_sockets.read_irq_disabled();
                let Some(connected) = connected_sockets.get(&event.into()) else {
                    return_errno_with_message!(Errno::ENOTCONN, "the socket hasn't connected");
                };
                driver
                    .credit_update(&connected.get_info())
                    .map_err(|_| Error::with_message(Errno::EIO, "cannot send credit update"))?;
            }
            VsockEventType::CreditUpdate => {
                let connected_sockets = self.connected_sockets.read_irq_disabled();
                let Some(connected) = connected_sockets.get(&event.into()) else {
                    return_errno_with_message!(Errno::ENOTCONN, "the socket hasn't connected");
                };
                connected.update_info(&event);
            }
        }
        Ok(Some(event))
    }
}

impl Default for VsockSpace {
    fn default() -> Self {
        Self::new()
    }
}
