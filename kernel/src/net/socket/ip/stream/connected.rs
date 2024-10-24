// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Weak;
use core::sync::atomic::{AtomicBool, Ordering};

use aster_bigtcp::{
    errors::tcp::{RecvError, SendError},
    socket::{RawTcpSocket, SocketEventObserver, TcpState},
    wire::IpEndpoint,
};

use crate::{
    events::IoEvents,
    net::{
        iface::BoundTcpSocket,
        socket::util::{send_recv_flags::SendRecvFlags, shutdown_cmd::SockShutdownCmd},
    },
    prelude::*,
    process::signal::Pollee,
    util::{MultiRead, MultiWrite},
};

pub struct ConnectedStream {
    bound_socket: BoundTcpSocket,
    remote_endpoint: IpEndpoint,
    /// Indicates whether this connection is "new" in a `connect()` system call.
    ///
    /// If the connection is not new, `connect()` will fail with the error code `EISCONN`,
    /// otherwise it will succeed. This means that `connect()` will succeed _exactly_ once,
    /// regardless of whether the connection is established synchronously or asynchronously.
    ///
    /// If the connection is established synchronously, the synchronous `connect()` will succeed
    /// and any subsequent `connect()` will fail; otherwise, the first `connect()` after the
    /// connection is established asynchronously will succeed and any subsequent `connect()` will
    /// fail.
    is_new_connection: bool,
    /// Indicates if the receiving side of this socket is closed.
    ///
    /// The receiving side may be closed if this side disables reading
    /// or if the peer side closes its sending half.
    is_receiving_closed: AtomicBool,
    /// Indicates if the sending side of this socket is closed.
    ///
    /// The sending side can only be closed if this side disables writing.
    is_sending_closed: AtomicBool,
}

impl ConnectedStream {
    pub fn new(
        bound_socket: BoundTcpSocket,
        remote_endpoint: IpEndpoint,
        is_new_connection: bool,
    ) -> Self {
        Self {
            bound_socket,
            remote_endpoint,
            is_new_connection,
            is_receiving_closed: AtomicBool::new(false),
            is_sending_closed: AtomicBool::new(false),
        }
    }

    pub fn shutdown(&self, cmd: SockShutdownCmd, pollee: &Pollee) -> Result<()> {
        if cmd.shut_read() {
            self.is_receiving_closed.store(true, Ordering::Relaxed);
            self.update_io_events(pollee);
        }

        if cmd.shut_write() {
            self.is_sending_closed.store(true, Ordering::Relaxed);
            self.bound_socket.close();
        }

        Ok(())
    }

    pub fn try_recv(&self, writer: &mut dyn MultiWrite, _flags: SendRecvFlags) -> Result<usize> {
        let result = self.bound_socket.recv(|socket_buffer| {
            match writer.write(&mut VmReader::from(&*socket_buffer)) {
                Ok(len) => (len, Ok(len)),
                Err(e) => (0, Err(e)),
            }
        });

        match result {
            Ok(Ok(0)) if self.is_receiving_closed.load(Ordering::Relaxed) => Ok(0),
            Ok(Ok(0)) => return_errno_with_message!(Errno::EAGAIN, "the receive buffer is empty"),
            Ok(Ok(recv_bytes)) => Ok(recv_bytes),
            Ok(Err(e)) => Err(e),
            Err(RecvError::Finished) => Ok(0),
            Err(RecvError::InvalidState) => {
                if self.before_established() {
                    return_errno_with_message!(Errno::EAGAIN, "the connection is not established");
                }
                return_errno_with_message!(Errno::ECONNRESET, "the connection is reset")
            }
        }
    }

    pub fn try_send(&self, reader: &mut dyn MultiRead, _flags: SendRecvFlags) -> Result<usize> {
        let result = self.bound_socket.send(|socket_buffer| {
            match reader.read(&mut VmWriter::from(socket_buffer)) {
                Ok(len) => (len, Ok(len)),
                Err(e) => (0, Err(e)),
            }
        });

        match result {
            Ok(Ok(0)) => return_errno_with_message!(Errno::EAGAIN, "the send buffer is full"),
            Ok(Ok(sent_bytes)) => Ok(sent_bytes),
            Ok(Err(e)) => Err(e),
            Err(SendError::InvalidState) => {
                if self.before_established() {
                    return_errno_with_message!(Errno::EAGAIN, "the connection is not established");
                }
                // FIXME: `EPIPE` is another possibility, which means that the socket is shut down
                // for writing. In that case, we should also trigger a `SIGPIPE` if `MSG_NOSIGNAL`
                // is not specified.
                return_errno_with_message!(Errno::ECONNRESET, "the connection is reset");
            }
        }
    }

    pub fn local_endpoint(&self) -> IpEndpoint {
        self.bound_socket.local_endpoint().unwrap()
    }

    pub fn remote_endpoint(&self) -> IpEndpoint {
        self.remote_endpoint
    }

    pub fn check_new(&mut self) -> Result<()> {
        if !self.is_new_connection {
            return_errno_with_message!(Errno::EISCONN, "the socket is already connected");
        }

        self.is_new_connection = false;
        Ok(())
    }

    pub(super) fn init_pollee(&self, pollee: &Pollee) {
        pollee.reset_events();
        self.update_io_events(pollee);
    }

    pub(super) fn update_io_events(&self, pollee: &Pollee) {
        self.bound_socket.raw_with(|socket| {
            if is_peer_closed(socket) {
                // Only the sending side of peer socket is closed
                self.is_receiving_closed.store(true, Ordering::Relaxed);
            } else if is_closed(socket) {
                // The sending side of both peer socket and this socket are closed
                self.is_receiving_closed.store(true, Ordering::Relaxed);
                self.is_sending_closed.store(true, Ordering::Relaxed);
            }

            let is_receiving_closed = self.is_receiving_closed.load(Ordering::Relaxed);
            let is_sending_closed = self.is_sending_closed.load(Ordering::Relaxed);

            // If the receiving side is closed, always add events IN and RDHUP;
            // otherwise, check if the socket can receive.
            if is_receiving_closed {
                pollee.add_events(IoEvents::IN | IoEvents::RDHUP);
            } else if socket.can_recv() {
                pollee.add_events(IoEvents::IN);
            } else {
                pollee.del_events(IoEvents::IN);
            }

            // If the sending side is closed, always add an OUT event;
            // otherwise, check if the socket can send.
            if is_sending_closed || socket.can_send() {
                pollee.add_events(IoEvents::OUT);
            } else {
                pollee.del_events(IoEvents::OUT);
            }

            // If both sending and receiving sides are closed, add a HUP event.
            if is_receiving_closed && is_sending_closed {
                pollee.add_events(IoEvents::HUP);
            }
        });
    }

    pub(super) fn set_observer(&self, observer: Weak<dyn SocketEventObserver>) {
        self.bound_socket.set_observer(observer)
    }

    /// Returns whether the connection is before established.
    ///
    /// Note that a newly accepted socket may not yet be in the [`TcpState::Established`] state.
    /// The accept syscall only verifies that a connection request is incoming by ensuring
    /// that the backlog socket is not in the [`TcpState::Listen`] state.
    /// However, the socket might still be waiting for further ACKs to complete the establishment process.
    /// Therefore, it could be in either the [`TcpState::SynSent`] or [`TcpState::SynReceived`] states.
    /// We must wait until the socket reaches the established state before it can send and receive data.
    ///
    /// FIXME: Should we check established state in accept or here?
    fn before_established(&self) -> bool {
        self.bound_socket.raw_with(|socket| {
            socket.state() == TcpState::SynSent || socket.state() == TcpState::SynReceived
        })
    }
}

/// Checks if the peer socket has closed its sending side.
///
/// If the sending side of this socket is also closed, this method will return `false`.
/// In such cases, you should verify using [`is_closed`].
fn is_peer_closed(socket: &RawTcpSocket) -> bool {
    socket.state() == TcpState::CloseWait
}

/// Checks if the socket is fully closed.
///
/// This function returns `true` if both this socket and the peer have closed their sending sides.
///
/// This TCP state corresponds to the `Normal Close Sequence` and `Simultaneous Close Sequence`
/// as outlined in RFC793 (https://datatracker.ietf.org/doc/html/rfc793#page-39).
fn is_closed(socket: &RawTcpSocket) -> bool {
    !socket.is_open() || socket.state() == TcpState::Closing || socket.state() == TcpState::LastAck
}
