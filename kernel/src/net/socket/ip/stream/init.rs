// SPDX-License-Identifier: MPL-2.0

use aster_bigtcp::{socket::RawTcpOption, wire::IpEndpoint};

use super::{connecting::ConnectingStream, listen::ListenStream, StreamObserver};
use crate::{
    events::IoEvents,
    net::{
        iface::BoundPort,
        socket::ip::common::{bind_port, get_ephemeral_endpoint},
    },
    prelude::*,
};

pub enum InitStream {
    Unbound,
    Bound(BoundPort),
}

impl InitStream {
    pub fn new() -> Self {
        InitStream::Unbound
    }

    pub fn new_bound(bound_port: BoundPort) -> Self {
        InitStream::Bound(bound_port)
    }

    pub fn bind(
        self,
        endpoint: &IpEndpoint,
        can_reuse: bool,
    ) -> core::result::Result<BoundPort, (Error, Self)> {
        match self {
            InitStream::Unbound => (),
            InitStream::Bound(bound_socket) => {
                return Err((
                    Error::with_message(Errno::EINVAL, "the socket is already bound to an address"),
                    InitStream::Bound(bound_socket),
                ));
            }
        };

        let bound_port = match bind_port(endpoint, can_reuse) {
            Ok(bound_port) => bound_port,
            Err(err) => return Err((err, Self::Unbound)),
        };

        Ok(bound_port)
    }

    fn bind_to_ephemeral_endpoint(
        self,
        remote_endpoint: &IpEndpoint,
    ) -> core::result::Result<BoundPort, (Error, Self)> {
        let endpoint = get_ephemeral_endpoint(remote_endpoint);
        self.bind(&endpoint, false)
    }

    pub fn connect(
        self,
        remote_endpoint: &IpEndpoint,
        option: &RawTcpOption,
        observer: StreamObserver,
    ) -> core::result::Result<ConnectingStream, (Error, Self)> {
        let bound_port = match self {
            InitStream::Bound(bound_port) => bound_port,
            InitStream::Unbound => self.bind_to_ephemeral_endpoint(remote_endpoint)?,
        };

        ConnectingStream::new(bound_port, *remote_endpoint, option, observer)
            .map_err(|(err, bound_port)| (err, InitStream::Bound(bound_port)))
    }

    pub fn listen(
        self,
        backlog: usize,
        option: &RawTcpOption,
        observer: StreamObserver,
    ) -> core::result::Result<ListenStream, (Error, Self)> {
        let InitStream::Bound(bound_port) = self else {
            // FIXME: The socket should be bound to INADDR_ANY (i.e., 0.0.0.0) with an ephemeral
            // port. However, INADDR_ANY is not yet supported, so we need to return an error first.
            debug_assert!(false, "listen() without bind() is not implemented");
            return Err((
                Error::with_message(Errno::EINVAL, "listen() without bind() is not implemented"),
                self,
            ));
        };

        Ok(ListenStream::new(bound_port, backlog, option, observer))
    }

    pub fn local_endpoint(&self) -> Option<IpEndpoint> {
        match self {
            InitStream::Unbound => None,
            InitStream::Bound(bound_port) => Some(bound_port.endpoint().unwrap()),
        }
    }

    pub(super) fn check_io_events(&self) -> IoEvents {
        // Linux adds OUT and HUP events for a newly created socket
        IoEvents::OUT | IoEvents::HUP
    }
}
