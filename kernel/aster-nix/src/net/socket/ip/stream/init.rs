// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Weak;

use super::{connecting::ConnectingStream, listen::ListenStream};
use crate::{
    events::Observer,
    net::{
        iface::{AnyBoundSocket, AnyUnboundSocket, IpEndpoint},
        socket::ip::common::{bind_socket, get_ephemeral_endpoint},
    },
    prelude::*,
};

pub enum InitStream {
    Unbound(Box<AnyUnboundSocket>),
    Bound(Arc<AnyBoundSocket>),
}

impl InitStream {
    // FIXME: In Linux we have the `POLLOUT` event for a newly created socket, while calling
    // `write()` on it triggers `SIGPIPE`/`EPIPE`. No documentation found yet, but confirmed by
    // experimentation and Linux source code.
    pub fn new(observer: Weak<dyn Observer<()>>) -> Self {
        InitStream::Unbound(Box::new(AnyUnboundSocket::new_tcp(observer)))
    }

    pub fn new_bound(bound_socket: Arc<AnyBoundSocket>) -> Self {
        InitStream::Bound(bound_socket)
    }

    pub fn bind(
        self,
        endpoint: &IpEndpoint,
    ) -> core::result::Result<Arc<AnyBoundSocket>, (Error, Self)> {
        let unbound_socket = match self {
            InitStream::Unbound(unbound_socket) => unbound_socket,
            InitStream::Bound(bound_socket) => {
                return Err((
                    Error::with_message(Errno::EINVAL, "the socket is already bound to an address"),
                    InitStream::Bound(bound_socket),
                ));
            }
        };
        let bound_socket = match bind_socket(unbound_socket, endpoint, false) {
            Ok(bound_socket) => bound_socket,
            Err((err, unbound_socket)) => return Err((err, InitStream::Unbound(unbound_socket))),
        };
        Ok(bound_socket)
    }

    fn bind_to_ephemeral_endpoint(
        self,
        remote_endpoint: &IpEndpoint,
    ) -> core::result::Result<Arc<AnyBoundSocket>, (Error, Self)> {
        let endpoint = get_ephemeral_endpoint(remote_endpoint);
        self.bind(&endpoint)
    }

    pub fn connect(
        self,
        remote_endpoint: &IpEndpoint,
    ) -> core::result::Result<ConnectingStream, (Error, Self)> {
        let bound_socket = match self {
            InitStream::Bound(bound_socket) => bound_socket,
            InitStream::Unbound(_) => self.bind_to_ephemeral_endpoint(remote_endpoint)?,
        };

        ConnectingStream::new(bound_socket, *remote_endpoint)
            .map_err(|(err, bound_socket)| (err, InitStream::Bound(bound_socket)))
    }

    pub fn listen(self, backlog: usize) -> core::result::Result<ListenStream, (Error, Self)> {
        let InitStream::Bound(bound_socket) = self else {
            return Err((
                Error::with_message(Errno::EINVAL, "cannot listen without bound"),
                self,
            ));
        };

        ListenStream::new(bound_socket, backlog)
            .map_err(|(err, bound_socket)| (err, InitStream::Bound(bound_socket)))
    }

    pub fn local_endpoint(&self) -> Result<IpEndpoint> {
        match self {
            InitStream::Unbound(_) => {
                return_errno_with_message!(Errno::EINVAL, "does not has local endpoint")
            }
            InitStream::Bound(bound_socket) => Ok(bound_socket.local_endpoint().unwrap()),
        }
    }
}
