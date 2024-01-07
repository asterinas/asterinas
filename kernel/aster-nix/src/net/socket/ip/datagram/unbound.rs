// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Weak;

use super::bound::BoundDatagram;
use crate::{
    events::Observer,
    net::{
        iface::{AnyUnboundSocket, IpEndpoint, RawUdpSocket},
        socket::ip::common::bind_socket,
    },
    prelude::*,
};

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
}
