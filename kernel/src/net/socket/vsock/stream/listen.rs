// SPDX-License-Identifier: MPL-2.0

use crate::{
    events::IoEvents,
    net::socket::vsock::{
        addr::VsockSocketAddr,
        stream::ConnectedStream,
        transport::{BoundPort, Listener},
    },
    prelude::*,
    process::signal::Pollee,
};

pub(super) struct ListenStream {
    listener: Listener,
}

impl ListenStream {
    pub(super) fn new(
        bound_port: BoundPort,
        backlog: usize,
        pollee: &Pollee,
    ) -> core::result::Result<Self, (Error, BoundPort)> {
        bound_port
            .listen(backlog, pollee)
            .map(|listener| Self { listener })
    }

    pub(super) fn try_accept(&self) -> Result<ConnectedStream> {
        self.listener
            .try_accept()
            .map(|connection| ConnectedStream::new(connection, false))
    }

    pub(super) fn set_backlog(&self, backlog: usize) {
        self.listener.set_backlog(backlog);
    }

    pub(super) fn local_addr(&self) -> VsockSocketAddr {
        self.listener.local_addr()
    }

    pub(super) fn check_io_events(&self) -> IoEvents {
        self.listener.check_io_events()
    }
}
