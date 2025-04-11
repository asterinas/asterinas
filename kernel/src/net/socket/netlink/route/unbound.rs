// SPDX-License-Identifier: MPL-2.0

use super::bound::BoundNetlinkRoute;
use crate::{
    events::IoEvents,
    net::socket::netlink::{
        table::NETLINK_SOCKET_TABLE, NetlinkSocketAddr, StandardNetlinkProtocol,
    },
    prelude::*,
};

pub(super) struct UnboundNetlinkRoute {
    _private: (),
}

impl UnboundNetlinkRoute {
    pub(super) const fn new() -> Self {
        Self { _private: () }
    }

    pub(super) fn bind(
        self,
        addr: &NetlinkSocketAddr,
    ) -> core::result::Result<BoundNetlinkRoute, (Error, Self)> {
        let bound_handle = NETLINK_SOCKET_TABLE
            .bind(StandardNetlinkProtocol::ROUTE as _, addr)
            .map_err(|err| (err, self))?;

        Ok(BoundNetlinkRoute::new(bound_handle))
    }

    pub(super) fn check_io_events(&self) -> IoEvents {
        IoEvents::OUT
    }
}
