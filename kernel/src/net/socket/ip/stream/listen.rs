// SPDX-License-Identifier: MPL-2.0

use aster_bigtcp::{
    socket::{RawTcpOption, RawTcpSetOption},
    wire::IpEndpoint,
};

use super::{connected::ConnectedStream, StreamObserver};
use crate::{
    events::IoEvents,
    net::iface::{BoundPort, Iface, TcpListener},
    prelude::*,
};

pub struct ListenStream {
    tcp_listener: TcpListener,
}

impl ListenStream {
    pub fn new(
        bound_port: BoundPort,
        backlog: usize,
        option: &RawTcpOption,
        observer: StreamObserver,
    ) -> Self {
        const SOMAXCONN: usize = 4096;
        let max_conn = SOMAXCONN.min(backlog);

        let tcp_listener = match TcpListener::new_listen(bound_port, max_conn, option, observer) {
            Ok(tcp_listener) => tcp_listener,
            Err((_, err)) => {
                unreachable!("`new_listen` fails with {:?}, which should not happen", err)
            }
        };

        Self { tcp_listener }
    }

    pub fn try_accept(&self) -> Result<ConnectedStream> {
        let (new_conn, remote_endpoint) = self.tcp_listener.accept().ok_or_else(|| {
            Error::with_message(Errno::EAGAIN, "no pending connection is available")
        })?;

        Ok(ConnectedStream::new(new_conn, remote_endpoint, false))
    }

    pub fn local_endpoint(&self) -> IpEndpoint {
        self.tcp_listener.local_endpoint().unwrap()
    }

    pub fn iface(&self) -> &Arc<Iface> {
        self.tcp_listener.iface()
    }

    pub(super) fn check_io_events(&self) -> IoEvents {
        let can_accept = self.tcp_listener.can_accept();

        // If network packets come in simultaneously, the socket state may change in the middle.
        // However, the current pollee implementation should be able to handle this race condition.
        if can_accept {
            IoEvents::IN
        } else {
            IoEvents::empty()
        }
    }

    pub(super) fn set_raw_option<R>(
        &self,
        set_option: impl FnOnce(&dyn RawTcpSetOption) -> R,
    ) -> R {
        set_option(&self.tcp_listener)
    }
}
