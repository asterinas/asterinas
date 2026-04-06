// SPDX-License-Identifier: MPL-2.0

use crate::{
    events::IoEvents,
    net::socket::vsock::{
        addr::VsockSocketAddr,
        stream::{ConnectedStream, InitStream},
        transport::{BoundPort, ConnectResult, Connection},
    },
    prelude::*,
    process::signal::Pollee,
};

pub(super) struct ConnectingStream {
    connection: Connection,
}

pub(super) enum ConnResult {
    Connecting(ConnectingStream),
    Connected(ConnectedStream),
    Failed(InitStream),
}

impl ConnectingStream {
    pub(super) fn new(
        bound_port: BoundPort,
        remote_addr: VsockSocketAddr,
        pollee: &Pollee,
    ) -> core::result::Result<Self, (Error, BoundPort)> {
        bound_port
            .connect(remote_addr, pollee)
            .map(|connection| Self { connection })
    }

    pub(super) fn local_addr(&self) -> VsockSocketAddr {
        self.connection.local_addr()
    }

    pub(super) fn has_result(&self) -> bool {
        self.connection.has_connect_result()
    }

    pub(super) fn into_result(self) -> ConnResult {
        match self.connection.finish_connect() {
            ConnectResult::Connecting(connection) => ConnResult::Connecting(Self { connection }),
            ConnectResult::Connected(connection) => {
                ConnResult::Connected(ConnectedStream::new(connection, true))
            }
            ConnectResult::Failed(bound_port, error) => {
                ConnResult::Failed(InitStream::new_connect_failed(bound_port, error))
            }
        }
    }

    pub(super) fn check_io_events(&self) -> IoEvents {
        IoEvents::empty()
    }
}
