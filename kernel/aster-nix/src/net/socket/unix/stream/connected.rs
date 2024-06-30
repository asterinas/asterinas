// SPDX-License-Identifier: MPL-2.0

use super::endpoint::Endpoint;
use crate::{
    events::IoEvents,
    net::socket::{unix::addr::UnixSocketAddrBound, SockShutdownCmd},
    prelude::*,
    process::signal::Poller,
};

pub(super) struct Connected {
    local_endpoint: Endpoint,
}

impl Connected {
    pub(super) fn new(local_endpoint: Endpoint) -> Self {
        Connected { local_endpoint }
    }

    pub(super) fn addr(&self) -> Option<&UnixSocketAddrBound> {
        self.local_endpoint.addr()
    }

    pub(super) fn peer_addr(&self) -> Option<&UnixSocketAddrBound> {
        self.local_endpoint.peer_addr()
    }

    pub(super) fn try_write(&self, buf: &[u8]) -> Result<usize> {
        self.local_endpoint.try_write(buf)
    }

    pub(super) fn try_read(&self, buf: &mut [u8]) -> Result<usize> {
        self.local_endpoint.try_read(buf)
    }

    pub(super) fn shutdown(&self, cmd: SockShutdownCmd) -> Result<()> {
        self.local_endpoint.shutdown(cmd)
    }

    pub(super) fn poll(&self, mask: IoEvents, poller: Option<&mut Poller>) -> IoEvents {
        self.local_endpoint.poll(mask, poller)
    }
}
