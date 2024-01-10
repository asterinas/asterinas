use core::sync::atomic::{AtomicBool, Ordering};

use alloc::sync::Weak;
use smoltcp::socket::tcp::{RecvError, SendError};

use crate::events::{IoEvents, Observer};
use crate::net::iface::IpEndpoint;
use crate::process::signal::Pollee;
use crate::{
    net::{
        iface::{AnyBoundSocket, RawTcpSocket},
        socket::util::{send_recv_flags::SendRecvFlags, shutdown_cmd::SockShutdownCmd},
    },
    prelude::*,
};

pub struct ConnectedStream {
    bound_socket: Arc<AnyBoundSocket>,
    remote_endpoint: IpEndpoint,
    new_connection: AtomicBool,
}

impl ConnectedStream {
    pub fn new(
        bound_socket: Arc<AnyBoundSocket>,
        remote_endpoint: IpEndpoint,
        new_connection: bool,
    ) -> Self {
        Self {
            bound_socket,
            remote_endpoint,
            new_connection: AtomicBool::new(new_connection),
        }
    }

    pub fn shutdown(&self, cmd: SockShutdownCmd) -> Result<()> {
        // TODO: deal with cmd
        self.bound_socket.raw_with(|socket: &mut RawTcpSocket| {
            socket.close();
        });
        Ok(())
    }

    pub fn try_recvfrom(&self, buf: &mut [u8], flags: SendRecvFlags) -> Result<usize> {
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

    pub fn try_sendto(&self, buf: &[u8], flags: SendRecvFlags) -> Result<usize> {
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

    pub fn is_new_connection(&self) -> bool {
        self.new_connection.load(Ordering::Relaxed)
    }

    pub fn clear_new_connection(&self) {
        self.new_connection.store(false, Ordering::Relaxed)
    }

    pub(super) fn reset_io_events(&self, pollee: &Pollee) {
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

    pub(super) fn set_observer(&self, observer: Weak<dyn Observer<()>>) {
        self.bound_socket.set_observer(observer)
    }
}
