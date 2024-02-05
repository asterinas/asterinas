// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::{AtomicBool, Ordering};

use crate::events::{IoEvents, Observer};
use crate::net::iface::{AnyUnboundSocket, BindPortConfig, IpEndpoint};

use crate::net::iface::{AnyBoundSocket, RawTcpSocket};
use crate::process::signal::{Pollee, Poller};
use crate::{net::poll_ifaces, prelude::*};

use super::connected::ConnectedStream;

pub struct ListenStream {
    is_nonblocking: AtomicBool,
    backlog: usize,
    /// A bound socket held to ensure the TCP port cannot be released
    bound_socket: Arc<AnyBoundSocket>,
    /// Backlog sockets listening at the local endpoint
    backlog_sockets: RwLock<Vec<BacklogSocket>>,
    pollee: Pollee,
}

impl ListenStream {
    pub fn new(
        nonblocking: bool,
        bound_socket: Arc<AnyBoundSocket>,
        backlog: usize,
        pollee: Pollee,
    ) -> Result<Arc<Self>> {
        let listen_stream = Arc::new(Self {
            is_nonblocking: AtomicBool::new(nonblocking),
            backlog,
            bound_socket,
            backlog_sockets: RwLock::new(Vec::new()),
            pollee,
        });
        listen_stream.fill_backlog_sockets()?;
        listen_stream.pollee.reset_events();
        listen_stream
            .bound_socket
            .set_observer(Arc::downgrade(&listen_stream) as _);
        Ok(listen_stream)
    }

    pub fn accept(&self) -> Result<(Arc<ConnectedStream>, IpEndpoint)> {
        // wait to accept
        let poller = Poller::new();
        loop {
            poll_ifaces();
            let accepted_socket = if let Some(accepted_socket) = self.try_accept() {
                accepted_socket
            } else {
                let events = self.poll(IoEvents::IN, Some(&poller));
                if !events.contains(IoEvents::IN) {
                    if self.is_nonblocking() {
                        return_errno_with_message!(Errno::EAGAIN, "try accept again");
                    }
                    // FIXME: deal with accept timeout
                    poller.wait()?;
                }
                continue;
            };
            let remote_endpoint = accepted_socket.remote_endpoint().unwrap();
            let connected_stream = {
                let BacklogSocket {
                    bound_socket: backlog_socket,
                } = accepted_socket;
                ConnectedStream::new(
                    false,
                    backlog_socket,
                    remote_endpoint,
                    Pollee::new(IoEvents::empty()),
                )
            };
            return Ok((connected_stream, remote_endpoint));
        }
    }

    /// Append sockets listening at LocalEndPoint to support backlog
    fn fill_backlog_sockets(&self) -> Result<()> {
        let backlog = self.backlog;
        let mut backlog_sockets = self.backlog_sockets.write();
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

    fn try_accept(&self) -> Option<BacklogSocket> {
        let backlog_socket = {
            let mut backlog_sockets = self.backlog_sockets.write();
            let index = backlog_sockets
                .iter()
                .position(|backlog_socket| backlog_socket.is_active())?;
            backlog_sockets.remove(index)
        };
        self.fill_backlog_sockets().unwrap();
        self.update_io_events();
        Some(backlog_socket)
    }

    pub fn local_endpoint(&self) -> Result<IpEndpoint> {
        self.bound_socket
            .local_endpoint()
            .ok_or_else(|| Error::with_message(Errno::EINVAL, "does not has remote endpoint"))
    }

    pub fn poll(&self, mask: IoEvents, poller: Option<&Poller>) -> IoEvents {
        self.pollee.poll(mask, poller)
    }

    fn update_io_events(&self) {
        // The lock should be held to avoid data races
        let backlog_sockets = self.backlog_sockets.read();

        let can_accept = backlog_sockets.iter().any(|socket| socket.is_active());
        if can_accept {
            self.pollee.add_events(IoEvents::IN);
        } else {
            self.pollee.del_events(IoEvents::IN);
        }
    }

    pub fn is_nonblocking(&self) -> bool {
        self.is_nonblocking.load(Ordering::Relaxed)
    }

    pub fn set_nonblocking(&self, nonblocking: bool) {
        self.is_nonblocking.store(nonblocking, Ordering::Relaxed);
    }
}

impl Observer<()> for ListenStream {
    fn on_events(&self, _: &()) {
        self.update_io_events();
    }
}

struct BacklogSocket {
    bound_socket: Arc<AnyBoundSocket>,
}

impl BacklogSocket {
    fn new(bound_socket: &Arc<AnyBoundSocket>) -> Result<Self> {
        let local_endpoint = bound_socket.local_endpoint().ok_or(Error::with_message(
            Errno::EINVAL,
            "the socket is not bound",
        ))?;
        let unbound_socket = Box::new(AnyUnboundSocket::new_tcp());
        let bound_socket = {
            let iface = bound_socket.iface();
            let bind_port_config = BindPortConfig::new(local_endpoint.port, true)?;
            iface
                .bind_socket(unbound_socket, bind_port_config)
                .map_err(|(e, _)| e)?
        };
        bound_socket.raw_with(|raw_tcp_socket: &mut RawTcpSocket| {
            raw_tcp_socket
                .listen(local_endpoint)
                .map_err(|_| Error::with_message(Errno::EINVAL, "fail to listen"))
        })?;
        Ok(Self { bound_socket })
    }

    fn is_active(&self) -> bool {
        self.bound_socket
            .raw_with(|socket: &mut RawTcpSocket| socket.is_active())
    }

    fn remote_endpoint(&self) -> Option<IpEndpoint> {
        self.bound_socket
            .raw_with(|socket: &mut RawTcpSocket| socket.remote_endpoint())
    }
}
