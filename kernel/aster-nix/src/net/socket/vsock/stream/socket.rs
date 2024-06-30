// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::AtomicBool;

use atomic::Ordering;

use super::{connected::Connected, connecting::Connecting, init::Init, listen::Listen};
use crate::{
    events::IoEvents,
    fs::{file_handle::FileLike, utils::StatusFlags},
    net::socket::{
        util::{copy_message_from_user, copy_message_to_user, create_message_buffer},
        vsock::{addr::VsockSocketAddr, VSOCK_GLOBAL},
        MessageHeader, SendRecvFlags, SockShutdownCmd, Socket, SocketAddr,
    },
    prelude::*,
    process::signal::{Pollable, Poller},
    util::IoVec,
};

pub struct VsockStreamSocket {
    status: RwLock<Status>,
    is_nonblocking: AtomicBool,
}

pub enum Status {
    Init(Arc<Init>),
    Listen(Arc<Listen>),
    Connected(Arc<Connected>),
}

impl VsockStreamSocket {
    pub fn new(nonblocking: bool) -> Self {
        let init = Arc::new(Init::new());
        Self {
            status: RwLock::new(Status::Init(init)),
            is_nonblocking: AtomicBool::new(nonblocking),
        }
    }

    pub(super) fn new_from_connected(connected: Arc<Connected>) -> Self {
        Self {
            status: RwLock::new(Status::Connected(connected)),
            is_nonblocking: AtomicBool::new(false),
        }
    }

    fn is_nonblocking(&self) -> bool {
        self.is_nonblocking.load(Ordering::Relaxed)
    }

    fn set_nonblocking(&self, nonblocking: bool) {
        self.is_nonblocking.store(nonblocking, Ordering::Relaxed);
    }

    fn try_accept(&self) -> Result<(Arc<dyn FileLike>, SocketAddr)> {
        let listen = match &*self.status.read() {
            Status::Listen(listen) => listen.clone(),
            Status::Init(_) | Status::Connected(_) => {
                return_errno_with_message!(Errno::EINVAL, "the socket is not listening");
            }
        };

        let connected = listen.try_accept()?;
        listen.update_io_events();

        let peer_addr = connected.peer_addr();

        VSOCK_GLOBAL
            .get()
            .unwrap()
            .insert_connected_socket(connected.id(), connected.clone());

        VSOCK_GLOBAL
            .get()
            .unwrap()
            .response(&connected.get_info())
            .unwrap();

        let socket = Arc::new(VsockStreamSocket::new_from_connected(connected));
        Ok((socket, peer_addr.into()))
    }

    fn send(&self, buf: &[u8], flags: SendRecvFlags) -> Result<usize> {
        let inner = self.status.read();
        match &*inner {
            Status::Connected(connected) => connected.send(buf, flags),
            Status::Init(_) | Status::Listen(_) => {
                return_errno_with_message!(Errno::EINVAL, "the socket is not connected");
            }
        }
    }

    fn try_recv(&self, buf: &mut [u8], _flags: SendRecvFlags) -> Result<(usize, SocketAddr)> {
        let connected = match &*self.status.read() {
            Status::Connected(connected) => connected.clone(),
            Status::Init(_) | Status::Listen(_) => {
                return_errno_with_message!(Errno::EINVAL, "the socket is not connected");
            }
        };

        let read_size = connected.try_recv(buf)?;
        connected.update_io_events();

        let peer_addr = self.peer_addr()?;
        // If buffer is now empty and the peer requested shutdown, finish shutting down the
        // connection.
        if connected.should_close() {
            if let Err(e) = self.shutdown(SockShutdownCmd::SHUT_RDWR) {
                debug!("The error is {:?}", e);
            }
        }
        Ok((read_size, peer_addr))
    }

    fn recv(&self, buf: &mut [u8], flags: SendRecvFlags) -> Result<(usize, SocketAddr)> {
        if self.is_nonblocking() {
            self.try_recv(buf, flags)
        } else {
            self.wait_events(IoEvents::IN, || self.try_recv(buf, flags))
        }
    }
}

impl Pollable for VsockStreamSocket {
    fn poll(&self, mask: IoEvents, poller: Option<&Poller>) -> IoEvents {
        match &*self.status.read() {
            Status::Init(init) => init.poll(mask, poller),
            Status::Listen(listen) => listen.poll(mask, poller),
            Status::Connected(connected) => connected.poll(mask, poller),
        }
    }
}

impl FileLike for VsockStreamSocket {
    fn as_socket(self: Arc<Self>) -> Option<Arc<dyn Socket>> {
        Some(self)
    }

    fn read(&self, buf: &mut [u8]) -> Result<usize> {
        // TODO: Set correct flags
        self.recv(buf, SendRecvFlags::empty()).map(|(len, _)| len)
    }

    fn write(&self, buf: &[u8]) -> Result<usize> {
        // TODO: Set correct flags
        self.send(buf, SendRecvFlags::empty())
    }

    fn status_flags(&self) -> StatusFlags {
        if self.is_nonblocking() {
            StatusFlags::O_NONBLOCK
        } else {
            StatusFlags::empty()
        }
    }

    fn set_status_flags(&self, new_flags: StatusFlags) -> Result<()> {
        if new_flags.contains(StatusFlags::O_NONBLOCK) {
            self.set_nonblocking(true);
        } else {
            self.set_nonblocking(false);
        }
        Ok(())
    }
}

impl Socket for VsockStreamSocket {
    fn bind(&self, sockaddr: SocketAddr) -> Result<()> {
        let addr = VsockSocketAddr::try_from(sockaddr)?;
        let inner = self.status.read();
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

    // Since blocking mode is supported, there is no need to store the connecting status.
    // TODO: Refactor when nonblocking mode is supported.
    fn connect(&self, sockaddr: SocketAddr) -> Result<()> {
        let init = match &*self.status.read() {
            Status::Init(init) => init.clone(),
            Status::Listen(_) => {
                return_errno_with_message!(Errno::EINVAL, "the socket is listened");
            }
            Status::Connected(_) => {
                return_errno_with_message!(Errno::EINVAL, "the socket is connected");
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
        vsockspace.insert_connecting_socket(connecting.local_addr(), connecting.clone());

        // Send request
        vsockspace.request(&connecting.info()).unwrap();
        // wait for response from driver
        // TODO: Add timeout
        let poller = Poller::new();
        if !connecting
            .poll(IoEvents::IN, Some(&poller))
            .contains(IoEvents::IN)
        {
            if let Err(e) = poller.wait() {
                vsockspace
                    .remove_connecting_socket(&connecting.local_addr())
                    .unwrap();
                return Err(e);
            }
        }

        vsockspace
            .remove_connecting_socket(&connecting.local_addr())
            .unwrap();
        let connected = Arc::new(Connected::from_connecting(connecting));
        *self.status.write() = Status::Connected(connected.clone());
        // move connecting socket map to connected sockmap
        vsockspace.insert_connected_socket(connected.id(), connected);

        Ok(())
    }

    fn listen(&self, backlog: usize) -> Result<()> {
        let init = match &*self.status.read() {
            Status::Init(init) => init.clone(),
            Status::Listen(_) => {
                return_errno_with_message!(Errno::EINVAL, "the socket is already listened");
            }
            Status::Connected(_) => {
                return_errno_with_message!(Errno::EISCONN, "the socket is already connected");
            }
        };
        let addr = init.bound_addr().ok_or(Error::with_message(
            Errno::EINVAL,
            "the socket is not bound",
        ))?;
        let listen = Arc::new(Listen::new(addr, backlog));
        *self.status.write() = Status::Listen(listen.clone());

        // push listen socket into vsockspace
        VSOCK_GLOBAL
            .get()
            .unwrap()
            .insert_listen_socket(listen.addr(), listen);

        Ok(())
    }

    fn accept(&self) -> Result<(Arc<dyn FileLike>, SocketAddr)> {
        if self.is_nonblocking() {
            self.try_accept()
        } else {
            self.wait_events(IoEvents::IN, || self.try_accept())
        }
    }

    fn shutdown(&self, cmd: SockShutdownCmd) -> Result<()> {
        match &*self.status.read() {
            Status::Connected(connected) => connected.shutdown(cmd),
            Status::Init(_) | Status::Listen(_) => {
                return_errno_with_message!(Errno::EINVAL, "the socket is not connected");
            }
        }
    }

    fn sendmsg(
        &self,
        io_vecs: &[IoVec],
        message_header: MessageHeader,
        flags: SendRecvFlags,
    ) -> Result<usize> {
        // TODO: Deal with flags
        debug_assert!(flags.is_all_supported());

        let MessageHeader {
            control_message, ..
        } = message_header;

        if control_message.is_some() {
            // TODO: Support sending control message
            warn!("sending control message is not supported");
        }

        let buf = copy_message_from_user(io_vecs);
        self.send(&buf, flags)
    }

    fn recvmsg(&self, io_vecs: &[IoVec], flags: SendRecvFlags) -> Result<(usize, MessageHeader)> {
        // TODO: Deal with flags
        debug_assert!(flags.is_all_supported());

        let mut buf = create_message_buffer(io_vecs);

        let (received_bytes, _) = self.recv(&mut buf, flags)?;

        let copied_bytes = {
            let message = &buf[..received_bytes];
            copy_message_to_user(io_vecs, message)
        };

        // TODO: Receive control message

        let messsge_header = MessageHeader::new(None, None);

        Ok((copied_bytes, messsge_header))
    }

    fn addr(&self) -> Result<SocketAddr> {
        let inner = self.status.read();
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
        let inner = self.status.read();
        if let Status::Connected(connected) = &*inner {
            Ok(connected.peer_addr().into())
        } else {
            return_errno_with_message!(Errno::EINVAL, "the socket is not connected");
        }
    }
}

impl Drop for VsockStreamSocket {
    fn drop(&mut self) {
        let vsockspace = VSOCK_GLOBAL.get().unwrap();
        let inner = self.status.read();
        match &*inner {
            Status::Init(init) => {
                if let Some(addr) = init.bound_addr() {
                    vsockspace.recycle_port(&addr.port);
                }
            }
            Status::Listen(listen) => {
                vsockspace.recycle_port(&listen.addr().port);
                vsockspace.remove_listen_socket(&listen.addr());
            }
            Status::Connected(connected) => {
                if !connected.is_closed() {
                    vsockspace.reset(&connected.get_info()).unwrap();
                }
                vsockspace.remove_connected_socket(&connected.id());
                vsockspace.recycle_port(&connected.local_addr().port);
            }
        }
    }
}
