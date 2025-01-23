// SPDX-License-Identifier: MPL-2.0

use aster_bigtcp::{
    iface::BoundPort,
    socket::RawSocket,
    wire::{IpAddress, IpProtocol},
};

use super::{bound::BoundRaw, RawSocketObserver};
use crate::{events::IoEvents, net::socket::ip::common::get_ephemeral_iface, prelude::*};

pub struct UnBoundRaw {
    ip_protocol: IpProtocol,
    _private: (),
}

impl UnBoundRaw {
    pub fn new(ip_protocol: IpProtocol) -> Self {
        Self {
            ip_protocol,
            _private: (),
        }
    }

    pub fn bind(
        self,
        addr: &IpAddress,
        observer: RawSocketObserver,
    ) -> core::result::Result<BoundRaw, (Error, Self)> {
        let iface = get_ephemeral_iface(addr);

        // NOTICE: It is a dummy implementation because raw sockets do not need to bind to a port.
        let bound_port = BoundPort { iface, port: 0_u16 };

        let bound_socket = match RawSocket::new_bind(bound_port, observer, self.ip_protocol) {
            Ok(bound_socket) => bound_socket,
            Err((_, err)) => {
                unreachable!("`new_bind fails with {:?}, which should not happen", err)
            }
        };

        Ok(BoundRaw::new(bound_socket, self.ip_protocol))
    }
    pub(super) fn check_io_events(&self) -> IoEvents {
        IoEvents::OUT
    }
}
