use core::{cmp::min, hint::spin_loop};

use alloc::{vec::Vec, boxed::Box, sync::Arc};
use aster_frame::sync::SpinLock;
use log::debug;

use crate::device::socket::error::SocketError;

use super::{device::SocketDevice, connect::{ConnectionInfo, VsockEvent, VsockEventType, DisconnectReason}, header::VsockAddr};


const PER_CONNECTION_BUFFER_CAPACITY: usize = 1024;

/// TODO: A higher level interface for VirtIO socket (vsock) devices.
///
/// This keeps track of multiple vsock connections.
///
/// # Example
///
/// ```
///
/// let mut socket = VsockConnectionManager::new(SocketDevice);
///
/// // Start a thread to call `socket.poll()` and handle events.
///
/// let remote_address = VsockAddr { cid: 2, port: 4321 };
/// let local_port = 1234;
/// socket.connect(remote_address, local_port)?;
///
/// // Wait until `socket.poll()` returns an event indicating that the socket is connected.
///
/// socket.send(remote_address, local_port, "Hello world".as_bytes())?;
///
/// socket.shutdown(remote_address, local_port)?;
/// # Ok(())
/// # }
/// ``
pub struct VsockConnectionManager {
    driver: Arc<SpinLock<SocketDevice>>,
    connections: Vec<Connection>,
    listening_ports: Vec<u32>,
}

impl VsockConnectionManager {
    /// Construct a new connection manager wrapping the given low-level VirtIO socket driver.
    pub fn new(driver: Arc<SpinLock<SocketDevice>>) -> Self {
        Self {
            driver,
            connections: Vec::new(),
            listening_ports: Vec::new(),
        }
    }

    /// Returns the CID which has been assigned to this guest.
    pub fn guest_cid(&self) -> u64 {
        self.driver.lock().guest_cid()
    }

    /// Allows incoming connections on the given port number.
    pub fn listen(&mut self, port: u32) {
        if !self.listening_ports.contains(&port) {
            self.listening_ports.push(port);
        }
    }

    /// Stops allowing incoming connections on the given port number.
    pub fn unlisten(&mut self, port: u32) {
        self.listening_ports.retain(|p| *p != port)
    }
    /// Sends a request to connect to the given destination.
    ///
    /// This returns as soon as the request is sent; you should wait until `poll` returns a
    /// `VsockEventType::Connected` event indicating that the peer has accepted the connection
    /// before sending data.
    pub fn connect(&mut self, destination: VsockAddr, src_port: u32) -> Result<(),SocketError> {
        if self.connections.iter().any(|connection| {
            connection.info.dst == destination && connection.info.src_port == src_port
        }) {
            return Err(SocketError::ConnectionExists.into());
        }

        let new_connection = Connection::new(destination, src_port);

        self.driver.lock().connect(&new_connection.info)?;
        debug!("Connection requested: {:?}", new_connection.info);
        self.connections.push(new_connection);
        Ok(())
    }

    /// Sends the buffer to the destination.
    pub fn send(&mut self, destination: VsockAddr, src_port: u32, buffer: &[u8]) -> Result<(),SocketError> {
        let (_, connection) = get_connection(&mut self.connections, destination, src_port)?;

        self.driver.lock().send(buffer, &mut connection.info)
    }

    /// Polls the vsock device to receive data or other updates.
    pub fn poll(&mut self) -> Result<Option<VsockEvent>,SocketError> {
        let guest_cid = self.driver.lock().guest_cid();
        let connections = &mut self.connections;

        let result = self.driver.lock().poll(|event, body| {
            let connection = get_connection_for_event(connections, &event, guest_cid);

            // Skip events which don't match any connection we know about, unless they are a
            // connection request.
            let connection = if let Some((_, connection)) = connection {
                connection
            } else if let VsockEventType::ConnectionRequest = event.event_type {
                // If the requested connection already exists or the CID isn't ours, ignore it.
                if connection.is_some() || event.destination.cid != guest_cid {
                    return Ok(None);
                }
                // Add the new connection to our list, at least for now. It will be removed again
                // below if we weren't listening on the port.
                connections.push(Connection::new(event.source, event.destination.port));
                connections.last_mut().unwrap()
            } else {
                return Ok(None);
            };

            // Update stored connection info.
            connection.info.update_for_event(&event);

            if let VsockEventType::Received { length } = event.event_type {
                // Copy to buffer
                if !connection.buffer.add(body) {
                    return Err(SocketError::OutputBufferTooShort(length));
                }
            }

            Ok(Some(event))
        })?;

        let Some(event) = result else {
            return Ok(None);
        };

        // The connection must exist because we found it above in the callback.
        let (connection_index, connection) =
            get_connection_for_event(connections, &event, guest_cid).unwrap();

        match event.event_type {
            VsockEventType::ConnectionRequest => {
                if self.listening_ports.contains(&event.destination.port) {
                    self.driver.lock().accept(&connection.info)?;
                } else {
                    // Reject the connection request and remove it from our list.
                    self.driver.lock().force_close(&connection.info)?;
                    self.connections.swap_remove(connection_index);

                    // No need to pass the request on to the client, as we've already rejected it.
                    return Ok(None);
                }
            }
            VsockEventType::Connected => {}
            VsockEventType::Disconnected { reason } => {
                // Wait until client reads all data before removing connection.
                if connection.buffer.is_empty() {
                    if reason == DisconnectReason::Shutdown {
                        self.driver.lock().force_close(&connection.info)?;
                    }
                    self.connections.swap_remove(connection_index);
                } else {
                    connection.peer_requested_shutdown = true;
                }
            }
            VsockEventType::Received { .. } => {
                // Already copied the buffer in the callback above.
            }
            VsockEventType::CreditRequest => {
                // If the peer requested credit, send an update.
                self.driver.lock().credit_update(&connection.info)?;
                // No need to pass the request on to the client, we've already handled it.
                return Ok(None);
            }
            VsockEventType::CreditUpdate => {}
        }

        Ok(Some(event))
    }

    /// Reads data received from the given connection.
    pub fn recv(&mut self, peer: VsockAddr, src_port: u32, buffer: &mut [u8]) -> Result<usize,SocketError> {
        debug!("connections is {:?}",self.connections);
        let (connection_index, connection) = get_connection(&mut self.connections, peer, src_port)?;

        // Copy from ring buffer
        let bytes_read = connection.buffer.drain(buffer);

        connection.info.done_forwarding(bytes_read);

        // If buffer is now empty and the peer requested shutdown, finish shutting down the
        // connection.
        if connection.peer_requested_shutdown && connection.buffer.is_empty() {
            self.driver.lock().force_close(&connection.info)?;
            self.connections.swap_remove(connection_index);
        }

        Ok(bytes_read) 
    }

    /// Blocks until we get some event from the vsock device.
    pub fn wait_for_event(&mut self) -> Result<VsockEvent,SocketError> {
        loop {
            if let Some(event) = self.poll()? {
                return Ok(event);
            } else {
                spin_loop();
            }
        }
    }

    /// Requests to shut down the connection cleanly.
    ///
    /// This returns as soon as the request is sent; you should wait until `poll` returns a
    /// `VsockEventType::Disconnected` event if you want to know that the peer has acknowledged the
    /// shutdown.
    pub fn shutdown(&mut self, destination: VsockAddr, src_port: u32) -> Result<(),SocketError> {
        let (_, connection) = get_connection(&mut self.connections, destination, src_port)?;

        self.driver.lock().shutdown(&connection.info)
    }

    /// Forcibly closes the connection without waiting for the peer.
    pub fn force_close(&mut self, destination: VsockAddr, src_port: u32) -> Result<(),SocketError> {
        let (index, connection) = get_connection(&mut self.connections, destination, src_port)?;

        self.driver.lock().force_close(&connection.info)?;

        self.connections.swap_remove(index);
        Ok(())
    }
}

/// Returns the connection from the given list matching the given peer address and local port, and
/// its index.
///
/// Returns `Err(SocketError::NotConnected)` if there is no matching connection in the list.
fn get_connection(
    connections: &mut [Connection],
    peer: VsockAddr,
    local_port: u32,
) -> core::result::Result<(usize, &mut Connection), SocketError> {
    connections
        .iter_mut()
        .enumerate()
        .find(|(_, connection)| {
            connection.info.dst == peer && connection.info.src_port == local_port
        })
        .ok_or(SocketError::NotConnected)
}

/// Returns the connection from the given list matching the event, if any, and its index.
fn get_connection_for_event<'a>(
    connections: &'a mut [Connection],
    event: &VsockEvent,
    local_cid: u64,
) -> Option<(usize, &'a mut Connection)> {
    connections
        .iter_mut()
        .enumerate()
        .find(|(_, connection)| event.matches_connection(&connection.info, local_cid))
}


#[derive(Debug)]
struct Connection {
    info: ConnectionInfo,
    buffer: RingBuffer,
    /// The peer sent a SHUTDOWN request, but we haven't yet responded with a RST because there is
    /// still data in the buffer.
    peer_requested_shutdown: bool,
}

impl Connection {
    fn new(peer: VsockAddr, local_port: u32) -> Self {
        let mut info = ConnectionInfo::new(peer, local_port);
        info.buf_alloc = PER_CONNECTION_BUFFER_CAPACITY.try_into().unwrap();
        Self {
            info,
            buffer: RingBuffer::new(PER_CONNECTION_BUFFER_CAPACITY),
            peer_requested_shutdown: false,
        }
    }
}

#[derive(Debug)]
struct RingBuffer {
    buffer: Box<[u8]>,
    /// The number of bytes currently in the buffer.
    used: usize,
    /// The index of the first used byte in the buffer.
    start: usize,
}

impl RingBuffer {
    pub fn new(capacity: usize) -> Self {
        // TODO: can be optimized.
        let mut temp = Vec::with_capacity(capacity);
        temp.resize(capacity,0);
        Self {
            // FIXME: if the capacity is excessive, elements move will be executed.
            buffer: temp.into_boxed_slice(),
            used: 0,
            start: 0,
        }
    }
    /// Returns the number of bytes currently used in the buffer.
    pub fn used(&self) -> usize {
        self.used
    }

    /// Returns true iff there are currently no bytes in the buffer.
    pub fn is_empty(&self) -> bool {
        self.used == 0
    }

    /// Returns the number of bytes currently free in the buffer.
    pub fn available(&self) -> usize {
        self.buffer.len() - self.used
    }

    /// Adds the given bytes to the buffer if there is enough capacity for them all.
    ///
    /// Returns true if they were added, or false if they were not.
    pub fn add(&mut self, bytes: &[u8]) -> bool {
        if bytes.len() > self.available() {
            return false;
        }

        // The index of the first available position in the buffer.
        let first_available = (self.start + self.used) % self.buffer.len();
        // The number of bytes to copy from `bytes` to `buffer` between `first_available` and
        // `buffer.len()`.
        let copy_length_before_wraparound = min(bytes.len(), self.buffer.len() - first_available);
        self.buffer[first_available..first_available + copy_length_before_wraparound]
            .copy_from_slice(&bytes[0..copy_length_before_wraparound]);
        if let Some(bytes_after_wraparound) = bytes.get(copy_length_before_wraparound..) {
            self.buffer[0..bytes_after_wraparound.len()].copy_from_slice(bytes_after_wraparound);
        }
        self.used += bytes.len();

        true
    }

    /// Reads and removes as many bytes as possible from the buffer, up to the length of the given
    /// buffer.
    pub fn drain(&mut self, out: &mut [u8]) -> usize {
        let bytes_read = min(self.used, out.len());

        // The number of bytes to copy out between `start` and the end of the buffer.
        let read_before_wraparound = min(bytes_read, self.buffer.len() - self.start);
        // The number of bytes to copy out from the beginning of the buffer after wrapping around.
        let read_after_wraparound = bytes_read
            .checked_sub(read_before_wraparound)
            .unwrap_or_default();

        out[0..read_before_wraparound]
            .copy_from_slice(&self.buffer[self.start..self.start + read_before_wraparound]);
        out[read_before_wraparound..bytes_read]
            .copy_from_slice(&self.buffer[0..read_after_wraparound]);

        self.used -= bytes_read;
        self.start = (self.start + bytes_read) % self.buffer.len();

        bytes_read
    }
}