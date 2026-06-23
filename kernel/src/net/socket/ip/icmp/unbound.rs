// SPDX-License-Identifier: MPL-2.0

use aster_bigtcp::{
    socket::IcmpSocket,
    wire::{IpAddress, IpEndpoint},
};

use super::{bound::BoundIcmp, observer::IcmpObserver};
use crate::{
    events::IoEvents,
    net::{
        iface::BoundIcmpPort,
        socket::{
            ip::common::{get_ephemeral_endpoint, resolve_bind_raw_iface_and_addr},
            util::datagram_common,
        },
    },
    prelude::*,
    process::signal::Pollee,
    util::random::getrandom,
};

/// Unbound ICMP socket state.
pub(super) struct UnboundIcmp {
    /// ICMP identifier (used to match echo replies to requests).
    /// This is similar to a port number for ICMP.
    pub icmp_id: u16,
}

impl UnboundIcmp {
    pub(super) fn new() -> Self {
        // Generate a random ICMP identifier
        let mut bytes = [0u8; 2];
        getrandom(&mut bytes);
        let icmp_id = u16::from_ne_bytes(bytes);
        Self { icmp_id }
    }
}

impl datagram_common::Unbound for UnboundIcmp {
    type Endpoint = IpEndpoint;
    type BindOptions = ();
    type Bound = BoundIcmp;

    fn bind(
        &mut self,
        endpoint: &Self::Endpoint,
        pollee: &Pollee,
        _options: (),
    ) -> Result<Self::Bound> {
        let bound_port = bind_icmp(self.icmp_id, endpoint.addr)?;

        let bound_socket =
            IcmpSocket::new_bind(bound_port, self.icmp_id, IcmpObserver::new(pollee.clone()));

        Ok(BoundIcmp::new(bound_socket))
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

fn bind_icmp(icmp_id: u16, addr: IpAddress) -> Result<BoundIcmpPort> {
    let (iface, bind_addr) = resolve_bind_raw_iface_and_addr(addr)?;
    Ok(iface.bind_icmp(icmp_id, bind_addr)?)
}
