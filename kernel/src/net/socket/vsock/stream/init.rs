// SPDX-License-Identifier: MPL-2.0

use crate::{
    events::IoEvents,
    net::socket::{
        util::{SockShutdownCmd, check_port_privilege},
        vsock::{
            addr::{VMADDR_CID_HOST, VMADDR_PORT_ANY, VsockSocketAddr},
            stream::{ConnectingStream, ListenStream},
            transport::BoundPort,
        },
    },
    prelude::*,
    process::signal::Pollee,
};

pub(super) struct InitStream {
    bound_port: Option<BoundPort>,
    /// Indicates if the last `connect()` is considered to be done.
    ///
    /// For vsock, a failed `connect()` attempt is never considered completed, so `is_connect_done`
    /// remains `false` permanently. As a result, the socket becomes unusable and cannot be
    /// connected again.
    ///
    /// This field is still kept for symmetry with IP sockets, where a second `connect()` is
    /// allowed, and to leave room for future support for it.
    is_connect_done: bool,
    last_connect_error: Option<Error>,
}

impl InitStream {
    pub(super) fn new() -> Self {
        Self {
            bound_port: None,
            is_connect_done: true,
            last_connect_error: None,
        }
    }

    pub(super) fn new_bound(bound_port: BoundPort) -> Self {
        Self {
            bound_port: Some(bound_port),
            is_connect_done: true,
            last_connect_error: None,
        }
    }

    pub(super) fn new_connect_failed(bound_port: BoundPort, error: Error) -> Self {
        Self {
            bound_port: Some(bound_port),
            is_connect_done: false,
            last_connect_error: Some(error),
        }
    }

    pub(super) fn bind(&mut self, addr: VsockSocketAddr) -> Result<()> {
        if self.bound_port.is_some() {
            return_errno_with_message!(Errno::EINVAL, "the socket is already bound to an address");
        }

        if let Ok(port) = u16::try_from(addr.port) {
            check_port_privilege(port)?;
        }

        // Linux does not support `SO_REUSEADDR`/`SO_REUSEPORT` for `AF_VSOCK`. Therefore, port
        // binding is always exclusive.
        self.bound_port = Some(BoundPort::new_exclusive(addr)?);
        Ok(())
    }

    pub(super) fn connect(
        self,
        remote_addr: VsockSocketAddr,
        pollee: &Pollee,
    ) -> core::result::Result<ConnectingStream, (Error, Self)> {
        if remote_addr.cid != VMADDR_CID_HOST {
            return Err((
                Error::with_message(Errno::ENETUNREACH, "only the host vsock CID is supported"),
                self,
            ));
        }
        if remote_addr.port == VMADDR_PORT_ANY {
            return Err((
                Error::with_message(Errno::EINVAL, "the vsock port is invalid to connect"),
                self,
            ));
        }

        let bound_port = if let Some(bound_port) = self.bound_port {
            bound_port
        } else {
            match BoundPort::new_ephemeral() {
                Ok(bound_port) => bound_port,
                Err(error) => return Err((error, self)),
            }
        };

        ConnectingStream::new(bound_port, remote_addr, pollee)
            .map_err(|(error, bound_port)| (error, Self::new_bound(bound_port)))
    }

    pub(super) fn is_connect_done(&self) -> bool {
        self.is_connect_done
    }

    pub(super) fn listen(
        self,
        backlog: usize,
        pollee: &Pollee,
    ) -> core::result::Result<ListenStream, (Error, Self)> {
        if !self.is_connect_done {
            return Err((
                Error::with_message(Errno::EINVAL, "a previous connection attempt exists"),
                self,
            ));
        }

        let Some(bound_port) = self.bound_port else {
            return Err((
                Error::with_message(Errno::EINVAL, "listen() without bind() is not implemented"),
                self,
            ));
        };

        ListenStream::new(bound_port, backlog, pollee)
            .map_err(|(error, bound_port)| (error, Self::new_bound(bound_port)))
    }

    pub(super) fn shutdown(&self, _cmd: SockShutdownCmd) -> Result<()> {
        if !self.is_connect_done {
            // There is no need to do anything because the socket is permanently unusable. However,
            // returning `Ok(())` mimics the Linux behavior.
            return Ok(());
        }

        return_errno_with_message!(Errno::ENOTCONN, "the socket is not connected");
    }

    pub(super) fn local_addr(&self) -> Option<VsockSocketAddr> {
        self.bound_port
            .as_ref()
            .map(|bound_port| bound_port.local_addr())
    }

    pub(super) fn test_and_clear_error(&mut self, pollee: &Pollee) -> Option<Error> {
        if let Some(error) = self.last_connect_error.take() {
            pollee.notify(IoEvents::IN | IoEvents::RDHUP | IoEvents::HUP);
            return Some(error);
        }

        None
    }

    pub(super) fn check_io_events(&self) -> IoEvents {
        let mut events = IoEvents::OUT;
        if self.last_connect_error.is_some() {
            events |= IoEvents::ERR;
        } else if !self.is_connect_done {
            // This socket is permanently unusable.
            events = IoEvents::IN | IoEvents::RDHUP | IoEvents::HUP;
        }
        events
    }
}
