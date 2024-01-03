// SPDX-License-Identifier: MPL-2.0

use super::endpoint::Endpoint;
use crate::events::IoEvents;
use crate::net::socket::{unix::addr::UnixSocketAddrBound, SockShutdownCmd};
use crate::prelude::*;
use crate::process::signal::Poller;

pub(super) struct Connected {
    local_endpoint: Arc<Endpoint>,
}

impl Connected {
    pub(super) fn new(local_endpoint: Arc<Endpoint>) -> Self {
        Connected { local_endpoint }
    }

    pub(super) fn addr(&self) -> Option<UnixSocketAddrBound> {
        self.local_endpoint.addr()
    }

    pub(super) fn peer_addr(&self) -> Option<UnixSocketAddrBound> {
        self.local_endpoint.peer_addr()
    }

    pub(super) fn is_bound(&self) -> bool {
        self.addr().is_some()
    }

    pub(super) fn write(&self, buf: &[u8]) -> Result<usize> {
        self.local_endpoint.write(buf)
    }

    pub(super) fn read(&self, buf: &mut [u8]) -> Result<usize> {
        self.local_endpoint.read(buf)
    }

    pub(super) fn shutdown(&self, cmd: SockShutdownCmd) -> Result<()> {
        self.local_endpoint.shutdown(cmd)
    }

    pub(super) fn is_nonblocking(&self) -> bool {
        self.local_endpoint.is_nonblocking()
    }

    pub(super) fn set_nonblocking(&self, is_nonblocking: bool) {
        self.local_endpoint.set_nonblocking(is_nonblocking).unwrap();
    }

    pub(super) fn poll(&self, mask: IoEvents, poller: Option<&Poller>) -> IoEvents {
        self.local_endpoint.poll(mask, poller)
    }
}
