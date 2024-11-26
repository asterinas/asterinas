// SPDX-License-Identifier: MPL-2.0

use aster_virtio::device::socket::connect::{ConnectionInfo, VsockEvent};

use super::connecting::Connecting;
use crate::{
    events::IoEvents,
    net::socket::{
        vsock::{addr::VsockSocketAddr, VSOCK_GLOBAL},
        SendRecvFlags, SockShutdownCmd,
    },
    prelude::*,
    process::signal::{PollHandle, Pollee},
    util::{ring_buffer::RingBuffer, MultiRead, MultiWrite},
};

const PER_CONNECTION_BUFFER_CAPACITY: usize = 4096;

pub struct Connected {
    connection: SpinLock<Connection>,
    id: ConnectionID,
    pollee: Pollee,
}

impl Connected {
    pub fn new(peer_addr: VsockSocketAddr, local_addr: VsockSocketAddr) -> Self {
        Self {
            connection: SpinLock::new(Connection::new(peer_addr, local_addr.port)),
            id: ConnectionID::new(local_addr, peer_addr),
            // FIXME: We should reuse `Pollee` from `Init`.
            pollee: Pollee::new(),
        }
    }

    pub fn from_connecting(connecting: Arc<Connecting>) -> Self {
        Self {
            connection: SpinLock::new(Connection::new_from_info(connecting.info())),
            id: connecting.id(),
            // FIXME: We should reuse `Pollee` from `Init`.
            pollee: Pollee::new(),
        }
    }
    pub fn peer_addr(&self) -> VsockSocketAddr {
        self.id.peer_addr
    }

    pub fn local_addr(&self) -> VsockSocketAddr {
        self.id.local_addr
    }

    pub fn id(&self) -> ConnectionID {
        self.id
    }

    pub fn try_recv(&self, writer: &mut dyn MultiWrite) -> Result<usize> {
        let mut connection = self.connection.disable_irq().lock();
        let bytes_read = connection.buffer.read_fallible(writer)?;
        connection.info.done_forwarding(bytes_read);
        self.pollee.invalidate();

        match bytes_read {
            0 => {
                if !connection.is_peer_requested_shutdown() {
                    return_errno_with_message!(Errno::EAGAIN, "the receive buffer is empty");
                } else {
                    return_errno_with_message!(Errno::ECONNRESET, "the connection is reset");
                }
            }
            bytes_read => Ok(bytes_read),
        }
    }

    pub fn send(&self, reader: &mut dyn MultiRead, flags: SendRecvFlags) -> Result<usize> {
        let mut connection = self.connection.disable_irq().lock();
        debug_assert!(flags.is_all_supported());
        let buf_len = reader.sum_lens();
        VSOCK_GLOBAL
            .get()
            .unwrap()
            .send(reader, &mut connection.info)?;

        Ok(buf_len)
    }

    pub fn should_close(&self) -> bool {
        let connection = self.connection.disable_irq().lock();
        // If buffer is now empty and the peer requested shutdown, finish shutting down the
        // connection.
        connection.is_peer_requested_shutdown() && connection.buffer.is_empty()
    }

    pub fn is_closed(&self) -> bool {
        let connection = self.connection.disable_irq().lock();
        connection.is_local_shutdown()
    }

    pub fn shutdown(&self, _cmd: SockShutdownCmd) -> Result<()> {
        // TODO: deal with cmd
        if self.should_close() {
            let mut connection = self.connection.disable_irq().lock();
            if connection.is_local_shutdown() {
                return Ok(());
            }
            let vsockspace = VSOCK_GLOBAL.get().unwrap();
            vsockspace.reset(&connection.info).unwrap();
            connection.set_local_shutdown();
        }
        Ok(())
    }
    pub fn update_info(&self, event: &VsockEvent) {
        let mut connection = self.connection.disable_irq().lock();
        connection.update_for_event(event)
    }

    pub fn get_info(&self) -> ConnectionInfo {
        let connection = self.connection.disable_irq().lock();
        connection.info.clone()
    }

    pub fn add_connection_buffer(&self, bytes: &[u8]) -> bool {
        let mut connection = self.connection.disable_irq().lock();

        let result = connection.add(bytes);
        self.pollee.notify(IoEvents::IN);

        result
    }

    pub fn set_peer_requested_shutdown(&self) {
        self.connection
            .disable_irq()
            .lock()
            .set_peer_requested_shutdown()
    }

    pub fn poll(&self, mask: IoEvents, poller: Option<&mut PollHandle>) -> IoEvents {
        self.pollee
            .poll_with(mask, poller, || self.check_io_events())
    }

    fn check_io_events(&self) -> IoEvents {
        let connection = self.connection.disable_irq().lock();

        // receive
        if !connection.buffer.is_empty() {
            IoEvents::IN
        } else {
            IoEvents::empty()
        }
    }
}

struct Connection {
    info: ConnectionInfo,
    buffer: RingBuffer<u8>,
    /// The peer sent a SHUTDOWN request, but we haven't yet responded with a RST because there is
    /// still data in the buffer.
    peer_requested_shutdown: bool,
    local_shutdown: bool,
}

impl Connection {
    fn new(peer: VsockSocketAddr, local_port: u32) -> Self {
        let mut info = ConnectionInfo::new(peer.into(), local_port);
        info.buf_alloc = PER_CONNECTION_BUFFER_CAPACITY.try_into().unwrap();
        Self {
            info,
            buffer: RingBuffer::new(PER_CONNECTION_BUFFER_CAPACITY),
            peer_requested_shutdown: false,
            local_shutdown: false,
        }
    }

    fn is_peer_requested_shutdown(&self) -> bool {
        self.peer_requested_shutdown
    }

    fn set_peer_requested_shutdown(&mut self) {
        self.peer_requested_shutdown = true
    }

    fn is_local_shutdown(&self) -> bool {
        self.local_shutdown
    }

    fn set_local_shutdown(&mut self) {
        self.local_shutdown = true
    }

    fn new_from_info(mut info: ConnectionInfo) -> Self {
        info.buf_alloc = PER_CONNECTION_BUFFER_CAPACITY.try_into().unwrap();
        Self {
            info,
            buffer: RingBuffer::new(PER_CONNECTION_BUFFER_CAPACITY),
            peer_requested_shutdown: false,
            local_shutdown: false,
        }
    }

    fn update_for_event(&mut self, event: &VsockEvent) {
        self.info.update_for_event(event)
    }

    fn add(&mut self, bytes: &[u8]) -> bool {
        if bytes.len() > self.buffer.capacity() - self.buffer.len() {
            return false;
        }
        self.buffer.push_slice(bytes).unwrap();
        true
    }
}

#[derive(Debug, Copy, Clone, Ord, PartialOrd, Eq, PartialEq)]
pub struct ConnectionID {
    pub local_addr: VsockSocketAddr,
    pub peer_addr: VsockSocketAddr,
}
impl ConnectionID {
    pub fn new(local_addr: VsockSocketAddr, peer_addr: VsockSocketAddr) -> Self {
        Self {
            local_addr,
            peer_addr,
        }
    }
}

impl From<VsockEvent> for ConnectionID {
    fn from(event: VsockEvent) -> Self {
        Self::new(event.destination.into(), event.source.into())
    }
}
