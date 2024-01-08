// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Weak;

use smoltcp::socket::tcp::{RecvError, SendError};

use crate::{
    events::{IoEvents, Observer},
    net::{
        iface::{AnyBoundSocket, IpEndpoint, RawTcpSocket},
        socket::util::{send_recv_flags::SendRecvFlags, shutdown_cmd::SockShutdownCmd},
    },
    prelude::*,
    process::signal::Pollee,
};

pub struct ConnectedStream {
    bound_socket: Arc<AnyBoundSocket>,
    remote_endpoint: IpEndpoint,
}

impl ConnectedStream {
    pub fn new(bound_socket: Arc<AnyBoundSocket>, remote_endpoint: IpEndpoint) -> Self {
        Self {
            bound_socket,
            remote_endpoint,
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

    pub(super) fn init_pollee(&self, pollee: &Pollee) {
        pollee.reset_events();
        self.update_io_events(pollee);
    }

    pub fn register_observer(
        &self,
        pollee: &Pollee,
        observer: Weak<dyn Observer<IoEvents>>,
        mask: IoEvents,
    ) -> Result<()> {
        pollee.register_observer(observer, mask);
        Ok(())
    }

    pub fn unregister_observer(
        &self,
        pollee: &Pollee,
        observer: &Weak<dyn Observer<IoEvents>>,
    ) -> Result<Weak<dyn Observer<IoEvents>>> {
        pollee
            .unregister_observer(observer)
            .ok_or_else(|| Error::with_message(Errno::EINVAL, "fails to unregister observer"))
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
