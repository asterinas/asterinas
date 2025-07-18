// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::{AtomicBool, Ordering};

use super::{connected::Connected, connecting::Connecting, init::Init, listen::Listen};
use crate::{
    events::IoEvents,
    fs::file_handle::FileLike,
    net::socket::{
        private::SocketPrivate,
        util::{MessageHeader, SendRecvFlags, SockShutdownCmd, SocketAddr},
        vsock::{addr::VsockSocketAddr, VSOCK_GLOBAL},
        Socket,
    },
    prelude::*,
    process::signal::{PollHandle, Pollable, Poller},
    util::{MultiRead, MultiWrite},
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
    pub fn new(nonblocking: bool) -> Result<Self> {
        if VSOCK_GLOBAL.get().is_none() {
            return_errno_with_message!(
                Errno::EINVAL,
                "cannot create vsock socket since no vsock device is found"
            );
        }

        let init = Arc::new(Init::new());
        Ok(Self {
            status: RwLock::new(Status::Init(init)),
            is_nonblocking: AtomicBool::new(nonblocking),
        })
    }

    pub(super) fn new_from_connected(connected: Arc<Connected>) -> Self {
        Self {
            status: RwLock::new(Status::Connected(connected)),
            is_nonblocking: AtomicBool::new(false),
        }
    }

    fn try_accept(&self) -> Result<(Arc<dyn FileLike>, SocketAddr)> {
        let listen = match &*self.status.read() {
            Status::Listen(listen) => listen.clone(),
            Status::Init(_) | Status::Connected(_) => {
                return_errno_with_message!(Errno::EINVAL, "the socket is not listening");
            }
        };

        let connected = listen.try_accept()?;

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

    fn send(&self, reader: &mut dyn MultiRead, flags: SendRecvFlags) -> Result<usize> {
        let inner = self.status.read();
        match &*inner {
            Status::Connected(connected) => connected.send(reader, flags),
            Status::Init(_) | Status::Listen(_) => {
                return_errno_with_message!(Errno::EINVAL, "the socket is not connected");
            }
        }
    }

    fn try_recv(
        &self,
        writer: &mut dyn MultiWrite,
        _flags: SendRecvFlags,
    ) -> Result<(usize, SocketAddr)> {
        let connected = match &*self.status.read() {
            Status::Connected(connected) => connected.clone(),
            Status::Init(_) | Status::Listen(_) => {
                return_errno_with_message!(Errno::EINVAL, "the socket is not connected");
            }
        };

        let read_size = connected.try_recv(writer)?;

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
}

impl Pollable for VsockStreamSocket {
    fn poll(&self, mask: IoEvents, poller: Option<&mut PollHandle>) -> IoEvents {
        match &*self.status.read() {
            Status::Init(init) => init.poll(mask, poller),
            Status::Listen(listen) => listen.poll(mask, poller),
            Status::Connected(connected) => connected.poll(mask, poller),
        }
    }
}

impl SocketPrivate for VsockStreamSocket {
    fn is_nonblocking(&self) -> bool {
        self.is_nonblocking.load(Ordering::Relaxed)
    }

    fn set_nonblocking(&self, nonblocking: bool) {
        self.is_nonblocking.store(nonblocking, Ordering::Relaxed);
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
        let mut poller = Poller::new(None);
        if !connecting
            .poll(IoEvents::IN, Some(poller.as_handle_mut()))
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
        self.block_on(IoEvents::IN, || self.try_accept())
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
        reader: &mut dyn MultiRead,
        message_header: MessageHeader,
        flags: SendRecvFlags,
    ) -> Result<usize> {
        // TODO: Deal with flags
        if !flags.is_all_supported() {
            warn!("unsupported flags: {:?}", flags);
        }

        let MessageHeader {
            control_messages, ..
        } = message_header;

        if !control_messages.is_empty() {
            // TODO: Support sending control message
            warn!("sending control message is not supported");
        }

        self.send(reader, flags)
    }

    fn recvmsg(
        &self,
        writer: &mut dyn MultiWrite,
        flags: SendRecvFlags,
    ) -> Result<(usize, MessageHeader)> {
        // TODO: Deal with flags
        if !flags.is_all_supported() {
            warn!("unsupported flags: {:?}", flags);
        }

        let (received_bytes, _) = self.block_on(IoEvents::IN, || self.try_recv(writer, flags))?;

        // TODO: Receive control message

        let messsge_header = MessageHeader::new(None, Vec::new());

        Ok((received_bytes, messsge_header))
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
        let inner = self.status.get_mut();
        match inner {
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
