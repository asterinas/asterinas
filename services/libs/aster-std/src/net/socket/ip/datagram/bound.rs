// SPDX-License-Identifier: MPL-2.0

use crate::events::{IoEvents, Observer};
use crate::net::iface::IpEndpoint;

use crate::net::poll_ifaces;
use crate::process::signal::{Pollee, Poller};
use crate::{
    net::{
        iface::{AnyBoundSocket, RawUdpSocket},
        socket::util::send_recv_flags::SendRecvFlags,
    },
    prelude::*,
};

pub struct BoundDatagram {
    bound_socket: Arc<AnyBoundSocket>,
    remote_endpoint: RwLock<Option<IpEndpoint>>,
    pollee: Pollee,
}

impl BoundDatagram {
    pub fn new(bound_socket: Arc<AnyBoundSocket>, pollee: Pollee) -> Arc<Self> {
        let bound = Arc::new(Self {
            bound_socket,
            remote_endpoint: RwLock::new(None),
            pollee,
        });
        bound.bound_socket.set_observer(Arc::downgrade(&bound) as _);
        bound
    }

    pub fn remote_endpoint(&self) -> Result<IpEndpoint> {
        self.remote_endpoint
            .read()
            .ok_or_else(|| Error::with_message(Errno::EINVAL, "remote endpoint is not specified"))
    }

    pub fn set_remote_endpoint(&self, endpoint: IpEndpoint) {
        *self.remote_endpoint.write() = Some(endpoint);
    }

    pub fn local_endpoint(&self) -> Result<IpEndpoint> {
        self.bound_socket.local_endpoint().ok_or_else(|| {
            Error::with_message(Errno::EINVAL, "socket does not bind to local endpoint")
        })
    }

    pub fn try_recvfrom(
        &self,
        buf: &mut [u8],
        flags: &SendRecvFlags,
    ) -> Result<(usize, IpEndpoint)> {
        poll_ifaces();
        let recv_slice = |socket: &mut RawUdpSocket| {
            socket
                .recv_slice(buf)
                .map_err(|_| Error::with_message(Errno::EAGAIN, "recv buf is empty"))
        };
        self.bound_socket.raw_with(recv_slice)
    }

    pub fn try_sendto(
        &self,
        buf: &[u8],
        remote: Option<IpEndpoint>,
        flags: SendRecvFlags,
    ) -> Result<usize> {
        let remote_endpoint = remote
            .or_else(|| self.remote_endpoint().ok())
            .ok_or_else(|| Error::with_message(Errno::EINVAL, "udp should provide remote addr"))?;
        let send_slice = |socket: &mut RawUdpSocket| {
            socket
                .send_slice(buf, remote_endpoint)
                .map(|_| buf.len())
                .map_err(|_| Error::with_message(Errno::EAGAIN, "send udp packet fails"))
        };
        let len = self.bound_socket.raw_with(send_slice)?;
        poll_ifaces();
        Ok(len)
    }

    pub fn poll(&self, mask: IoEvents, poller: Option<&Poller>) -> IoEvents {
        self.pollee.poll(mask, poller)
    }

    fn update_io_events(&self) {
        self.bound_socket.raw_with(|socket: &mut RawUdpSocket| {
            let pollee = &self.pollee;

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

impl Observer<()> for BoundDatagram {
    fn on_events(&self, _: &()) {
        self.update_io_events();
    }
}
