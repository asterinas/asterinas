// SPDX-License-Identifier: MPL-2.0

use aster_bigtcp::{socket::UdpSocket, wire::IpEndpoint};

use super::{bound::BoundDatagram, observer::DatagramObserver};
use crate::{
    events::IoEvents,
    net::{
        iface::BoundUdpPort,
        socket::{
            ip::{
                addr::IpAddressFamily,
                common::{get_ephemeral_endpoint, resolve_bind_iface_and_config},
            },
            util::datagram_common,
        },
    },
    prelude::*,
    process::signal::Pollee,
};

pub(super) struct UnboundDatagram {
    family: IpAddressFamily,
}

impl UnboundDatagram {
    pub(super) fn new(family: IpAddressFamily) -> Self {
        Self { family }
    }
}

pub(super) struct BindOptions {
    pub(super) can_reuse: bool,
    pub(super) is_ipv6_only: bool,
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
        self.check_endpoint_family(endpoint)?;
        let bound_port = bind_port(endpoint, options.can_reuse, options.is_ipv6_only)?;

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
        self.check_endpoint_family(remote_endpoint)?;
        let endpoint = get_ephemeral_endpoint(remote_endpoint).ok_or_else(|| {
            Error::with_message(
                Errno::EADDRNOTAVAIL,
                "no interface has an address for the specified family",
            )
        })?;
        self.bind(
            &endpoint,
            pollee,
            BindOptions {
                can_reuse: false,
                is_ipv6_only: false,
            },
        )
    }

    fn check_io_events(&self) -> IoEvents {
        IoEvents::OUT
    }
}

impl UnboundDatagram {
    fn check_endpoint_family(&self, endpoint: &IpEndpoint) -> Result<()> {
        let endpoint_family = IpAddressFamily::from(endpoint.addr);
        if endpoint_family != self.family
            && !(self.family == IpAddressFamily::IPv6 && endpoint_family == IpAddressFamily::IPv4)
        {
            return_errno_with_message!(
                Errno::EAFNOSUPPORT,
                "the protocol family does not match the address family"
            );
        }

        Ok(())
    }
}

fn bind_port(endpoint: &IpEndpoint, can_reuse: bool, is_ipv6_only: bool) -> Result<BoundUdpPort> {
    let (iface, config) = resolve_bind_iface_and_config(endpoint, can_reuse, is_ipv6_only)?;
    Ok(iface.bind_udp(config)?)
}
