// SPDX-License-Identifier: MPL-2.0

use alloc::collections::BTreeSet;

use aster_virtio::device::socket::{
    connect::{ConnectionInfo, VsockEvent, VsockEventType},
    device::SocketDevice,
    error::SocketError,
};
use ostd::sync::LocalIrqDisabled;

use super::{
    addr::VsockSocketAddr,
    stream::{
        connected::{Connected, ConnectionID},
        connecting::Connecting,
        listen::Listen,
    },
};
use crate::{prelude::*, return_errno_with_message, util::MultiRead};

/// Manage all active sockets
pub struct VsockSpace {
    driver: Arc<SpinLock<SocketDevice>>,
    // (key, value) = (local_addr, connecting)
    connecting_sockets: SpinLock<BTreeMap<VsockSocketAddr, Arc<Connecting>>>,
    // (key, value) = (local_addr, listen)
    listen_sockets: SpinLock<BTreeMap<VsockSocketAddr, Arc<Listen>>>,
    // (key, value) = (id(local_addr,peer_addr), connected)
    connected_sockets: RwLock<BTreeMap<ConnectionID, Arc<Connected>>, LocalIrqDisabled>,
    // Used ports
    used_ports: SpinLock<BTreeSet<u32>>,
}

impl VsockSpace {
    /// Create a new global VsockSpace
    pub fn new(driver: Arc<SpinLock<SocketDevice>>) -> Self {
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
            .disable_irq()
            .lock()
            .contains_key(&event.destination.into())
            || self
                .listen_sockets
                .disable_irq()
                .lock()
                .contains_key(&event.destination.into())
            || self.connected_sockets.read().contains_key(&(*event).into())
    }

    /// Alloc an unused port range
    pub fn alloc_ephemeral_port(&self) -> Result<u32> {
        let mut used_ports = self.used_ports.disable_irq().lock();
        // FIXME: the maximal port number is not defined by spec
        for port in 1024..u32::MAX {
            if !used_ports.contains(&port) {
                used_ports.insert(port);
                return Ok(port);
            }
        }
        return_errno_with_message!(Errno::EAGAIN, "cannot find unused high port");
    }

    /// Bind a port
    pub fn bind_port(&self, port: u32) -> bool {
        let mut used_ports = self.used_ports.disable_irq().lock();
        used_ports.insert(port)
    }

    /// Recycle a port
    pub fn recycle_port(&self, port: &u32) -> bool {
        let mut used_ports = self.used_ports.disable_irq().lock();
        used_ports.remove(port)
    }

    /// Insert a connected socket
    pub fn insert_connected_socket(
        &self,
        id: ConnectionID,
        connected: Arc<Connected>,
    ) -> Option<Arc<Connected>> {
        let mut connected_sockets = self.connected_sockets.write();
        connected_sockets.insert(id, connected)
    }

    /// Remove a connected socket
    pub fn remove_connected_socket(&self, id: &ConnectionID) -> Option<Arc<Connected>> {
        let mut connected_sockets = self.connected_sockets.write();
        connected_sockets.remove(id)
    }

    /// Insert a connecting socket
    pub fn insert_connecting_socket(
        &self,
        addr: VsockSocketAddr,
        connecting: Arc<Connecting>,
    ) -> Option<Arc<Connecting>> {
        let mut connecting_sockets = self.connecting_sockets.disable_irq().lock();
        connecting_sockets.insert(addr, connecting)
    }

    /// Remove a connecting socket
    pub fn remove_connecting_socket(&self, addr: &VsockSocketAddr) -> Option<Arc<Connecting>> {
        let mut connecting_sockets = self.connecting_sockets.disable_irq().lock();
        connecting_sockets.remove(addr)
    }

    /// Insert a listening socket
    pub fn insert_listen_socket(
        &self,
        addr: VsockSocketAddr,
        listen: Arc<Listen>,
    ) -> Option<Arc<Listen>> {
        let mut listen_sockets = self.listen_sockets.disable_irq().lock();
        listen_sockets.insert(addr, listen)
    }

    /// Remove a listening socket
    pub fn remove_listen_socket(&self, addr: &VsockSocketAddr) -> Option<Arc<Listen>> {
        let mut listen_sockets = self.listen_sockets.disable_irq().lock();
        listen_sockets.remove(addr)
    }
}

impl VsockSpace {
    /// Get the CID of the guest
    pub fn guest_cid(&self) -> u32 {
        let driver = self.driver.disable_irq().lock();
        driver.guest_cid() as u32
    }

    /// Send a request packet for initializing a new connection.
    pub fn request(&self, info: &ConnectionInfo) -> Result<()> {
        let mut driver = self.driver.disable_irq().lock();
        driver
            .request(info)
            .map_err(|_| Error::with_message(Errno::EIO, "cannot send connect packet"))
    }

    /// Send a response packet for accepting a new connection.
    pub fn response(&self, info: &ConnectionInfo) -> Result<()> {
        let mut driver = self.driver.disable_irq().lock();
        driver
            .response(info)
            .map_err(|_| Error::with_message(Errno::EIO, "cannot send response packet"))
    }

    /// Send a shutdown packet to close a connection
    pub fn shutdown(&self, info: &ConnectionInfo) -> Result<()> {
        let mut driver = self.driver.disable_irq().lock();
        driver
            .shutdown(info)
            .map_err(|_| Error::with_message(Errno::EIO, "cannot send shutdown packet"))
    }

    /// Send a reset packet to reset a connection
    pub fn reset(&self, info: &ConnectionInfo) -> Result<()> {
        let mut driver = self.driver.disable_irq().lock();
        driver
            .reset(info)
            .map_err(|_| Error::with_message(Errno::EIO, "cannot send reset packet"))
    }

    /// Send a credit request packet
    pub fn request_credit(&self, info: &ConnectionInfo) -> Result<()> {
        let mut driver = self.driver.disable_irq().lock();
        driver
            .credit_request(info)
            .map_err(|_| Error::with_message(Errno::EIO, "cannot send credit request packet"))
    }

    /// Send a credit update packet
    pub fn update_credit(&self, info: &ConnectionInfo) -> Result<()> {
        let mut driver = self.driver.disable_irq().lock();
        driver
            .credit_update(info)
            .map_err(|_| Error::with_message(Errno::EIO, "cannot send credit update packet"))
    }

    /// Send a data packet
    pub fn send(&self, reader: &mut dyn MultiRead, info: &mut ConnectionInfo) -> Result<()> {
        // FIXME: Creating this buffer should be avoided
        // if the underlying driver can accept reader.
        let mut buffer = vec![0u8; reader.sum_lens()];
        reader.read(&mut VmWriter::from(buffer.as_mut_slice()))?;

        let mut driver = self.driver.disable_irq().lock();
        driver
            .send(&buffer, info)
            .map_err(|_| Error::with_message(Errno::EIO, "cannot send data packet"))
    }

    /// Poll for each event from the driver
    pub fn poll(&self) -> Result<()> {
        let mut driver = self.driver.disable_irq().lock();

        while let Some(event) = self.poll_single(&mut driver)? {
            if !self.is_event_for_socket(&event) {
                debug!("ignore event {:?}", event);
                continue;
            }

            debug!("vsock receive event: {:?}", event);
            // The socket must be stored in the VsockSpace.
            if let Some(connected) = self.connected_sockets.read().get(&event.into()) {
                connected.update_info(&event);
            }

            // Response to the event
            match event.event_type {
                VsockEventType::ConnectionRequest => {
                    // Preparation for listen socket `accept`
                    let listen_sockets = self.listen_sockets.disable_irq().lock();
                    let Some(listen) = listen_sockets.get(&event.destination.into()) else {
                        return_errno_with_message!(
                            Errno::EINVAL,
                            "connection request can only be handled by listening socket"
                        );
                    };
                    let peer = event.source;
                    let connected = Arc::new(Connected::new(peer.into(), listen.addr()));
                    connected.update_info(&event);
                    listen.push_incoming(connected).unwrap();
                }
                VsockEventType::ConnectionResponse => {
                    let connecting_sockets = self.connecting_sockets.disable_irq().lock();
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
                    connecting.set_connected();
                }
                VsockEventType::Disconnected { .. } => {
                    let connected_sockets = self.connected_sockets.read();
                    let Some(connected) = connected_sockets.get(&event.into()) else {
                        return_errno_with_message!(Errno::ENOTCONN, "the socket hasn't connected");
                    };
                    connected.set_peer_requested_shutdown();
                }
                VsockEventType::Received { .. } => {}
                VsockEventType::CreditRequest => {
                    let connected_sockets = self.connected_sockets.read();
                    let Some(connected) = connected_sockets.get(&event.into()) else {
                        return_errno_with_message!(Errno::ENOTCONN, "the socket hasn't connected");
                    };
                    driver.credit_update(&connected.get_info()).map_err(|_| {
                        Error::with_message(Errno::EIO, "cannot send credit update")
                    })?;
                }
                VsockEventType::CreditUpdate => {
                    let connected_sockets = self.connected_sockets.read();
                    let Some(connected) = connected_sockets.get(&event.into()) else {
                        return_errno_with_message!(Errno::ENOTCONN, "the socket hasn't connected");
                    };
                    connected.update_info(&event);
                }
            }
        }
        Ok(())
    }

    fn poll_single(&self, driver: &mut SocketDevice) -> Result<Option<VsockEvent>> {
        driver
            .poll(|event, body| {
                // Deal with Received before the buffer are recycled.
                if let VsockEventType::Received { .. } = event.event_type {
                    // Only consider the connected socket and copy body to buffer
                    let connected_sockets = self.connected_sockets.read();
                    let connected = connected_sockets.get(&event.into()).unwrap();
                    debug!("Rw matches a connection with id {:?}", connected.id());
                    if !connected.add_connection_buffer(body) {
                        return Err(SocketError::BufferTooShort);
                    }
                }
                Ok(Some(event))
            })
            .map_err(|_| Error::with_message(Errno::EIO, "driver poll failed"))
    }
}
