use core::sync::atomic::{AtomicBool, Ordering};

use crate::net::iface::{AnyUnboundSocket, BindPortConfig, IpEndpoint};

use crate::fs::utils::{IoEvents, Poller};
use crate::net::iface::{AnyBoundSocket, RawTcpSocket};
use crate::{net::poll_ifaces, prelude::*};

use super::connected::ConnectedStream;

pub struct ListenStream {
    nonblocking: AtomicBool,
    backlog: usize,
    /// Sockets also listening at LocalEndPoint when called `listen`
    backlog_sockets: RwLock<Vec<BacklogSocket>>,
}

impl ListenStream {
    pub fn new(
        nonblocking: bool,
        bound_socket: Arc<AnyBoundSocket>,
        backlog: usize,
    ) -> Result<Self> {
        debug_assert!(backlog >= 1);
        let backlog_socket = BacklogSocket::new(&bound_socket)?;
        let listen_stream = Self {
            nonblocking: AtomicBool::new(nonblocking),
            backlog,
            backlog_sockets: RwLock::new(vec![backlog_socket]),
        };
        listen_stream.fill_backlog_sockets()?;
        Ok(listen_stream)
    }

    pub fn accept(&self) -> Result<(ConnectedStream, IpEndpoint)> {
        // wait to accept
        let poller = Poller::new();
        loop {
            poll_ifaces();
            let accepted_socket = if let Some(accepted_socket) = self.try_accept() {
                accepted_socket
            } else {
                let events = self.poll(IoEvents::IN | IoEvents::OUT, Some(&poller));
                if !events.contains(IoEvents::IN) && !events.contains(IoEvents::OUT) {
                    poller.wait();
                }
                continue;
            };
            let remote_endpoint = accepted_socket.remote_endpoint().unwrap();
            let connected_stream = {
                let BacklogSocket {
                    bound_socket: backlog_socket,
                } = accepted_socket;
                let nonblocking = self.nonblocking();
                ConnectedStream::new(nonblocking, backlog_socket, remote_endpoint)
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
        let bound_socket = backlog_sockets[0].bound_socket.clone();
        for _ in current_backlog_len..backlog {
            let backlog_socket = BacklogSocket::new(&bound_socket)?;
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
        Some(backlog_socket)
    }

    pub fn local_endpoint(&self) -> Result<IpEndpoint> {
        self.bound_socket()
            .local_endpoint()
            .ok_or_else(|| Error::with_message(Errno::EINVAL, "does not has remote endpoint"))
    }

    pub fn poll(&self, mask: IoEvents, poller: Option<&Poller>) -> IoEvents {
        let backlog_sockets = self.backlog_sockets.read();
        for backlog_socket in backlog_sockets.iter() {
            if backlog_socket.is_active() {
                return IoEvents::IN;
            } else {
                // regiser poller to the backlog socket
                backlog_socket.poll(mask, poller);
            }
        }
        return IoEvents::empty();
    }

    fn bound_socket(&self) -> Arc<AnyBoundSocket> {
        self.backlog_sockets.read()[0].bound_socket.clone()
    }

    pub fn nonblocking(&self) -> bool {
        self.nonblocking.load(Ordering::SeqCst)
    }

    pub fn set_nonblocking(&self, nonblocking: bool) {
        self.nonblocking.store(nonblocking, Ordering::SeqCst);
    }
}

struct BacklogSocket {
    bound_socket: Arc<AnyBoundSocket>,
}

impl BacklogSocket {
    fn new(bound_socket: &AnyBoundSocket) -> Result<Self> {
        let local_endpoint = bound_socket.local_endpoint().ok_or(Error::with_message(
            Errno::EINVAL,
            "the socket is not bound",
        ))?;
        let unbound_socket = AnyUnboundSocket::new_tcp();
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
        bound_socket.update_socket_state();
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

    fn poll(&self, mask: IoEvents, poller: Option<&Poller>) -> IoEvents {
        self.bound_socket.poll(mask, poller)
    }
}
