// SPDX-License-Identifier: MPL-2.0

use super::bound::BoundDatagram;
use crate::{
    events::IoEvents,
    net::{
        iface::{AnyUnboundSocket, IpEndpoint, RawUdpSocket},
        socket::ip::common::bind_socket,
    },
    prelude::*,
    process::signal::{Pollee, Poller},
};

pub struct UnboundDatagram {
    unbound_socket: Box<AnyUnboundSocket>,
    pollee: Pollee,
}

impl UnboundDatagram {
    pub fn new() -> Self {
        Self {
            unbound_socket: Box::new(AnyUnboundSocket::new_udp()),
            pollee: Pollee::new(IoEvents::empty()),
        }
    }

    pub fn poll(&self, mask: IoEvents, poller: Option<&Poller>) -> IoEvents {
        self.pollee.poll(mask, poller)
    }

    pub fn bind(
        self,
        endpoint: IpEndpoint,
    ) -> core::result::Result<Arc<BoundDatagram>, (Error, Self)> {
        let bound_socket = match bind_socket(self.unbound_socket, endpoint, false) {
            Ok(bound_socket) => bound_socket,
            Err((err, unbound_socket)) => {
                return Err((
                    err,
                    Self {
                        unbound_socket,
                        pollee: self.pollee,
                    },
                ))
            }
        };
        let bound_endpoint = bound_socket.local_endpoint().unwrap();
        bound_socket.raw_with(|socket: &mut RawUdpSocket| {
            socket.bind(bound_endpoint).unwrap();
        });
        Ok(BoundDatagram::new(bound_socket, self.pollee))
    }
}
