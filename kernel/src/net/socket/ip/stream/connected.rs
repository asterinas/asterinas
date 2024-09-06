// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Weak;

use aster_bigtcp::{
    errors::tcp::{RecvError, SendError},
    socket::{RawTcpSocket, SocketEventObserver},
    wire::IpEndpoint,
};

use crate::{
    events::IoEvents,
    net::{
        iface::AnyBoundSocket,
        socket::util::{send_recv_flags::SendRecvFlags, shutdown_cmd::SockShutdownCmd},
    },
    prelude::*,
    process::signal::Pollee,
};

pub struct ConnectedStream {
    bound_socket: AnyBoundSocket,
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
}

impl ConnectedStream {
    pub fn new(
        bound_socket: AnyBoundSocket,
        remote_endpoint: IpEndpoint,
        is_new_connection: bool,
    ) -> Self {
        Self {
            bound_socket,
            remote_endpoint,
            is_new_connection,
        }
    }

    pub fn shutdown(&self, _cmd: SockShutdownCmd) -> Result<()> {
        // TODO: deal with cmd
        self.bound_socket.raw_with(|socket: &mut RawTcpSocket| {
            socket.close();
        });
        Ok(())
    }

    pub fn try_recv(&self, buf: &mut [u8], _flags: SendRecvFlags) -> Result<usize> {
        let result = self
            .bound_socket
            .raw_with(|socket: &mut RawTcpSocket| socket.recv_slice(buf));

        match result {
            Ok(0) => return_errno_with_message!(Errno::EAGAIN, "the receive buffer is empty"),
            Ok(recv_bytes) => Ok(recv_bytes),
            Err(RecvError::Finished) => Ok(0),
            Err(RecvError::InvalidState) => {
                return_errno_with_message!(Errno::ECONNRESET, "the connection is reset")
            }
        }
    }

    pub fn try_send(&self, buf: &[u8], _flags: SendRecvFlags) -> Result<usize> {
        let result = self
            .bound_socket
            .raw_with(|socket: &mut RawTcpSocket| socket.send_slice(buf));

        match result {
            Ok(0) => return_errno_with_message!(Errno::EAGAIN, "the send buffer is full"),
            Ok(sent_bytes) => Ok(sent_bytes),
            Err(SendError::InvalidState) => {
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
        self.bound_socket.raw_with(|socket: &mut RawTcpSocket| {
            if socket.can_recv() {
                pollee.add_events(IoEvents::IN);
            } else {
                pollee.del_events(IoEvents::IN);
            }

            if socket.can_send() {
                pollee.add_events(IoEvents::OUT);
            } else {
                pollee.del_events(IoEvents::OUT);
            }
        });
    }

    pub(super) fn set_observer(&self, observer: Weak<dyn SocketEventObserver>) {
        self.bound_socket.set_observer(observer)
    }
}
