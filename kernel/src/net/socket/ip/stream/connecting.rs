// SPDX-License-Identifier: MPL-2.0

use aster_bigtcp::{
    socket::{ConnectState, RawTcpOption, RawTcpSetOption},
    wire::IpEndpoint,
};

use super::{connected::ConnectedStream, init::InitStream, StreamObserver};
use crate::{
    events::IoEvents,
    net::iface::{BoundPort, Iface, TcpConnection},
    prelude::*,
};

pub struct ConnectingStream {
    tcp_conn: TcpConnection,
    remote_endpoint: IpEndpoint,
}

pub enum ConnResult {
    Connecting(ConnectingStream),
    Connected(ConnectedStream),
    Refused(InitStream),
}

impl ConnectingStream {
    pub fn new(
        bound_port: BoundPort,
        remote_endpoint: IpEndpoint,
        option: &RawTcpOption,
        observer: StreamObserver,
    ) -> core::result::Result<Self, (Error, BoundPort)> {
        // The only reason this method might fail is because we're trying to connect to an
        // unspecified address (i.e. 0.0.0.0). We currently have no support for binding to,
        // listening on, or connecting to the unspecified address.
        //
        // We assume the remote will just refuse to connect, so we return `ECONNREFUSED`.
        let tcp_conn =
            match TcpConnection::new_connect(bound_port, remote_endpoint, option, observer) {
                Ok(tcp_conn) => tcp_conn,
                Err((bound_port, _)) => {
                    return Err((
                        Error::with_message(
                            Errno::ECONNREFUSED,
                            "connecting to an unspecified address is not supported",
                        ),
                        bound_port,
                    ))
                }
            };

        Ok(Self {
            tcp_conn,
            remote_endpoint,
        })
    }

    pub fn has_result(&self) -> bool {
        match self.tcp_conn.connect_state() {
            ConnectState::Connecting => false,
            ConnectState::Connected => true,
            ConnectState::Refused => true,
        }
    }

    pub fn into_result(self) -> ConnResult {
        let next_state = self.tcp_conn.connect_state();

        match next_state {
            ConnectState::Connecting => ConnResult::Connecting(self),
            ConnectState::Connected => ConnResult::Connected(ConnectedStream::new(
                self.tcp_conn,
                self.remote_endpoint,
                true,
            )),
            ConnectState::Refused => ConnResult::Refused(InitStream::new_bound(
                self.tcp_conn.into_bound_port().unwrap(),
            )),
        }
    }

    pub fn local_endpoint(&self) -> IpEndpoint {
        self.tcp_conn.local_endpoint().unwrap()
    }

    pub fn remote_endpoint(&self) -> IpEndpoint {
        self.remote_endpoint
    }

    pub fn iface(&self) -> &Arc<Iface> {
        self.tcp_conn.iface()
    }

    pub(super) fn check_io_events(&self) -> IoEvents {
        IoEvents::empty()
    }

    pub(super) fn set_raw_option<R>(
        &self,
        set_option: impl FnOnce(&dyn RawTcpSetOption) -> R,
    ) -> R {
        set_option(&self.tcp_conn)
    }
}
