// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::AtomicBool;

use atomic::Ordering;

use super::{
    connected::Connected,
    endpoint::Endpoint,
    init::Init,
    listener::{unregister_backlog, Listener},
};
use crate::{
    events::IoEvents,
    fs::{
        file_handle::FileLike,
        fs_resolver::FsPath,
        path::Dentry,
        utils::{InodeType, StatusFlags},
    },
    net::socket::{
        unix::{addr::UnixSocketAddrBound, UnixSocketAddr},
        util::{
            copy_message_from_user, copy_message_to_user, create_message_buffer,
            send_recv_flags::SendRecvFlags, socket_addr::SocketAddr, MessageHeader,
        },
        SockShutdownCmd, Socket,
    },
    prelude::*,
    process::signal::{Pollable, Poller},
    util::IoVec,
};

pub struct UnixStreamSocket {
    state: RwLock<State>,
    is_nonblocking: AtomicBool,
}

impl UnixStreamSocket {
    pub(super) fn new_init(init: Init, is_nonblocking: bool) -> Arc<Self> {
        Arc::new(Self {
            state: RwLock::new(State::Init(init)),
            is_nonblocking: AtomicBool::new(is_nonblocking),
        })
    }

    pub(super) fn new_connected(connected: Connected, is_nonblocking: bool) -> Arc<Self> {
        Arc::new(Self {
            state: RwLock::new(State::Connected(connected)),
            is_nonblocking: AtomicBool::new(is_nonblocking),
        })
    }
}

enum State {
    Init(Init),
    Listen(Listener),
    Connected(Connected),
}

impl UnixStreamSocket {
    pub fn new(is_nonblocking: bool) -> Arc<Self> {
        Self::new_init(Init::new(), is_nonblocking)
    }

    pub fn new_pair(is_nonblocking: bool) -> (Arc<Self>, Arc<Self>) {
        let (end_a, end_b) = Endpoint::new_pair(None, None);
        (
            Self::new_connected(Connected::new(end_a), is_nonblocking),
            Self::new_connected(Connected::new(end_b), is_nonblocking),
        )
    }

    fn bound_addr(&self) -> Option<UnixSocketAddrBound> {
        let state = self.state.read();
        match &*state {
            State::Init(init) => init.addr(),
            State::Listen(listen) => Some(listen.addr().clone()),
            State::Connected(connected) => connected.addr().cloned(),
        }
    }

    fn send(&self, buf: &[u8], flags: SendRecvFlags) -> Result<usize> {
        if self.is_nonblocking() {
            self.try_send(buf, flags)
        } else {
            self.wait_events(IoEvents::OUT, || self.try_send(buf, flags))
        }
    }

    fn try_send(&self, buf: &[u8], _flags: SendRecvFlags) -> Result<usize> {
        match &*self.state.read() {
            State::Connected(connected) => connected.try_write(buf),
            _ => return_errno_with_message!(Errno::ENOTCONN, "the socket is not connected"),
        }
    }

    fn recv(&self, buf: &mut [u8], flags: SendRecvFlags) -> Result<usize> {
        if self.is_nonblocking() {
            self.try_recv(buf, flags)
        } else {
            self.wait_events(IoEvents::IN, || self.try_recv(buf, flags))
        }
    }

    fn try_recv(&self, buf: &mut [u8], _flags: SendRecvFlags) -> Result<usize> {
        match &*self.state.read() {
            State::Connected(connected) => connected.try_read(buf),
            _ => return_errno_with_message!(Errno::ENOTCONN, "the socket is not connected"),
        }
    }

    fn try_accept(&self) -> Result<(Arc<dyn FileLike>, SocketAddr)> {
        match &*self.state.read() {
            State::Listen(listen) => listen.try_accept() as _,
            _ => return_errno_with_message!(Errno::EINVAL, "the socket is not listening"),
        }
    }

    fn is_nonblocking(&self) -> bool {
        self.is_nonblocking.load(Ordering::Relaxed)
    }

    fn set_nonblocking(&self, nonblocking: bool) {
        self.is_nonblocking.store(nonblocking, Ordering::Relaxed);
    }
}

impl Pollable for UnixStreamSocket {
    fn poll(&self, mask: IoEvents, poller: Option<&mut Poller>) -> IoEvents {
        let inner = self.state.read();
        match &*inner {
            State::Init(init) => init.poll(mask, poller),
            State::Listen(listen) => listen.poll(mask, poller),
            State::Connected(connected) => connected.poll(mask, poller),
        }
    }
}

impl FileLike for UnixStreamSocket {
    fn as_socket(self: Arc<Self>) -> Option<Arc<dyn Socket>> {
        Some(self)
    }

    fn read(&self, buf: &mut [u8]) -> Result<usize> {
        // TODO: Set correct flags
        let flags = SendRecvFlags::empty();
        self.recv(buf, flags)
    }

    fn write(&self, buf: &[u8]) -> Result<usize> {
        // TODO: Set correct flags
        let flags = SendRecvFlags::empty();
        self.send(buf, flags)
    }

    fn status_flags(&self) -> StatusFlags {
        if self.is_nonblocking() {
            StatusFlags::O_NONBLOCK
        } else {
            StatusFlags::empty()
        }
    }

    fn set_status_flags(&self, new_flags: StatusFlags) -> Result<()> {
        self.set_nonblocking(new_flags.contains(StatusFlags::O_NONBLOCK));
        Ok(())
    }
}

impl Socket for UnixStreamSocket {
    fn bind(&self, socket_addr: SocketAddr) -> Result<()> {
        let addr = UnixSocketAddr::try_from(socket_addr)?;

        match &*self.state.read() {
            State::Init(init) => init.bind(&addr),
            _ => return_errno_with_message!(
                Errno::EINVAL,
                "cannot bind a listening or connected socket"
            ),
            // FIXME: Maybe binding a connected socket should also be allowed?
        }
    }

    fn connect(&self, socket_addr: SocketAddr) -> Result<()> {
        let remote_addr = {
            let unix_socket_addr = UnixSocketAddr::try_from(socket_addr)?;
            match unix_socket_addr {
                UnixSocketAddr::Abstract(abstract_name) => {
                    UnixSocketAddrBound::Abstract(abstract_name)
                }
                UnixSocketAddr::Path(path) => {
                    let dentry = lookup_socket_file(&path)?;
                    UnixSocketAddrBound::Path(dentry)
                }
            }
        };

        let connected = match &*self.state.read() {
            State::Init(init) => init.connect(&remote_addr)?,
            State::Listen(_) => return_errno_with_message!(Errno::EINVAL, "the socket is listened"),
            State::Connected(_) => {
                return_errno_with_message!(Errno::EISCONN, "the socket is connected")
            }
        };

        *self.state.write() = State::Connected(connected);
        Ok(())
    }

    fn listen(&self, backlog: usize) -> Result<()> {
        let addr = match &*self.state.read() {
            State::Init(init) => init
                .addr()
                .ok_or(Error::with_message(
                    Errno::EINVAL,
                    "the socket is not bound",
                ))?
                .clone(),
            State::Listen(_) => {
                return_errno_with_message!(Errno::EINVAL, "the socket is already listening")
            }
            State::Connected(_) => {
                return_errno_with_message!(Errno::EISCONN, "the socket is already connected")
            }
        };

        let listener = Listener::new(addr, backlog)?;
        *self.state.write() = State::Listen(listener);
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
        match &*self.state.read() {
            State::Connected(connected) => connected.shutdown(cmd),
            _ => return_errno_with_message!(Errno::ENOTCONN, "the socked is not connected"),
        }
    }

    fn addr(&self) -> Result<SocketAddr> {
        let addr = match &*self.state.read() {
            State::Init(init) => init.addr(),
            State::Listen(listen) => Some(listen.addr().clone()),
            State::Connected(connected) => connected.addr().cloned(),
        };

        addr.map(Into::<SocketAddr>::into)
            .ok_or(Error::with_message(
                Errno::EINVAL,
                "the socket does not bind to addr",
            ))
    }

    fn peer_addr(&self) -> Result<SocketAddr> {
        let peer_addr = match &*self.state.read() {
            State::Connected(connected) => connected.peer_addr().cloned(),
            _ => return_errno_with_message!(Errno::ENOTCONN, "the socket is not connected"),
        };

        match peer_addr {
            None => Ok(SocketAddr::Unix(UnixSocketAddr::Path(String::new()))),
            Some(peer_addr) => Ok(SocketAddr::from(peer_addr)),
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
        let received_bytes = self.recv(&mut buf, flags)?;

        let copied_bytes = {
            let message = &buf[..received_bytes];
            copy_message_to_user(io_vecs, message)
        };

        // TODO: Receive control message

        let message_header = MessageHeader::new(None, None);

        Ok((copied_bytes, message_header))
    }
}

impl Drop for UnixStreamSocket {
    fn drop(&mut self) {
        let Some(bound_addr) = self.bound_addr() else {
            return;
        };

        if let State::Listen(_) = &*self.state.read() {
            unregister_backlog(&bound_addr);
        }
    }
}

fn lookup_socket_file(path: &str) -> Result<Arc<Dentry>> {
    let dentry = {
        let current = current!();
        let fs = current.fs().read();
        let fs_path = FsPath::try_from(path)?;
        fs.lookup(&fs_path)?
    };

    if dentry.type_() != InodeType::Socket {
        return_errno_with_message!(Errno::ENOTSOCK, "not a socket file")
    }

    if !dentry.mode()?.is_readable() || !dentry.mode()?.is_writable() {
        return_errno_with_message!(Errno::EACCES, "the socket cannot be read or written")
    }
    Ok(dentry)
}
