// SPDX-License-Identifier: MPL-2.0

use aster_bigtcp::wire::IpEndpoint;

use super::{connected::ConnectedStream, init::InitStream};
use crate::{net::iface::BoundTcpSocket, prelude::*, process::signal::Pollee};

pub struct ConnectingStream {
    bound_socket: BoundTcpSocket,
    remote_endpoint: IpEndpoint,
    conn_result: RwLock<Option<ConnResult>>,
}

#[derive(Clone, Copy)]
enum ConnResult {
    Connected,
    Refused,
}

pub enum NonConnectedStream {
    Init(InitStream),
    Connecting(ConnectingStream),
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
            conn_result: RwLock::new(None),
        })
    }

    pub fn into_result(self) -> core::result::Result<ConnectedStream, (Error, NonConnectedStream)> {
        let conn_result = *self.conn_result.read();
        match conn_result {
            Some(ConnResult::Connected) => Ok(ConnectedStream::new(
                self.bound_socket,
                self.remote_endpoint,
                true,
            )),
            Some(ConnResult::Refused) => Err((
                Error::with_message(Errno::ECONNREFUSED, "the connection is refused"),
                NonConnectedStream::Init(InitStream::new_bound(self.bound_socket)),
            )),
            None => Err((
                Error::with_message(Errno::EAGAIN, "the connection is pending"),
                NonConnectedStream::Connecting(self),
            )),
        }
    }

    pub fn local_endpoint(&self) -> IpEndpoint {
        self.bound_socket.local_endpoint().unwrap()
    }

    pub fn remote_endpoint(&self) -> IpEndpoint {
        self.remote_endpoint
    }

    pub(super) fn init_pollee(&self, pollee: &Pollee) {
        pollee.reset_events();
    }

    /// Returns `true` when `conn_result` becomes ready, which indicates that the caller should
    /// invoke the `into_result()` method as soon as possible.
    ///
    /// Since `into_result()` needs to be called only once, this method will return `true`
    /// _exactly_ once. The caller is responsible for not missing this event.
    #[must_use]
    pub(super) fn update_io_events(&self) -> bool {
        if self.conn_result.read().is_some() {
            return false;
        }

        self.bound_socket.raw_with(|socket| {
            let mut result = self.conn_result.write();
            if result.is_some() {
                return false;
            }

            // Connected
            if socket.can_send() {
                *result = Some(ConnResult::Connected);
                return true;
            }
            // Connecting
            if socket.is_open() {
                return false;
            }
            // Refused
            *result = Some(ConnResult::Refused);
            true
        })
    }
}
