// SPDX-License-Identifier: MPL-2.0
use smoltcp::socket::udp::{RecvError, SendError};

use crate::{
    events::IoEvents,
    net::{
        iface::{AnyBoundSocket, IpEndpoint, RawUdpSocket},
        socket::util::send_recv_flags::SendRecvFlags,
    },
    prelude::*,
    process::signal::Pollee,
};

pub struct BoundDatagram {
    bound_socket: Arc<AnyBoundSocket>,
    remote_endpoint: Option<IpEndpoint>,
}

impl BoundDatagram {
    pub fn new(bound_socket: Arc<AnyBoundSocket>) -> Self {
        Self {
            bound_socket,
            remote_endpoint: None,
        }
    }

    pub fn local_endpoint(&self) -> IpEndpoint {
        self.bound_socket.local_endpoint().unwrap()
    }

    pub fn remote_endpoint(&self) -> Option<IpEndpoint> {
        self.remote_endpoint
    }

    pub fn set_remote_endpoint(&mut self, endpoint: &IpEndpoint) {
        self.remote_endpoint = Some(*endpoint)
    }

    pub fn try_recvfrom(
        &self,
        buf: &mut [u8],
        flags: SendRecvFlags,
    ) -> Result<(usize, IpEndpoint)> {
        let result = self
            .bound_socket
            .raw_with(|socket: &mut RawUdpSocket| socket.recv_slice(buf));
        match result {
            Ok((recv_len, endpoint)) => Ok((recv_len, endpoint)),
            Err(RecvError::Exhausted) => {
                return_errno_with_message!(Errno::EAGAIN, "the receive buffer is empty")
            }
        }
    }

    pub fn try_sendto(
        &self,
        buf: &[u8],
        remote: &IpEndpoint,
        flags: SendRecvFlags,
    ) -> Result<usize> {
        let result = self.bound_socket.raw_with(|socket: &mut RawUdpSocket| {
            if socket.payload_send_capacity() < buf.len() {
                return None;
            }
            Some(socket.send_slice(buf, *remote))
        });
        match result {
            Some(Ok(())) => Ok(buf.len()),
            Some(Err(SendError::BufferFull)) => {
                return_errno_with_message!(Errno::EAGAIN, "the send buffer is full")
            }
            Some(Err(SendError::Unaddressable)) => {
                return_errno_with_message!(Errno::EINVAL, "the destionation address is invalid")
            }
            None => return_errno_with_message!(Errno::EMSGSIZE, "the message is too large"),
        }
    }

    pub(super) fn init_pollee(&self, pollee: &Pollee) {
        pollee.reset_events();
        self.update_io_events(pollee)
    }

    pub(super) fn update_io_events(&self, pollee: &Pollee) {
        self.bound_socket.raw_with(|socket: &mut RawUdpSocket| {
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
}
