use alloc::sync::Arc;

use crate::events::IoEvents;
use crate::net::iface::RawTcpSocket;
use crate::prelude::*;

use crate::net::iface::{AnyBoundSocket, IpEndpoint};
use crate::process::signal::Pollee;

use super::connected::ConnectedStream;
use super::init::InitStream;

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

    pub(super) fn reset_io_events(&self, pollee: &Pollee) {
        pollee.del_events(IoEvents::IN);
        pollee.del_events(IoEvents::OUT);
    }

    /// Returns `true` when `conn_result` becomes ready, which indicates that the caller should
    /// the `into_result()` method as soon as possible.
    pub(super) fn update_io_events(&self, pollee: &Pollee) -> bool {
        if self.conn_result.read().is_some() {
            return false;
        }

        self.bound_socket.raw_with(|socket: &mut RawTcpSocket| {
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
