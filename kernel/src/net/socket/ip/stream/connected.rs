// SPDX-License-Identifier: MPL-2.0

use aster_bigtcp::{
    errors::tcp::{RecvError, SendError},
    socket::{NeedIfacePoll, RawTcpSetOption},
    wire::IpEndpoint,
};

use super::observer::StreamObserver;
use crate::{
    events::IoEvents,
    net::{
        iface::{BoundPort, Iface, RawTcpSocketExt, TcpConnection},
        socket::util::{LingerOption, SendRecvFlags, SockShutdownCmd},
    },
    prelude::*,
    process::signal::{Pollee, Poller},
    util::{MultiRead, MultiWrite},
};

pub(super) struct ConnectedStream {
    tcp_conn: TcpConnection,
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
    pub(super) fn new(
        tcp_conn: TcpConnection,
        remote_endpoint: IpEndpoint,
        is_new_connection: bool,
    ) -> Self {
        Self {
            tcp_conn,
            remote_endpoint,
            is_new_connection,
        }
    }

    pub(super) fn shutdown(&self, cmd: SockShutdownCmd, pollee: &Pollee) -> Result<()> {
        let mut events = IoEvents::empty();

        if cmd.shut_read() {
            if !self.tcp_conn.shut_recv() {
                return_errno_with_message!(Errno::ENOTCONN, "the socket is not connected");
            }
            events |= IoEvents::IN | IoEvents::RDHUP;
        }

        if cmd.shut_write() {
            if !self.tcp_conn.shut_send() {
                return_errno_with_message!(Errno::ENOTCONN, "the socket is not connected");
            }
            events |= IoEvents::OUT | IoEvents::HUP;
        }

        pollee.notify(events);

        Ok(())
    }

    pub(super) fn try_recv(
        &self,
        writer: &mut dyn MultiWrite,
        _flags: SendRecvFlags,
    ) -> Result<(usize, NeedIfacePoll)> {
        let result = self.tcp_conn.recv(|socket_buffer| {
            match writer.write(&mut VmReader::from(&*socket_buffer)) {
                Ok(len) => (len, Ok(len)),
                Err(e) => (0, Err(e)),
            }
        });

        match result {
            Ok((Ok(0), need_poll)) => {
                debug_assert!(!*need_poll);
                return_errno_with_message!(Errno::EAGAIN, "the receive buffer is empty")
            }
            Ok((Ok(recv_bytes), need_poll)) => Ok((recv_bytes, need_poll)),
            Ok((Err(e), need_poll)) => {
                debug_assert!(!*need_poll);
                Err(e)
            }
            Err(RecvError::Finished) | Err(RecvError::InvalidState) => {
                // `InvalidState` occurs when the connection is reset but `ECONNRESET` was reported
                // earlier. Linux returns EOF in this case, so we follow it.
                Ok((0, NeedIfacePoll::FALSE))
            }
            Err(RecvError::ConnReset) => {
                return_errno_with_message!(Errno::ECONNRESET, "the connection is reset")
            }
        }
    }

    pub(super) fn try_send(
        &self,
        reader: &mut dyn MultiRead,
        _flags: SendRecvFlags,
    ) -> Result<(usize, NeedIfacePoll)> {
        let result = self.tcp_conn.send(|socket_buffer| {
            match reader.read(&mut VmWriter::from(socket_buffer)) {
                Ok(len) => (len, Ok(len)),
                Err(e) => (0, Err(e)),
            }
        });

        match result {
            Ok((Ok(0), need_poll)) => {
                debug_assert!(!*need_poll);
                return_errno_with_message!(Errno::EAGAIN, "the send buffer is full")
            }
            Ok((Ok(sent_bytes), need_poll)) => Ok((sent_bytes, need_poll)),
            Ok((Err(e), need_poll)) => {
                debug_assert!(!*need_poll);
                Err(e)
            }
            Err(SendError::InvalidState) => {
                return_errno_with_message!(Errno::EPIPE, "the connection is closed");
            }
            Err(SendError::ConnReset) => {
                return_errno_with_message!(Errno::ECONNRESET, "the connection is reset");
            }
        }
    }

    pub(super) fn local_endpoint(&self) -> IpEndpoint {
        self.tcp_conn.local_endpoint().unwrap()
    }

    pub(super) fn remote_endpoint(&self) -> IpEndpoint {
        self.remote_endpoint
    }

    pub(super) fn iface(&self) -> &Arc<Iface> {
        self.tcp_conn.iface()
    }

    pub(super) fn bound_port(&self) -> &BoundPort {
        self.tcp_conn.bound_port()
    }

    pub(super) fn finish_last_connect(&mut self) -> Result<()> {
        if !self.is_new_connection {
            return_errno_with_message!(Errno::EISCONN, "the socket is already connected");
        }

        self.is_new_connection = false;
        Ok(())
    }

    pub(super) fn init_observer(&self, observer: StreamObserver) {
        self.tcp_conn.init_observer(observer);
    }

    pub(super) fn check_io_events(&self) -> IoEvents {
        self.tcp_conn.raw_with(|socket| {
            let is_receiving_closed = socket.is_recv_shut() || !socket.may_recv_new();
            let is_sending_closed = !socket.may_send();

            let mut events = IoEvents::empty();

            // If the receiving side is closed, always add events IN and RDHUP;
            // otherwise, check if the socket can receive.
            if is_receiving_closed {
                events |= IoEvents::IN | IoEvents::RDHUP;
            } else if socket.can_recv() {
                events |= IoEvents::IN;
            }

            // If the sending side is closed, always add an OUT event;
            // otherwise, check if the socket can send.
            if is_sending_closed || socket.can_send() {
                events |= IoEvents::OUT;
            }

            // If both sending and receiving sides are closed, add a HUP event.
            if is_receiving_closed && is_sending_closed {
                events |= IoEvents::HUP;
            }

            // If the connection is reset, add an ERR event.
            if socket.is_rst_closed() {
                events |= IoEvents::ERR;
            }

            events
        })
    }

    pub(super) fn test_and_clear_error(&self) -> Option<Error> {
        if self.tcp_conn.clear_rst_closed() {
            Some(Error::with_message(
                Errno::ECONNRESET,
                "the connection is reset",
            ))
        } else {
            None
        }
    }

    pub(super) fn set_raw_option<R>(
        &self,
        set_option: impl FnOnce(&dyn RawTcpSetOption) -> R,
    ) -> R {
        set_option(&self.tcp_conn)
    }

    pub(super) fn raw_with<R>(&self, f: impl FnOnce(&RawTcpSocketExt) -> R) -> R {
        self.tcp_conn.raw_with(f)
    }

    pub(super) fn into_connection(self) -> TcpConnection {
        self.tcp_conn
    }
}

pub(super) fn close_and_linger(tcp_conn: TcpConnection, linger: LingerOption, pollee: &Pollee) {
    let timeout = match (linger.is_on(), linger.timeout()) {
        // No linger. Drain the send buffer in the background.
        (false, _) => {
            tcp_conn.close();
            tcp_conn.iface().poll();
            return;
        }
        // Linger with a zero timeout. Reset the connection immediately.
        (true, duration) if duration.is_zero() => {
            tcp_conn.reset();
            tcp_conn.iface().poll();
            return;
        }
        // Linger with a non-zero timeout. See below.
        (true, duration) => {
            tcp_conn.close();
            tcp_conn.iface().poll();
            duration
        }
    };

    let mut poller = Poller::new(Some(&timeout));
    pollee.register_poller(poller.as_handle_mut(), IoEvents::HUP);

    // Now wait for the ACK packet to acknowledge the FIN packet we sent. If the timeout expires or
    // we are interrupted by signals, the remaining task is done in the background.
    while tcp_conn.raw_with(|socket| socket.is_closing()) {
        if poller.wait().is_err() {
            break;
        }
    }
}
