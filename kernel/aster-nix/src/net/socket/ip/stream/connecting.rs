// SPDX-License-Identifier: MPL-2.0

use super::{connected::ConnectedStream, init::InitStream};
use crate::{
    events::IoEvents,
    net::iface::{AnyBoundSocket, IpEndpoint, RawTcpSocket},
    prelude::*,
    process::signal::Pollee,
};

pub struct ConnectingStream {
    bound_socket: Arc<AnyBoundSocket>,
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
        bound_socket: Arc<AnyBoundSocket>,
        remote_endpoint: IpEndpoint,
    ) -> core::result::Result<Self, (Error, Arc<AnyBoundSocket>)> {
        if let Err(err) = bound_socket.do_connect(remote_endpoint) {
            return Err((err, bound_socket));
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
        self.update_io_events(pollee);
    }

    pub(super) fn update_io_events(&self, pollee: &Pollee) {
        if self.conn_result.read().is_some() {
            return;
        }

        let became_writable = self.bound_socket.raw_with(|socket: &mut RawTcpSocket| {
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
        });

        // Either when the connection is established, or when the connection fails, the socket
        // shall indicate that it is writable.
        //
        // TODO: Find a way to turn `ConnectingStream` into `ConnectedStream` or `InitStream`
        // here, so non-blocking `connect()` can work correctly. Meanwhile, the latter should
        // be responsible to initialize all the I/O events including `IoEvents::OUT`, so the
        // following hard-coded event addition can be removed.
        if became_writable {
            pollee.add_events(IoEvents::OUT);
        }
    }
}
