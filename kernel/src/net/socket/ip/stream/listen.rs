// SPDX-License-Identifier: MPL-2.0

use aster_bigtcp::{
    errors::tcp::ListenError,
    iface::BindPortConfig,
    socket::{RawTcpSocket, TcpState, UnboundTcpSocket},
    wire::IpEndpoint,
};
use ostd::sync::WriteIrqDisabled;

use super::connected::ConnectedStream;
use crate::{
    events::IoEvents,
    net::iface::{BoundTcpSocket, Iface},
    prelude::*,
};

pub struct ListenStream {
    backlog: usize,
    /// A bound socket held to ensure the TCP port cannot be released
    bound_socket: BoundTcpSocket,
    /// Backlog sockets listening at the local endpoint
    backlog_sockets: RwLock<Vec<BacklogSocket>, WriteIrqDisabled>,
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
            .position(|backlog_socket| backlog_socket.can_accept())
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

    pub fn iface(&self) -> &Arc<Iface> {
        self.bound_socket.iface()
    }

    pub(super) fn check_io_events(&self) -> IoEvents {
        let backlog_sockets = self.backlog_sockets.read();

        let can_accept = backlog_sockets.iter().any(|socket| socket.can_accept());

        // If network packets come in simultaneously, the socket state may change in the middle.
        // However, the current pollee implementation should be able to handle this race condition.
        if can_accept {
            IoEvents::IN
        } else {
            IoEvents::empty()
        }
    }

    /// Calls `f` on each backlog socket that is in listen state.
    ///
    /// This method will also call `f` on the bound socket.
    pub(super) fn for_each_listen_backlog(&self, f: impl Fn(&mut RawTcpSocket)) {
        self.bound_socket
            .raw_with_mut(|raw_tcp_socket| f(raw_tcp_socket));
        self.backlog_sockets.read().iter().for_each(|socket| {
            socket.bound_socket.raw_with_mut(|raw_tcp_socket| {
                if raw_tcp_socket.state() != TcpState::Listen {
                    return;
                }
                f(raw_tcp_socket);
            });
        });
    }

    /// Calls `f` on each backlog socket, no matter which state the socket is in.
    ///
    /// This method will also call `f` on the bound socket.
    pub(super) fn set_keep_alive(&self, interval: Option<aster_bigtcp::time::Duration>) {
        // Disable keep alive on listening socket actually does not take effect.
        if interval.is_none() {
            return;
        }

        self.bound_socket.set_keep_alive(interval);

        self.backlog_sockets
            .read()
            .iter()
            .for_each(|backlog_socket| {
                if backlog_socket
                    .bound_socket
                    .raw_with(|socket| socket.state() != TcpState::Listen)
                {
                    return;
                }
                // If the socket receives SYN after above check,
                // we will also set keep alive on the socket that is not in `Listen` state.
                // But such a race doesn't matter, we just let it happen.
                backlog_socket.bound_socket.set_keep_alive(interval);
            });
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

        let unbound_socket = {
            let mut unbound = UnboundTcpSocket::new(bound_socket.observer());
            unbound.raw_with_mut(|raw_tcp_socket| {
                raw_tcp_socket.set_keep_alive(bound_socket.raw_with(|socket| socket.keep_alive()));
                raw_tcp_socket
                    .set_nagle_enabled(bound_socket.raw_with(|socket| socket.nagle_enabled()));

                // TODO: Inherit other options that can be set via `setsockopt` from bound socket
            });

            Box::new(unbound)
        };
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

    /// Returns whether the backlog socket can be `accept`ed.
    ///
    /// According to the Linux implementation, assuming the TCP Fast Open mechanism is off, a
    /// backlog socket becomes ready to be returned in the `accept` system call when the 3-way
    /// handshake is complete (i.e., when it enters the ESTABLISHED state).
    ///
    /// The Linux kernel implementation can be found at
    /// <https://elixir.bootlin.com/linux/v6.11.8/source/net/ipv4/tcp_input.c#L7304>.
    //
    // FIMXE: Some sockets may be dead (e.g., RSTed), and such sockets can never become alive
    // again. We need to remove them from the backlog sockets.
    fn can_accept(&self) -> bool {
        self.bound_socket.raw_with(|socket| socket.may_send())
    }

    fn remote_endpoint(&self) -> Option<IpEndpoint> {
        self.bound_socket
            .raw_with(|socket| socket.remote_endpoint())
    }

    fn into_bound_socket(self) -> BoundTcpSocket {
        self.bound_socket
    }
}
