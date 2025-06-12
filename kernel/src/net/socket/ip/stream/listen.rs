// SPDX-License-Identifier: MPL-2.0

use aster_bigtcp::{
    errors::tcp::ListenError,
    socket::{RawTcpOption, RawTcpSetOption},
    wire::IpEndpoint,
};

use super::{connected::ConnectedStream, observer::StreamObserver};
use crate::{
    events::IoEvents,
    net::iface::{BoundPort, Iface, TcpListener},
    prelude::*,
};

pub(super) struct ListenStream {
    tcp_listener: TcpListener,
}

impl ListenStream {
    pub(super) fn new(
        bound_port: BoundPort,
        backlog: usize,
        option: &RawTcpOption,
        observer: StreamObserver,
    ) -> core::result::Result<Self, (BoundPort, Error)> {
        const SOMAXCONN: usize = 4096;
        let max_conn = SOMAXCONN.min(backlog);

        match TcpListener::new_listen(bound_port, max_conn, option, observer) {
            Ok(tcp_listener) => Ok(Self { tcp_listener }),
            Err((bound_port, ListenError::AddressInUse)) => Err((
                bound_port,
                Error::with_message(Errno::EADDRINUSE, "listener key conflicts"),
            )),
            Err((_, err)) => {
                unreachable!("`new_listen` fails with {:?}, which should not happen", err)
            }
        }
    }

    pub(super) fn try_accept(&self) -> Result<ConnectedStream> {
        let (new_conn, remote_endpoint) = self.tcp_listener.accept().ok_or_else(|| {
            Error::with_message(Errno::EAGAIN, "no pending connection is available")
        })?;

        Ok(ConnectedStream::new(new_conn, remote_endpoint, false))
    }

    pub(super) fn local_endpoint(&self) -> IpEndpoint {
        self.tcp_listener.local_endpoint().unwrap()
    }

    pub(super) fn iface(&self) -> &Arc<Iface> {
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

    pub(super) fn into_listener(self) -> TcpListener {
        self.tcp_listener
    }
}
