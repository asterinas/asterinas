// SPDX-License-Identifier: MPL-2.0

use super::bound::BoundNetlinkRoute;
use crate::{
    events::IoEvents,
    net::socket::{
        netlink::{table::NETLINK_SOCKET_TABLE, NetlinkSocketAddr, StandardNetlinkProtocol},
        util::datagram_common,
    },
    prelude::*,
    process::signal::Pollee,
};

pub(super) struct UnboundNetlinkRoute {
    _private: (),
}

impl UnboundNetlinkRoute {
    pub(super) const fn new() -> Self {
        Self { _private: () }
    }
}

impl datagram_common::Unbound for UnboundNetlinkRoute {
    type Endpoint = NetlinkSocketAddr;
    type BindOptions = ();

    type Bound = BoundNetlinkRoute;

    fn bind(
        &mut self,
        endpoint: &Self::Endpoint,
        _pollee: &Pollee,
        _options: Self::BindOptions,
    ) -> Result<BoundNetlinkRoute> {
        let bound_handle =
            NETLINK_SOCKET_TABLE.bind(StandardNetlinkProtocol::ROUTE as _, endpoint)?;

        Ok(BoundNetlinkRoute::new(bound_handle))
    }

    fn bind_ephemeral(
        &mut self,
        _remote_endpoint: &Self::Endpoint,
        _pollee: &Pollee,
    ) -> Result<Self::Bound> {
        let bound_handle = NETLINK_SOCKET_TABLE.bind(
            StandardNetlinkProtocol::ROUTE as _,
            &NetlinkSocketAddr::new_unspecified(),
        )?;

        Ok(BoundNetlinkRoute::new(bound_handle))
    }

    fn check_io_events(&self) -> IoEvents {
        IoEvents::OUT
    }
}
