// SPDX-License-Identifier: MPL-2.0

use aster_bigtcp::{socket::UnboundUdpSocket, wire::IpEndpoint};

use super::{bound::BoundDatagram, DatagramObserver};
use crate::{events::IoEvents, net::socket::ip::common::bind_socket, prelude::*};

pub struct UnboundDatagram {
    unbound_socket: Box<UnboundUdpSocket>,
}

impl UnboundDatagram {
    pub fn new() -> Self {
        Self {
            unbound_socket: Box::new(UnboundUdpSocket::new()),
        }
    }

    pub fn bind(
        self,
        endpoint: &IpEndpoint,
        can_reuse: bool,
        observer: DatagramObserver,
    ) -> core::result::Result<BoundDatagram, (Error, Self)> {
        let bound_socket = match bind_socket(
            self.unbound_socket,
            endpoint,
            can_reuse,
            |iface, socket, config| iface.bind_udp(socket, observer, config),
        ) {
            Ok(bound_socket) => bound_socket,
            Err((err, unbound_socket)) => return Err((err, Self { unbound_socket })),
        };

        let bound_endpoint = bound_socket.local_endpoint().unwrap();
        bound_socket.bind(bound_endpoint).unwrap();

        Ok(BoundDatagram::new(bound_socket))
    }

    pub(super) fn check_io_events(&self) -> IoEvents {
        IoEvents::OUT
    }
}
