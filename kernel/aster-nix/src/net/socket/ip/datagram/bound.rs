// SPDX-License-Identifier: MPL-2.0

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

    pub fn remote_endpoint(&self) -> Result<IpEndpoint> {
        self.remote_endpoint
            .ok_or_else(|| Error::with_message(Errno::EINVAL, "remote endpoint is not specified"))
    }

    pub fn set_remote_endpoint(&mut self, endpoint: &IpEndpoint) {
        self.remote_endpoint = Some(*endpoint)
    }

    pub fn try_recvfrom(
        &self,
        buf: &mut [u8],
        flags: SendRecvFlags,
    ) -> Result<(usize, IpEndpoint)> {
        self.bound_socket
            .raw_with(|socket: &mut RawUdpSocket| socket.recv_slice(buf))
            .map_err(|_| Error::with_message(Errno::EAGAIN, "recv buf is empty"))
    }

    pub fn try_sendto(
        &self,
        buf: &[u8],
        remote: Option<IpEndpoint>,
        flags: SendRecvFlags,
    ) -> Result<usize> {
        let remote_endpoint = remote
            .or(self.remote_endpoint)
            .ok_or_else(|| Error::with_message(Errno::EINVAL, "udp should provide remote addr"))?;
        self.bound_socket
            .raw_with(|socket: &mut RawUdpSocket| socket.send_slice(buf, remote_endpoint))
            .map(|_| buf.len())
            .map_err(|_| Error::with_message(Errno::EAGAIN, "send udp packet fails"))
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
