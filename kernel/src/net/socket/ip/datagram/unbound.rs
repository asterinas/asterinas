// SPDX-License-Identifier: MPL-2.0

use aster_bigtcp::{socket::UdpSocket, wire::IpEndpoint};

use super::{bound::BoundDatagram, observer::DatagramObserver};
use crate::{
    events::IoEvents,
    net::socket::{
        ip::common::{bind_port, get_ephemeral_endpoint},
        util::datagram_common,
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
        let endpoint = get_ephemeral_endpoint(remote_endpoint);
        self.bind(&endpoint, pollee, BindOptions { can_reuse: false })
    }

    fn check_io_events(&self) -> IoEvents {
        IoEvents::OUT
    }
}
