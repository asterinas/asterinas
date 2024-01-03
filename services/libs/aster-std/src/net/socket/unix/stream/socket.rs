// SPDX-License-Identifier: MPL-2.0

use crate::events::IoEvents;
use crate::fs::file_handle::FileLike;
use crate::fs::fs_resolver::FsPath;
use crate::fs::utils::{Dentry, InodeType, StatusFlags};
use crate::net::socket::unix::addr::UnixSocketAddrBound;
use crate::net::socket::unix::UnixSocketAddr;
use crate::net::socket::util::send_recv_flags::SendRecvFlags;
use crate::net::socket::util::socket_addr::SocketAddr;
use crate::net::socket::{SockShutdownCmd, Socket};
use crate::prelude::*;
use crate::process::signal::Poller;

use super::connected::Connected;
use super::endpoint::Endpoint;
use super::init::Init;
use super::listener::{unregister_backlog, Listener};

pub struct UnixStreamSocket(RwLock<State>);

impl UnixStreamSocket {
    pub(super) fn new_init(init: Init) -> Self {
        Self(RwLock::new(State::Init(Arc::new(init))))
    }

    pub(super) fn new_listen(listen: Listener) -> Self {
        Self(RwLock::new(State::Listen(Arc::new(listen))))
    }

    pub(super) fn new_connected(connected: Connected) -> Self {
        Self(RwLock::new(State::Connected(Arc::new(connected))))
    }
}

enum State {
    Init(Arc<Init>),
    Listen(Arc<Listener>),
    Connected(Arc<Connected>),
}

impl UnixStreamSocket {
    pub fn new(nonblocking: bool) -> Self {
        let init = Init::new(nonblocking);
        Self::new_init(init)
    }

    pub fn new_pair(nonblocking: bool) -> Result<(Arc<Self>, Arc<Self>)> {
        let (end_a, end_b) = Endpoint::new_pair(nonblocking)?;
        let connected_a = {
            let connected = Connected::new(end_a);
            Self::new_connected(connected)
        };
        let connected_b = {
            let connected = Connected::new(end_b);
            Self::new_connected(connected)
        };
        Ok((Arc::new(connected_a), Arc::new(connected_b)))
    }

    fn bound_addr(&self) -> Option<UnixSocketAddrBound> {
        let status = self.0.read();
        match &*status {
            State::Init(init) => init.addr(),
            State::Listen(listen) => Some(listen.addr().clone()),
            State::Connected(connected) => connected.addr(),
        }
    }

    fn mask_flags(status_flags: &StatusFlags) -> StatusFlags {
        const SUPPORTED_FLAGS: StatusFlags = StatusFlags::O_NONBLOCK;
        const UNSUPPORTED_FLAGS: StatusFlags = SUPPORTED_FLAGS.complement();

        if status_flags.intersects(UNSUPPORTED_FLAGS) {
            warn!("ignore unsupported flags");
        }

        status_flags.intersection(SUPPORTED_FLAGS)
    }
}

impl FileLike for UnixStreamSocket {
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
            State::Init(init) => init.poll(mask, poller),
            State::Listen(listen) => listen.poll(mask, poller),
            State::Connected(connected) => connected.poll(mask, poller),
        }
    }

    fn status_flags(&self) -> StatusFlags {
        let inner = self.0.read();
        let is_nonblocking = match &*inner {
            State::Init(init) => init.is_nonblocking(),
            State::Listen(listen) => listen.is_nonblocking(),
            State::Connected(connected) => connected.is_nonblocking(),
        };

        if is_nonblocking {
            StatusFlags::O_NONBLOCK
        } else {
            StatusFlags::empty()
        }
    }

    fn set_status_flags(&self, new_flags: StatusFlags) -> Result<()> {
        let is_nonblocking = {
            let supported_flags = Self::mask_flags(&new_flags);
            supported_flags.contains(StatusFlags::O_NONBLOCK)
        };

        let mut inner = self.0.write();
        match &mut *inner {
            State::Init(init) => init.set_nonblocking(is_nonblocking),
            State::Listen(listen) => listen.set_nonblocking(is_nonblocking),
            State::Connected(connected) => connected.set_nonblocking(is_nonblocking),
        }
        Ok(())
    }
}

impl Socket for UnixStreamSocket {
    fn bind(&self, socket_addr: SocketAddr) -> Result<()> {
        let addr = UnixSocketAddr::try_from(socket_addr)?;

        let init = match &*self.0.read() {
            State::Init(init) => init.clone(),
            _ => return_errno_with_message!(
                Errno::EINVAL,
                "cannot bind a listening or connected socket"
            ),
            // FIXME: Maybe binding a connected socket should also be allowed?
        };

        init.bind(&addr)
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

        let init = match &*self.0.read() {
            State::Init(init) => init.clone(),
            State::Listen(_) => return_errno_with_message!(Errno::EINVAL, "the socket is listened"),
            State::Connected(_) => {
                return_errno_with_message!(Errno::EISCONN, "the socket is connected")
            }
        };

        let connected = init.connect(&remote_addr)?;

        *self.0.write() = State::Connected(Arc::new(connected));
        Ok(())
    }

    fn listen(&self, backlog: usize) -> Result<()> {
        let init = match &*self.0.read() {
            State::Init(init) => init.clone(),
            State::Listen(_) => {
                return_errno_with_message!(Errno::EINVAL, "the socket is already listening")
            }
            State::Connected(_) => {
                return_errno_with_message!(Errno::EISCONN, "the socket is already connected")
            }
        };

        let addr = init.addr().ok_or(Error::with_message(
            Errno::EINVAL,
            "the socket is not bound",
        ))?;

        let listener = Listener::new(addr.clone(), backlog, init.is_nonblocking())?;
        *self.0.write() = State::Listen(Arc::new(listener));
        Ok(())
    }

    fn accept(&self) -> Result<(Arc<dyn FileLike>, SocketAddr)> {
        let listen = match &*self.0.read() {
            State::Listen(listen) => listen.clone(),
            _ => return_errno_with_message!(Errno::EINVAL, "the socket is not listening"),
        };

        listen.accept()
    }

    fn shutdown(&self, cmd: SockShutdownCmd) -> Result<()> {
        let connected = match &*self.0.read() {
            State::Connected(connected) => connected.clone(),
            _ => return_errno_with_message!(Errno::ENOTCONN, "the socked is not connected"),
        };

        connected.shutdown(cmd)
    }

    fn addr(&self) -> Result<SocketAddr> {
        let addr = match &*self.0.read() {
            State::Init(init) => init.addr(),
            State::Listen(listen) => Some(listen.addr().clone()),
            State::Connected(connected) => connected.addr(),
        };

        addr.map(Into::<SocketAddr>::into)
            .ok_or(Error::with_message(
                Errno::EINVAL,
                "the socket does not bind to addr",
            ))
    }

    fn peer_addr(&self) -> Result<SocketAddr> {
        let connected = match &*self.0.read() {
            State::Connected(connected) => connected.clone(),
            _ => return_errno_with_message!(Errno::ENOTCONN, "the socket is not connected"),
        };

        match connected.peer_addr() {
            None => Ok(SocketAddr::Unix(UnixSocketAddr::Path(String::new()))),
            Some(peer_addr) => Ok(SocketAddr::from(peer_addr.clone())),
        }
    }

    fn recvfrom(&self, buf: &mut [u8], flags: SendRecvFlags) -> Result<(usize, SocketAddr)> {
        let connected = match &*self.0.read() {
            State::Connected(connected) => connected.clone(),
            _ => return_errno_with_message!(Errno::ENOTCONN, "the socket is not connected"),
        };

        let peer_addr = self.peer_addr()?;
        let read_size = connected.read(buf)?;
        Ok((read_size, peer_addr))
    }

    fn sendto(
        &self,
        buf: &[u8],
        remote: Option<SocketAddr>,
        flags: SendRecvFlags,
    ) -> Result<usize> {
        debug_assert!(remote.is_none());
        // TODO: deal with flags
        let connected = match &*self.0.read() {
            State::Connected(connected) => connected.clone(),
            _ => return_errno_with_message!(Errno::ENOTCONN, "the socket is not connected"),
        };

        connected.write(buf)
    }
}

impl Drop for UnixStreamSocket {
    fn drop(&mut self) {
        let Some(bound_addr) = self.bound_addr() else {
            return;
        };

        if let State::Listen(_) = &*self.0.read() {
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

    if dentry.inode_type() != InodeType::Socket {
        return_errno_with_message!(Errno::ENOTSOCK, "not a socket file")
    }

    if !dentry.inode_mode().is_readable() || !dentry.inode_mode().is_writable() {
        return_errno_with_message!(Errno::EACCES, "the socket cannot be read or written")
    }
    Ok(dentry)
}
