// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Weak;

use aster_bigtcp::{
    socket::{SocketEventObserver, UnboundUdpSocket},
    wire::IpEndpoint,
};

use super::bound::BoundDatagram;
use crate::{
    events::IoEvents, net::socket::ip::common::bind_socket, prelude::*, process::signal::Pollee,
};

pub struct UnboundDatagram {
    unbound_socket: Box<UnboundUdpSocket>,
}

impl UnboundDatagram {
    pub fn new(observer: Weak<dyn SocketEventObserver>) -> Self {
        Self {
            unbound_socket: Box::new(UnboundUdpSocket::new(observer)),
        }
    }

    pub fn bind(
        self,
        endpoint: &IpEndpoint,
        can_reuse: bool,
    ) -> core::result::Result<BoundDatagram, (Error, Self)> {
        let bound_socket = match bind_socket(
            self.unbound_socket,
            endpoint,
            can_reuse,
            |iface, socket, config| iface.bind_udp(socket, config),
        ) {
            Ok(bound_socket) => bound_socket,
            Err((err, unbound_socket)) => return Err((err, Self { unbound_socket })),
        };

        let bound_endpoint = bound_socket.local_endpoint().unwrap();
        bound_socket.bind(bound_endpoint).unwrap();

        Ok(BoundDatagram::new(bound_socket))
    }

    pub(super) fn init_pollee(&self, pollee: &Pollee) {
        pollee.reset_events();
        pollee.add_events(IoEvents::OUT);
    }
}
