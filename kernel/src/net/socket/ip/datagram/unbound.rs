// SPDX-License-Identifier: MPL-2.0

use aster_bigtcp::{socket::UdpSocket, wire::IpEndpoint};

use super::{bound::BoundDatagram, observer::DatagramObserver};
use crate::{
    events::IoEvents,
    net::{
        iface::BoundUdpPort,
        socket::{
            ip::common::{get_ephemeral_endpoint, resolve_bind_iface_and_config},
            util::datagram_common,
        },
    },
    prelude::*,
    process::signal::Pollee,
};

pub(super) struct UnboundDatagram {
    _private: (),
}

impl UnboundDatagram {
    pub(super) fn new() -> Self {
        Self { _private: () }
    }
}

pub(super) struct BindOptions {
    pub(super) can_reuse: bool,
}

impl datagram_common::Unbound for UnboundDatagram {
    type Endpoint = IpEndpoint;
    type BindOptions = BindOptions;

    type Bound = BoundDatagram;

    fn bind(
        &mut self,
        endpoint: &Self::Endpoint,
        pollee: &Pollee,
        options: BindOptions,
    ) -> Result<Self::Bound> {
        let bound_port = bind_port(endpoint, options.can_reuse)?;

        let bound_socket =
            match UdpSocket::new_bind(bound_port, DatagramObserver::new(pollee.clone())) {
                Ok(bound_socket) => bound_socket,
                Err((_, err)) => {
                    unreachable!("`new_bind` fails with {:?}, which should not happen", err)
                }
            };

        Ok(BoundDatagram::new(bound_socket))
    }

    fn bind_ephemeral(
        &mut self,
        remote_endpoint: &Self::Endpoint,
        pollee: &Pollee,
    ) -> Result<Self::Bound> {
        let endpoint = get_ephemeral_endpoint(remote_endpoint).ok_or_else(|| {
            Error::with_message(
                Errno::EADDRNOTAVAIL,
                "no interface has an address for the specified family",
            )
        })?;
        self.bind(&endpoint, pollee, BindOptions { can_reuse: false })
    }

    fn check_io_events(&self) -> IoEvents {
        IoEvents::OUT
    }
}

fn bind_port(endpoint: &IpEndpoint, can_reuse: bool) -> Result<BoundUdpPort> {
    let (iface, config) = resolve_bind_iface_and_config(endpoint, can_reuse)?;
    Ok(iface.bind_udp(config)?)
}
