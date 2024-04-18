// SPDX-License-Identifier: MPL-2.0

use super::{connected::Connected, connecting::Connecting, init::Init, listen::Listen};
use crate::{
    events::IoEvents,
    fs::file_handle::FileLike,
    net::socket::{
        vsock::{addr::VsockSocketAddr, VSOCK_GLOBAL},
        SendRecvFlags, SockShutdownCmd, Socket, SocketAddr,
    },
    prelude::*,
    process::signal::Poller,
};

pub struct VsockStreamSocket(RwLock<Status>);

impl VsockStreamSocket {
    pub(super) fn new_from_init(init: Arc<Init>) -> Self {
        Self(RwLock::new(Status::Init(init)))
    }

    pub(super) fn new_from_listen(listen: Arc<Listen>) -> Self {
        Self(RwLock::new(Status::Listen(listen)))
    }

    pub(super) fn new_from_connected(connected: Arc<Connected>) -> Self {
        Self(RwLock::new(Status::Connected(connected)))
    }
}

pub enum Status {
    Init(Arc<Init>),
    Listen(Arc<Listen>),
    Connected(Arc<Connected>),
}

impl VsockStreamSocket {
    pub fn new() -> Self {
        let init = Arc::new(Init::new());
        Self(RwLock::new(Status::Init(init)))
    }
}

impl FileLike for VsockStreamSocket {
    fn as_socket(self: Arc<Self>) -> Option<Arc<dyn Socket>> {
        Some(self)
    }

    fn read(&self, buf: &mut [u8]) -> Result<usize> {
        self.recvfrom(buf, SendRecvFlags::empty())
            .map(|(read_size, _)| read_size)
    }

    fn write(&self, buf: &[u8]) -> Result<usize> {
        self.sendto(buf, None, SendRecvFlags::empty())
    }

    fn poll(&self, mask: IoEvents, poller: Option<&Poller>) -> IoEvents {
        let inner = self.0.read();
        match &*inner {
            Status::Init(init) => init.poll(mask, poller),
            Status::Listen(listen) => listen.poll(mask, poller),
            Status::Connected(connect) => connect.poll(mask, poller),
        }
    }
}

impl Socket for VsockStreamSocket {
    fn bind(&self, sockaddr: SocketAddr) -> Result<()> {
        let addr = VsockSocketAddr::try_from(sockaddr)?;
        let inner = self.0.read();
        match &*inner {
            Status::Init(init) => init.bind(addr),
            Status::Listen(_) | Status::Connected(_) => {
                return_errno_with_message!(
                    Errno::EINVAL,
                    "cannot bind a listening or connected socket"
                )
            }
        }
    }

    fn connect(&self, sockaddr: SocketAddr) -> Result<()> {
        let init = match &*self.0.read() {
            Status::Init(init) => init.clone(),
            Status::Listen(_) => {
                return_errno_with_message!(Errno::EINVAL, "The socket is listened");
            }
            Status::Connected(_) => {
                return_errno_with_message!(Errno::EINVAL, "The socket is connected");
            }
        };
        let remote_addr = VsockSocketAddr::try_from(sockaddr)?;
        let local_addr = init.bound_addr();

        if let Some(addr) = local_addr {
            if addr == remote_addr {
                return_errno_with_message!(Errno::EINVAL, "try to connect to self is invalid");
            }
        } else {
            init.bind(VsockSocketAddr::any_addr())?;
        }

        let connecting = Arc::new(Connecting::new(remote_addr, init.bound_addr().unwrap()));
        let vsockspace = VSOCK_GLOBAL.get().unwrap();
        vsockspace
            .connecting_sockets
            .lock_irq_disabled()
            .insert(connecting.local_addr(), connecting.clone());

        // Send request
        vsockspace
            .driver
            .lock_irq_disabled()
            .request(&connecting.info())
            .map_err(|e| Error::with_message(Errno::EAGAIN, "can not send connect packet"))?;

        // wait for response from driver
        // TODO: add timeout
        let poller = Poller::new();
        if !connecting
            .poll(IoEvents::IN, Some(&poller))
            .contains(IoEvents::IN)
        {
            poller.wait()?;
        }
        vsockspace
            .connecting_sockets
            .lock_irq_disabled()
            .remove(&connecting.local_addr())
            .unwrap();

        let connected = Arc::new(Connected::from_connecting(connecting));
        *self.0.write() = Status::Connected(connected.clone());
        // move connecting socket map to connected sockmap
        vsockspace
            .connected_sockets
            .lock_irq_disabled()
            .insert(connected.id(), connected);

        Ok(())
    }

    fn listen(&self, backlog: usize) -> Result<()> {
        let init = match &*self.0.read() {
            Status::Init(init) => init.clone(),
            Status::Listen(_) => {
                return_errno_with_message!(Errno::EINVAL, "The socket is already listened");
            }
            Status::Connected(_) => {
                return_errno_with_message!(Errno::EISCONN, "The socket is already connected");
            }
        };
        let addr = init.bound_addr().ok_or(Error::with_message(
            Errno::EINVAL,
            "The socket is not bound",
        ))?;
        let listen = Arc::new(Listen::new(addr, backlog));
        *self.0.write() = Status::Listen(listen.clone());

        // push listen socket into vsockspace
        VSOCK_GLOBAL
            .get()
            .unwrap()
            .listen_sockets
            .lock_irq_disabled()
            .insert(listen.addr(), listen);

        Ok(())
    }

    fn accept(&self) -> Result<(Arc<dyn FileLike>, SocketAddr)> {
        let listen = match &*self.0.read() {
            Status::Listen(listen) => listen.clone(),
            Status::Init(_) | Status::Connected(_) => {
                return_errno_with_message!(Errno::EINVAL, "The socket is not listening");
            }
        };
        let connected = listen.accept()?;
        let peer_addr = connected.peer_addr();

        VSOCK_GLOBAL
            .get()
            .unwrap()
            .connected_sockets
            .lock_irq_disabled()
            .insert(connected.id(), connected.clone());

        VSOCK_GLOBAL
            .get()
            .unwrap()
            .driver
            .lock_irq_disabled()
            .response(&connected.get_info())
            .map_err(|e| Error::with_message(Errno::EAGAIN, "can not send response packet"))?;

        let socket = Arc::new(VsockStreamSocket::new_from_connected(connected));
        Ok((socket, peer_addr.into()))
    }

    fn shutdown(&self, cmd: SockShutdownCmd) -> Result<()> {
        let inner = self.0.read();
        if let Status::Connected(connected) = &*inner {
            let result = connected.shutdown(cmd);
            if result.is_ok() {
                let vsockspace = VSOCK_GLOBAL.get().unwrap();
                vsockspace
                    .used_ports
                    .lock_irq_disabled()
                    .remove(&connected.local_addr().port);
                vsockspace
                    .connected_sockets
                    .lock_irq_disabled()
                    .remove(&connected.id());
            }
            result
        } else {
            return_errno_with_message!(Errno::EINVAL, "The socket is not connected.");
        }
    }

    fn recvfrom(&self, buf: &mut [u8], flags: SendRecvFlags) -> Result<(usize, SocketAddr)> {
        let connected = match &*self.0.read() {
            Status::Connected(connected) => connected.clone(),
            Status::Init(_) | Status::Listen(_) => {
                return_errno_with_message!(Errno::EINVAL, "the socket is not connected");
            }
        };
        let read_size = connected.recv(buf)?;
        let peer_addr = self.peer_addr()?;
        // If buffer is now empty and the peer requested shutdown, finish shutting down the
        // connection.
        if connected.should_close() {
            VSOCK_GLOBAL
                .get()
                .unwrap()
                .driver
                .lock_irq_disabled()
                .reset(&connected.get_info())
                .map_err(|e| Error::with_message(Errno::EAGAIN, "can not send close packet"))?;
        }
        Ok((read_size, peer_addr))
    }

    fn sendto(
        &self,
        buf: &[u8],
        remote: Option<SocketAddr>,
        flags: SendRecvFlags,
    ) -> Result<usize> {
        debug_assert!(remote.is_none());
        if remote.is_some() {
            return_errno_with_message!(Errno::EINVAL, "vsock should not provide remote addr");
        }
        let inner = self.0.read();
        match &*inner {
            Status::Connected(connected) => connected.send(buf, flags),
            Status::Init(_) | Status::Listen(_) => {
                return_errno_with_message!(Errno::EINVAL, "The socket is not connected");
            }
        }
    }

    fn addr(&self) -> Result<SocketAddr> {
        let inner = self.0.read();
        let addr = match &*inner {
            Status::Init(init) => init.bound_addr(),
            Status::Listen(listen) => Some(listen.addr()),
            Status::Connected(connected) => Some(connected.local_addr()),
        };
        addr.map(Into::<SocketAddr>::into)
            .ok_or(Error::with_message(
                Errno::EINVAL,
                "The socket does not bind to addr",
            ))
    }

    fn peer_addr(&self) -> Result<SocketAddr> {
        let inner = self.0.read();
        if let Status::Connected(connected) = &*inner {
            Ok(connected.peer_addr().into())
        } else {
            return_errno_with_message!(Errno::EINVAL, "the socket is not connected");
        }
    }
}

impl Default for VsockStreamSocket {
    fn default() -> Self {
        Self::new()
    }
}
