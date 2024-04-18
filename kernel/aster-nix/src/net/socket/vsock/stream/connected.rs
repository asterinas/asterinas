// SPDX-License-Identifier: MPL-2.0

use alloc::boxed::Box;
use core::cmp::min;

use aster_virtio::device::socket::connect::{ConnectionInfo, VsockEvent};

use super::connecting::Connecting;
use crate::{
    events::IoEvents,
    net::socket::{
        vsock::{addr::VsockSocketAddr, VSOCK_GLOBAL},
        SendRecvFlags, SockShutdownCmd,
    },
    prelude::*,
    process::signal::{Pollee, Poller},
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
            pollee: Pollee::new(IoEvents::empty()),
        }
    }

    pub fn from_connecting(connecting: Arc<Connecting>) -> Self {
        Self {
            connection: SpinLock::new(Connection::from_info(connecting.info())),
            id: connecting.id(),
            pollee: Pollee::new(IoEvents::empty()),
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

    pub fn recv(&self, buf: &mut [u8]) -> Result<usize> {
        let poller = Poller::new();
        if !self
            .poll(IoEvents::IN, Some(&poller))
            .contains(IoEvents::IN)
        {
            poller.wait()?;
        }

        let mut connection = self.connection.lock_irq_disabled();
        let bytes_read = connection.buffer.drain(buf);

        connection.info.done_forwarding(bytes_read);

        Ok(bytes_read)
    }

    pub fn send(&self, buf: &[u8], flags: SendRecvFlags) -> Result<usize> {
        let mut connection = self.connection.lock_irq_disabled();
        debug_assert!(flags.is_all_supported());
        let buf_len = buf.len();
        VSOCK_GLOBAL
            .get()
            .unwrap()
            .driver
            .lock_irq_disabled()
            .send(buf, &mut connection.info)
            .map_err(|e| Error::with_message(Errno::ENOBUFS, "cannot send packet"))?;
        Ok(buf_len)
    }

    pub fn should_close(&self) -> bool {
        let connection = self.connection.lock_irq_disabled();
        // If buffer is now empty and the peer requested shutdown, finish shutting down the
        // connection.
        connection.peer_requested_shutdown && connection.buffer.is_empty()
    }

    pub fn shutdown(&self, cmd: SockShutdownCmd) -> Result<()> {
        let connection = self.connection.lock_irq_disabled();
        // TODO: deal with cmd
        if self.should_close() {
            let vsockspace = VSOCK_GLOBAL.get().unwrap();
            vsockspace
                .driver
                .lock_irq_disabled()
                .reset(&connection.info)
                .map_err(|e| Error::with_message(Errno::ENOMEM, "can not send close packet"))?;
            vsockspace
                .connected_sockets
                .lock_irq_disabled()
                .remove(&self.id())
                .unwrap();
        }
        Ok(())
    }
    pub fn update_for_event(&self, event: &VsockEvent) {
        let mut connection = self.connection.lock_irq_disabled();
        connection.update_for_event(event)
    }

    pub fn get_info(&self) -> ConnectionInfo {
        let connection = self.connection.lock_irq_disabled();
        connection.info.clone()
    }

    pub fn connection_buffer_add(&self, bytes: &[u8]) -> bool {
        let mut connection = self.connection.lock_irq_disabled();
        self.add_events(IoEvents::IN);
        connection.add(bytes)
    }

    pub fn peer_requested_shutdown(&self) {
        self.connection.lock_irq_disabled().peer_requested_shutdown = true
    }

    pub fn poll(&self, mask: IoEvents, poller: Option<&Poller>) -> IoEvents {
        self.pollee.poll(mask, poller)
    }
    pub fn add_events(&self, events: IoEvents) {
        self.pollee.add_events(events)
    }
}

impl Drop for Connected {
    fn drop(&mut self) {
        let vsockspace = VSOCK_GLOBAL.get().unwrap();
        vsockspace
            .used_ports
            .lock_irq_disabled()
            .remove(&self.local_addr().port);
    }
}

#[derive(Debug)]
pub struct Connection {
    info: ConnectionInfo,
    buffer: RingBuffer,
    /// The peer sent a SHUTDOWN request, but we haven't yet responded with a RST because there is
    /// still data in the buffer.
    pub peer_requested_shutdown: bool,
}

impl Connection {
    pub fn new(peer: VsockSocketAddr, local_port: u32) -> Self {
        let mut info = ConnectionInfo::new(peer.into(), local_port);
        info.buf_alloc = PER_CONNECTION_BUFFER_CAPACITY.try_into().unwrap();
        Self {
            info,
            buffer: RingBuffer::new(PER_CONNECTION_BUFFER_CAPACITY),
            peer_requested_shutdown: false,
        }
    }
    pub fn from_info(info: ConnectionInfo) -> Self {
        let mut info = info.clone();
        info.buf_alloc = PER_CONNECTION_BUFFER_CAPACITY.try_into().unwrap();
        Self {
            info,
            buffer: RingBuffer::new(PER_CONNECTION_BUFFER_CAPACITY),
            peer_requested_shutdown: false,
        }
    }
    pub fn update_for_event(&mut self, event: &VsockEvent) {
        self.info.update_for_event(event)
    }
    pub fn add(&mut self, bytes: &[u8]) -> bool {
        self.buffer.add(bytes)
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

#[derive(Debug)]
struct RingBuffer {
    buffer: Box<[u8]>,
    /// The number of bytes currently in the buffer.
    used: usize,
    /// The index of the first used byte in the buffer.
    start: usize,
}
//TODO: ringbuf
impl RingBuffer {
    pub fn new(capacity: usize) -> Self {
        // TODO: can be optimized.
        let temp = vec![0; capacity];
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
