use alloc::sync::Weak;

use crate::events::{IoEvents, Observer};
use crate::net::iface::IpEndpoint;

use crate::net::socket::ip::common::bind_socket;
use crate::process::signal::Pollee;
use crate::{
    net::iface::{AnyUnboundSocket, RawUdpSocket},
    prelude::*,
};

use super::bound::BoundDatagram;

pub struct UnboundDatagram {
    unbound_socket: Box<AnyUnboundSocket>,
}

impl UnboundDatagram {
    pub fn new(observer: Weak<dyn Observer<()>>) -> Self {
        Self {
            unbound_socket: Box::new(AnyUnboundSocket::new_udp(observer)),
        }
    }

    pub fn bind(self, endpoint: &IpEndpoint) -> core::result::Result<BoundDatagram, (Error, Self)> {
        let bound_socket = match bind_socket(self.unbound_socket, endpoint, false) {
            Ok(bound_socket) => bound_socket,
            Err((err, unbound_socket)) => return Err((err, Self { unbound_socket })),
        };

        let bound_endpoint = bound_socket.local_endpoint().unwrap();
        bound_socket.raw_with(|socket: &mut RawUdpSocket| {
            socket.bind(bound_endpoint).unwrap();
        });

        Ok(BoundDatagram::new(bound_socket))
    }

    pub(super) fn reset_io_events(&self, pollee: &Pollee) {
        pollee.del_events(IoEvents::IN);
        pollee.add_events(IoEvents::OUT);
    }
}
