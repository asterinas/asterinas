// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Weak;

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
        let recv_bytes = self
            .bound_socket
            .raw_with(|socket: &mut RawTcpSocket| socket.recv_slice(buf))
            .map_err(|_| Error::with_message(Errno::ENOTCONN, "fail to recv packet"))?;
        if recv_bytes == 0 {
            return_errno_with_message!(Errno::EAGAIN, "try to recv again");
        }
        Ok(recv_bytes)
    }

    pub fn try_sendto(&self, buf: &[u8], flags: SendRecvFlags) -> Result<usize> {
        let sent_bytes = self
            .bound_socket
            .raw_with(|socket: &mut RawTcpSocket| socket.send_slice(buf))
            .map_err(|_| Error::with_message(Errno::ENOBUFS, "cannot send packet"))?;
        if sent_bytes == 0 {
            return_errno_with_message!(Errno::EAGAIN, "try to send again");
        }
        Ok(sent_bytes)
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
