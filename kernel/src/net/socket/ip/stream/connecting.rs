// SPDX-License-Identifier: MPL-2.0

use aster_bigtcp::{socket::ConnectState, wire::IpEndpoint};

use super::{connected::ConnectedStream, init::InitStream};
use crate::{
    events::IoEvents,
    net::iface::{BoundTcpSocket, Iface},
    prelude::*,
};

pub struct ConnectingStream {
    bound_socket: BoundTcpSocket,
    remote_endpoint: IpEndpoint,
}

pub enum ConnResult {
    Connecting(ConnectingStream),
    Connected(ConnectedStream),
    Refused(InitStream),
}

impl ConnectingStream {
    pub fn new(
        bound_socket: BoundTcpSocket,
        remote_endpoint: IpEndpoint,
    ) -> core::result::Result<Self, (Error, BoundTcpSocket)> {
        // The only reason this method might fail is because we're trying to connect to an
        // unspecified address (i.e. 0.0.0.0). We currently have no support for binding to,
        // listening on, or connecting to the unspecified address.
        //
        // We assume the remote will just refuse to connect, so we return `ECONNREFUSED`.
        if bound_socket.connect(remote_endpoint).is_err() {
            return Err((
                Error::with_message(
                    Errno::ECONNREFUSED,
                    "connecting to an unspecified address is not supported",
                ),
                bound_socket,
            ));
        }

        Ok(Self {
            bound_socket,
            remote_endpoint,
        })
    }

    pub fn has_result(&self) -> bool {
        match self.bound_socket.connect_state() {
            ConnectState::Connecting => false,
            ConnectState::Connected => true,
            ConnectState::Refused => true,
        }
    }

    pub fn into_result(self) -> ConnResult {
        let next_state = self.bound_socket.connect_state();

        match next_state {
            ConnectState::Connecting => ConnResult::Connecting(self),
            ConnectState::Connected => ConnResult::Connected(ConnectedStream::new(
                self.bound_socket,
                self.remote_endpoint,
                true,
            )),
            ConnectState::Refused => ConnResult::Refused(InitStream::new_bound(self.bound_socket)),
        }
    }

    pub fn local_endpoint(&self) -> IpEndpoint {
        self.bound_socket.local_endpoint().unwrap()
    }

    pub fn remote_endpoint(&self) -> IpEndpoint {
        self.remote_endpoint
    }

    pub fn iface(&self) -> &Arc<Iface> {
        self.bound_socket.iface()
    }

    pub(super) fn check_io_events(&self) -> IoEvents {
        IoEvents::empty()
    }
}
