// SPDX-License-Identifier: MPL-2.0

use aster_bigtcp::{
    socket::RawSocket,
    wire::{IpAddress, IpEndpoint, IpProtocol},
};

use super::{bound::BoundRaw, observer::RawObserver};
use crate::{
    events::IoEvents,
    net::{
        iface::BoundRawPort,
        socket::{
            ip::common::{get_ephemeral_endpoint, resolve_bind_raw_iface_and_addr},
            util::datagram_common,
        },
    },
    prelude::*,
    process::signal::Pollee,
};

pub(super) struct UnboundRaw {
    pub protocol: IpProtocol,
}

impl UnboundRaw {
    pub(super) fn new(protocol: IpProtocol) -> Self {
        Self { protocol }
    }
}

impl datagram_common::Unbound for UnboundRaw {
    type Endpoint = IpEndpoint;
    type BindOptions = ();

    type Bound = BoundRaw;

    fn bind(
        &mut self,
        endpoint: &Self::Endpoint,
        pollee: &Pollee,
        _options: (),
    ) -> Result<Self::Bound> {
        let bound_port = bind_raw_protocol(self.protocol, endpoint.addr, endpoint.port)?;

        let bound_socket =
            RawSocket::new_bind(bound_port, self.protocol, RawObserver::new(pollee.clone()));

        Ok(BoundRaw::new(bound_socket, endpoint.port))
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
        self.bind(&endpoint, pollee, ())
    }

    fn check_io_events(&self) -> IoEvents {
        IoEvents::OUT
    }
}

fn bind_raw_protocol(protocol: IpProtocol, addr: IpAddress, port: u16) -> Result<BoundRawPort> {
    let (iface, bind_addr) = resolve_bind_raw_iface_and_addr(addr)?;
    Ok(iface.bind_raw(protocol, bind_addr, port)?)
}
