// SPDX-License-Identifier: MPL-2.0

use aster_bigtcp::{socket::UdpSocket, wire::IpEndpoint};

use super::{bound::BoundDatagram, DatagramObserver};
use crate::{events::IoEvents, net::socket::ip::common::bind_port, prelude::*};

pub struct UnboundDatagram {
    _private: (),
}

impl UnboundDatagram {
    pub fn new() -> Self {
        Self { _private: () }
    }

    pub fn bind(
        self,
        endpoint: &IpEndpoint,
        can_reuse: bool,
        observer: DatagramObserver,
    ) -> core::result::Result<BoundDatagram, (Error, Self)> {
        let bound_port = match bind_port(endpoint, can_reuse) {
            Ok(bound_port) => bound_port,
            Err(err) => return Err((err, self)),
        };

        let bound_socket = match UdpSocket::new_bind(bound_port, observer) {
            Ok(bound_socket) => bound_socket,
            Err((_, err)) => {
                unreachable!("`new_bind fails with {:?}, which should not happen", err)
            }
        };

        Ok(BoundDatagram::new(bound_socket))
    }

    pub(super) fn check_io_events(&self) -> IoEvents {
        IoEvents::OUT
    }
}
