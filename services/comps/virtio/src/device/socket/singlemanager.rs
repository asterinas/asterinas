use core::hint::spin_loop;

use log::debug;

use super::{device::SocketDevice, connect::{ConnectionInfo, VsockEvent, VsockEventType}, header::VsockAddr, error::SocketError};



/// A higher level interface for VirtIO socket (vsock) devices.
///
/// This can only keep track of a single vsock connection. If you want to support multiple
/// simultaneous connections, try [`VsockConnectionManager`](super::VsockConnectionManager).
pub struct SingleConnectionManager {
    driver: SocketDevice,
    connection_info: Option<ConnectionInfo>,
}

impl SingleConnectionManager{
    /// Construct a new connection manager wrapping the given low-level VirtIO socket driver.
    pub fn new(driver: SocketDevice) -> Self {
        Self { 
            driver, 
            connection_info: None,
        }
    }

    /// Returns the CID which has been assigned to this guest.
    pub fn guest_cid(&self) -> u64 {
        self.driver.guest_cid()
    }

    /// Sends a request to connect to the given destination.
    ///
    /// This returns as soon as the request is sent; you should wait until `poll_recv` returns a
    /// `VsockEventType::Connected` event indicating that the peer has accepted the connection
    /// before sending data.
    pub fn connect(&mut self, destination: VsockAddr, src_port: u32) -> Result<(),SocketError> {
        if self.connection_info.is_some() {
            return Err(SocketError::ConnectionExists);
        }

        let new_connection_info = ConnectionInfo::new(destination, src_port);

        self.driver.connect(&new_connection_info)?;
        debug!("Connection requested: {:?}", new_connection_info);
        self.connection_info = Some(new_connection_info);
        Ok(())
    }

    /// Sends the buffer to the destination.
    pub fn send(&mut self, buffer: &[u8]) -> Result<(),SocketError> {
        let connection_info: &mut ConnectionInfo = self
            .connection_info
            .as_mut()
            .ok_or(SocketError::NotConnected)?;
        connection_info.buf_alloc = 0;
        self.driver.send(buffer, connection_info)
    }

    /// Polls the vsock device to receive data or other updates.
    ///
    /// A buffer must be provided to put the data in if there is some to
    /// receive.
    pub fn poll_recv(&mut self, buffer: &mut [u8]) -> Result<Option<VsockEvent>,SocketError> {
        let Some(connection_info) = &mut self.connection_info else {
            return Err(SocketError::NotConnected);
        };

        // Tell the peer that we have space to receive some data.
        connection_info.buf_alloc = buffer.len() as u32;
        self.driver.credit_update(connection_info)?;

        self.poll_rx_queue(buffer)
    }

    /// Blocks until we get some event from the vsock device.
    ///
    /// A buffer must be provided to put the data in if there is some to
    /// receive.
    pub fn wait_for_recv(&mut self, buffer: &mut [u8]) -> Result<VsockEvent,SocketError> {
        loop {
            if let Some(event) = self.poll_recv(buffer)? {
                return Ok(event);
            } else {
                spin_loop();
            }
        }
    }

    fn poll_rx_queue(&mut self, body: &mut [u8]) -> Result<Option<VsockEvent>,SocketError> {
        let guest_cid = self.driver.guest_cid();
        let self_connection_info = &mut self.connection_info;

        self.driver.poll(|event, borrowed_body| {
            let Some(connection_info) = self_connection_info else {
                return Ok(None);
            };

            // Skip packets which don't match our current connection.
            if !event.matches_connection(connection_info, guest_cid) {
                debug!(
                    "Skipping {:?} as connection is {:?}",
                    event, connection_info
                );
                return Ok(None);
            }

            // Update stored connection info.
            connection_info.update_for_event(&event);

            match event.event_type {
                VsockEventType::ConnectionRequest => {
                    // TODO: Send Rst or handle incoming connections.
                }
                VsockEventType::Connected => {}
                VsockEventType::Disconnected { .. } => {
                    *self_connection_info = None;
                }
                VsockEventType::Received { length } => {
                    body.get_mut(0..length)
                        .ok_or(SocketError::OutputBufferTooShort(length))?
                        .copy_from_slice(borrowed_body);
                    connection_info.done_forwarding(length);
                }
                VsockEventType::CreditRequest => {
                    // No point sending a credit update until `poll_recv` is called with a buffer,
                    // as otherwise buf_alloc would just be 0 anyway.
                }
                VsockEventType::CreditUpdate => {}
            }

            Ok(Some(event))
        })
    }

    /// Requests to shut down the connection cleanly.
    ///
    /// This returns as soon as the request is sent; you should wait until `poll_recv` returns a
    /// `VsockEventType::Disconnected` event if you want to know that the peer has acknowledged the
    /// shutdown.
    pub fn shutdown(&mut self) -> Result<(),SocketError> {
        let connection_info = self
            .connection_info
            .as_mut()
            .ok_or(SocketError::NotConnected)?;
        connection_info.buf_alloc = 0;

        self.driver.shutdown(connection_info)
    }

    /// Forcibly closes the connection without waiting for the peer.
    pub fn force_close(&mut self) -> Result<(),SocketError> {
        let connection_info = self
            .connection_info
            .as_mut()
            .ok_or(SocketError::NotConnected)?;
        connection_info.buf_alloc = 0;

        self.driver.force_close(connection_info)?;
        self.connection_info = None;
        Ok(())
    }

    /// Blocks until the peer either accepts our connection request (with a
    /// `VIRTIO_VSOCK_OP_RESPONSE`) or rejects it (with a
    /// `VIRTIO_VSOCK_OP_RST`).
    pub fn wait_for_connect(&mut self) -> Result<(),SocketError> {
        loop {
            match self.wait_for_recv(&mut [])?.event_type {
                VsockEventType::Connected => return Ok(()),
                VsockEventType::Disconnected { .. } => {
                    return Err(SocketError::ConnectionFailed)
                }
                VsockEventType::Received { .. } => return Err(SocketError::InvalidOperation),
                VsockEventType::ConnectionRequest
                | VsockEventType::CreditRequest
                | VsockEventType::CreditUpdate => {}
            }
        }
    }
}

#[cfg(test)]
mod tests{
    use super::*;

    #[test]
    fn send_recv(){
        let host_cid = 2;
        let guest_cid = 66;
        let host_port = 1234;
        let guest_port = 4321;
        let host_address = VsockAddr {
            cid: host_cid,
            port: host_port,
        };
        let hello_from_guest = "Hello from guest";
        let hello_from_host = "Hello from host";

        let mut config_space = VirtioVsockConfig {
            guest_cid_low: 66,
            guest_cid_high: 0,
        };  

        let mut socket = SingleConnectionManager::new{
            driver: SocketDevice::new{

            },
        };
    }
}