// SPDX-License-Identifier: MPL-2.0

use aster_bigtcp::wire::IpEndpoint;
use ostd::sync::LocalIrqDisabled;

use super::{connected::ConnectedStream, init::InitStream};
use crate::{events::IoEvents, net::iface::BoundTcpSocket, prelude::*, process::signal::Pollee};

pub struct ConnectingStream {
    bound_socket: BoundTcpSocket,
    remote_endpoint: IpEndpoint,
    conn_result: SpinLock<Option<ConnResult>, LocalIrqDisabled>,
}

#[derive(Clone, Copy)]
enum ConnResult {
    Connected,
    Refused,
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
            conn_result: SpinLock::new(None),
        })
    }

    pub fn has_result(&self) -> bool {
        self.conn_result.lock_with(|r| r.is_some())
    }

    pub fn into_result(self) -> core::result::Result<ConnectedStream, (Error, InitStream)> {
        self.conn_result
            .lock_with(|conn_result| match *conn_result {
                Some(ConnResult::Connected) => Ok(ConnectedStream::new(
                    self.bound_socket,
                    self.remote_endpoint,
                    true,
                )),
                Some(ConnResult::Refused) => Err((
                    Error::with_message(Errno::ECONNREFUSED, "the connection is refused"),
                    InitStream::new_bound(self.bound_socket),
                )),
                None => unreachable!("`has_result` must be true before calling `into_result`"),
            })
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

    pub(super) fn update_io_events(&self, pollee: &Pollee) {
        if self.conn_result.lock_with(|r| r.is_some()) {
            return;
        }

        self.bound_socket.raw_with(|socket| {
            self.conn_result.lock_with(|result| {
                if result.is_some() {
                    return;
                }

                // Connected
                if socket.can_send() {
                    *result = Some(ConnResult::Connected);
                    pollee.add_events(IoEvents::OUT);
                    return;
                }
                // Connecting
                if socket.is_open() {
                    return;
                }
                // Refused
                *result = Some(ConnResult::Refused);
                pollee.add_events(IoEvents::OUT);

                // Add `IoEvents::OUT` because the man pages say "EINPROGRESS [..] It is possible to
                // select(2) or poll(2) for completion by selecting the socket for writing". For
                // details, see <https://man7.org/linux/man-pages/man2/connect.2.html>.
                //
                // TODO: It is better to do the state transition and let `ConnectedStream` or
                // `InitStream` set the correct I/O events. However, the state transition is delayed
                // because we're probably in IRQ handlers. Maybe mark the `pollee` as obsolete and
                // re-calculate the I/O events in `poll`.
            });
        })
    }
}
