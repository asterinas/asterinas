// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::{AtomicBool, Ordering};

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

pub struct InitStream {
    bound_port: Option<BoundPort>,
    /// Indicates if we can connect to a new remote.
    ///
    /// For non-blocking sockets, if the first `connect()` returns `EINPROGRESS`, the second
    /// `connect()` will _always_ fail, even if the connection was refused and the socket error was
    /// cleared (via `getsockopt(SOL_SOCKET, SO_ERROR)`).
    ///
    /// This boolean value is used to mimic the above behavior. It is set to false after the
    /// connection is refused, and then set to true on the second `connect()`.
    can_connect_new: bool,
    /// Indicates whether the socket error is `ECONNREFUSED`.
    ///
    /// This boolean value is true after the connection is refused and is set to false on either
    /// `getsockopt(SOL_SOCKET, SO_ERROR)` or the second `connect()`.
    is_conn_refused: AtomicBool,
}

impl InitStream {
    pub fn new() -> Self {
        Self {
            bound_port: None,
            can_connect_new: true,
            is_conn_refused: AtomicBool::new(false),
        }
    }

    pub fn new_bound(bound_port: BoundPort) -> Self {
        Self {
            bound_port: Some(bound_port),
            can_connect_new: true,
            is_conn_refused: AtomicBool::new(false),
        }
    }

    pub fn new_refused(bound_port: BoundPort) -> Self {
        Self {
            bound_port: Some(bound_port),
            can_connect_new: false,
            is_conn_refused: AtomicBool::new(true),
        }
    }

    pub fn bind(&mut self, endpoint: &IpEndpoint, can_reuse: bool) -> Result<()> {
        if self.bound_port.is_some() {
            return_errno_with_message!(Errno::EINVAL, "the socket is already bound to an address");
        }

        self.bound_port = Some(bind_port(endpoint, can_reuse)?);

        Ok(())
    }

    pub fn connect(
        self,
        remote_endpoint: &IpEndpoint,
        option: &RawTcpOption,
        observer: StreamObserver,
    ) -> core::result::Result<ConnectingStream, (Error, Self)> {
        // `check_connect` should be called before calling `connect`.
        debug_assert!(self.can_connect_new);

        let bound_port = if let Some(bound_port) = self.bound_port {
            bound_port
        } else {
            let endpoint = get_ephemeral_endpoint(remote_endpoint);
            match bind_port(&endpoint, false) {
                Ok(bound_port) => bound_port,
                Err(err) => return Err((err, self)),
            }
        };

        ConnectingStream::new(bound_port, *remote_endpoint, option, observer).map_err(
            |(err, bound_port)| {
                if err.error() == Errno::ECONNREFUSED {
                    (err, InitStream::new_refused(bound_port))
                } else {
                    (err, InitStream::new_bound(bound_port))
                }
            },
        )
    }

    pub fn check_connect(&mut self) -> Result<()> {
        if self.can_connect_new {
            return Ok(());
        }

        self.can_connect_new = true;

        let is_conn_refused = self.is_conn_refused.get_mut();
        if *is_conn_refused {
            *is_conn_refused = false;
            return_errno_with_message!(Errno::ECONNREFUSED, "the connection is refused");
        } else {
            return_errno_with_message!(Errno::ECONNABORTED, "the connection is refused");
        }
    }

    pub fn listen(
        self,
        backlog: usize,
        option: &RawTcpOption,
        observer: StreamObserver,
    ) -> core::result::Result<ListenStream, (Error, Self)> {
        if !self.can_connect_new {
            // See the comments of `can_connect_new`.
            // `listen()` is also not allowed until the second `connect()`.
            return Err((
                Error::with_message(Errno::EINVAL, "the connection is refused"),
                self,
            ));
        }

        let Some(bound_port) = self.bound_port else {
            // FIXME: The socket should be bound to INADDR_ANY (i.e., 0.0.0.0) with an ephemeral
            // port. However, INADDR_ANY is not yet supported, so we need to return an error first.
            warn!("listen() without bind() is not implemented");
            return Err((
                Error::with_message(Errno::EINVAL, "listen() without bind() is not implemented"),
                self,
            ));
        };

        match ListenStream::new(bound_port, backlog, option, observer) {
            Ok(listen_stream) => Ok(listen_stream),
            Err((bound_port, error)) => Err((error, Self::new_bound(bound_port))),
        }
    }

    pub fn local_endpoint(&self) -> Option<IpEndpoint> {
        self.bound_port
            .as_ref()
            .map(|bound_port| bound_port.endpoint().unwrap())
    }

    pub(super) fn check_io_events(&self) -> IoEvents {
        // Linux adds OUT and HUP events for a newly created socket
        IoEvents::OUT | IoEvents::HUP
    }

    pub(super) fn test_and_clear_error(&self) -> Option<Error> {
        if self.is_conn_refused.swap(false, Ordering::Relaxed) {
            Some(Error::with_message(
                Errno::ECONNREFUSED,
                "the connection is refused",
            ))
        } else {
            None
        }
    }
}
