// SPDX-License-Identifier: MPL-2.0

use aster_bigtcp::{
    errors::tcp::ListenError, iface::BindPortConfig, socket::UnboundTcpSocket, wire::IpEndpoint,
};

use super::connected::ConnectedStream;
use crate::{events::IoEvents, net::iface::BoundTcpSocket, prelude::*, process::signal::Pollee};

pub struct ListenStream {
    backlog: usize,
    /// A bound socket held to ensure the TCP port cannot be released
    bound_socket: BoundTcpSocket,
    /// Backlog sockets listening at the local endpoint
    backlog_sockets: RwLock<Vec<BacklogSocket>>,
}

impl ListenStream {
    pub fn new(
        bound_socket: BoundTcpSocket,
        backlog: usize,
    ) -> core::result::Result<Self, (Error, BoundTcpSocket)> {
        const SOMAXCONN: usize = 4096;
        let somaxconn = SOMAXCONN.min(backlog);

        let listen_stream = Self {
            backlog: somaxconn,
            bound_socket,
            backlog_sockets: RwLock::new(Vec::new()),
        };
        if let Err(err) = listen_stream.fill_backlog_sockets() {
            return Err((err, listen_stream.bound_socket));
        }
        Ok(listen_stream)
    }

    /// Append sockets listening at LocalEndPoint to support backlog
    fn fill_backlog_sockets(&self) -> Result<()> {
        let mut backlog_sockets = self.backlog_sockets.write();

        let backlog = self.backlog;
        let current_backlog_len = backlog_sockets.len();
        debug_assert!(backlog >= current_backlog_len);
        if backlog == current_backlog_len {
            return Ok(());
        }

        for _ in current_backlog_len..backlog {
            let backlog_socket = BacklogSocket::new(&self.bound_socket)?;
            backlog_sockets.push(backlog_socket);
        }

        Ok(())
    }

    pub fn try_accept(&self) -> Result<ConnectedStream> {
        let mut backlog_sockets = self.backlog_sockets.write();

        let index = backlog_sockets
            .iter()
            .position(|backlog_socket| backlog_socket.is_active())
            .ok_or_else(|| {
                Error::with_message(Errno::EAGAIN, "no pending connection is available")
            })?;
        let active_backlog_socket = backlog_sockets.remove(index);

        if let Ok(backlog_socket) = BacklogSocket::new(&self.bound_socket) {
            backlog_sockets.push(backlog_socket);
        }

        let remote_endpoint = active_backlog_socket.remote_endpoint().unwrap();
        Ok(ConnectedStream::new(
            active_backlog_socket.into_bound_socket(),
            remote_endpoint,
            false,
        ))
    }

    pub fn local_endpoint(&self) -> IpEndpoint {
        self.bound_socket.local_endpoint().unwrap()
    }

    pub(super) fn init_pollee(&self, pollee: &Pollee) {
        pollee.reset_events();
        self.update_io_events(pollee);
    }

    pub(super) fn update_io_events(&self, pollee: &Pollee) {
        // The lock should be held to avoid data races
        let backlog_sockets = self.backlog_sockets.read();

        let can_accept = backlog_sockets.iter().any(|socket| socket.is_active());
        if can_accept {
            pollee.add_events(IoEvents::IN);
        } else {
            pollee.del_events(IoEvents::IN);
        }
    }
}

struct BacklogSocket {
    bound_socket: BoundTcpSocket,
}

impl BacklogSocket {
    // FIXME: All of the error codes below seem to have no Linux equivalents, and I see no reason
    // why the error may occur. Perhaps it is better to call `unwrap()` directly?
    fn new(bound_socket: &BoundTcpSocket) -> Result<Self> {
        let local_endpoint = bound_socket.local_endpoint().ok_or(Error::with_message(
            Errno::EINVAL,
            "the socket is not bound",
        ))?;

        let unbound_socket = Box::new(UnboundTcpSocket::new(bound_socket.observer()));
        let bound_socket = {
            let iface = bound_socket.iface();
            let bind_port_config = BindPortConfig::new(local_endpoint.port, true);
            iface
                .bind_tcp(unbound_socket, bind_port_config)
                .map_err(|(err, _)| err)?
        };

        match bound_socket.listen(local_endpoint) {
            Ok(()) => Ok(Self { bound_socket }),
            Err(ListenError::Unaddressable) => {
                return_errno_with_message!(Errno::EINVAL, "the listening address is invalid")
            }
            Err(ListenError::InvalidState) => {
                return_errno_with_message!(Errno::EINVAL, "the listening socket is invalid")
            }
        }
    }

    fn is_active(&self) -> bool {
        self.bound_socket.raw_with(|socket| socket.is_active())
    }

    fn remote_endpoint(&self) -> Option<IpEndpoint> {
        self.bound_socket
            .raw_with(|socket| socket.remote_endpoint())
    }

    fn into_bound_socket(self) -> BoundTcpSocket {
        self.bound_socket
    }
}
