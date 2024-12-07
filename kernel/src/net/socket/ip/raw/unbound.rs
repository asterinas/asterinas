// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Weak;

use aster_bigtcp::{
    socket::{SocketEventObserver, UnboundRawSocket},
    wire::{IpAddress, IpEndpoint, IpProtocol},
};

use super::bound::BoundRaw;
use crate::{
    events::IoEvents, net::socket::ip::common::bind_socket, prelude::*, util::net::Protocol,
};

pub struct UnBoundRaw {
    unbound_socket: Box<UnboundRawSocket>,
    ip_protocol: IpProtocol,
}

impl UnBoundRaw {
    pub fn new(observer: Weak<dyn SocketEventObserver>, protocol: Protocol) -> Self {
        let ip_protocol = match protocol {
            //support more?
            Protocol::IPPROTO_TCP => IpProtocol::Tcp,
            Protocol::IPPROTO_UDP => IpProtocol::Udp,
            Protocol::IPPROTO_ICMP => IpProtocol::Icmp,
            _ => {
                todo!("this protocol of raw socket is not supported yet.")
            }
        };

        Self {
            unbound_socket: Box::new(UnboundRawSocket::new(observer, ip_protocol)),
            ip_protocol,
        }
    }

    pub fn bind(
        self,
        addr: &IpAddress,
        can_reuse: bool,
    ) -> core::result::Result<BoundRaw, (Error, Self)> {
        let endpoint = IpEndpoint {
            addr: *addr,
            port: 0,
        };
        let bound_socket = match bind_socket(
            self.unbound_socket,
            &endpoint,
            can_reuse,
            |iface, socket, config| iface.bind_raw(socket, config, self.ip_protocol),
        ) {
            Ok(bound_socket) => bound_socket,
            Err((err, unbound_socket)) => {
                return Err((
                    err,
                    Self {
                        unbound_socket,
                        ip_protocol: self.ip_protocol,
                    },
                ))
            }
        };

        //native socket in smoltcp is no need to bind.

        Ok(BoundRaw::new(bound_socket, self.ip_protocol))
    }

    pub(super) fn check_io_events(&self) -> IoEvents {
        IoEvents::OUT
    }
}
