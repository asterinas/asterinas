use crate::fs::file_handle::FileLike;
use crate::fs::utils::{IoEvents, Poller, StatusFlags};
use crate::net::socket::unix::addr::UnixSocketAddr;
use crate::net::socket::util::send_recv_flags::SendRecvFlags;
use crate::net::socket::util::sockaddr::SocketAddr;
use crate::net::socket::{SockShutdownCmd, Socket};
use crate::prelude::*;

use super::connected::Connected;
use super::endpoint::Endpoint;
use super::init::Init;
use super::listen::Listen;
use super::ACTIVE_LISTENERS;

pub struct UnixStreamSocket(RwLock<Status>);

enum Status {
    Init(Init),
    Listen(Listen),
    Connected(Connected),
}

impl UnixStreamSocket {
    pub fn new(nonblocking: bool) -> Self {
        let status = Status::Init(Init::new(nonblocking));
        Self(RwLock::new(status))
    }

    pub fn new_pair(nonblocking: bool) -> Result<(Arc<Self>, Arc<Self>)> {
        let (end_a, end_b) = Endpoint::end_pair(nonblocking)?;
        let connected_a = UnixStreamSocket(RwLock::new(Status::Connected(Connected::new(end_a))));
        let connected_b = UnixStreamSocket(RwLock::new(Status::Connected(Connected::new(end_b))));
        Ok((Arc::new(connected_a), Arc::new(connected_b)))
    }

    fn bound_addr(&self) -> Option<UnixSocketAddr> {
        let status = self.0.read();
        match &*status {
            Status::Init(init) => init.bound_addr().map(Clone::clone),
            Status::Listen(listen) => Some(listen.addr().clone()),
            Status::Connected(connected) => connected.addr(),
        }
    }

    fn supported_flags(status_flags: &StatusFlags) -> StatusFlags {
        const SUPPORTED_FLAGS: StatusFlags = StatusFlags::O_NONBLOCK;
        const UNSUPPORTED_FLAGS: StatusFlags = SUPPORTED_FLAGS.complement();

        if status_flags.intersects(UNSUPPORTED_FLAGS) {
            warn!("ignore unsupported flags");
        }

        status_flags.intersection(SUPPORTED_FLAGS)
    }
}

impl FileLike for UnixStreamSocket {
    fn as_socket(&self) -> Option<&dyn Socket> {
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
            Status::Listen(listen) => {
                let addr = listen.addr();
                let listener = ACTIVE_LISTENERS.get_listener(addr).unwrap();
                listener.poll(mask, poller)
            }
            Status::Connected(connet) => todo!(),
        }
    }

    fn status_flags(&self) -> StatusFlags {
        let inner = self.0.read();
        let is_nonblocking = match &*inner {
            Status::Init(init) => init.is_nonblocking(),
            Status::Listen(listen) => listen.is_nonblocking(),
            Status::Connected(connected) => connected.is_nonblocking(),
        };

        if is_nonblocking {
            StatusFlags::O_NONBLOCK
        } else {
            StatusFlags::empty()
        }
    }

    fn set_status_flags(&self, new_flags: StatusFlags) -> Result<()> {
        let is_nonblocking = {
            let supported_flags = Self::supported_flags(&new_flags);
            supported_flags.contains(StatusFlags::O_NONBLOCK)
        };

        let mut inner = self.0.write();
        match &mut *inner {
            Status::Init(init) => init.set_nonblocking(is_nonblocking),
            Status::Listen(listen) => listen.set_nonblocking(is_nonblocking),
            Status::Connected(connected) => connected.set_nonblocking(is_nonblocking),
        }
        Ok(())
    }
}

impl Socket for UnixStreamSocket {
    fn bind(&self, sockaddr: SocketAddr) -> Result<()> {
        let addr = UnixSocketAddr::try_from(sockaddr)?;
        let mut inner = self.0.write();
        match &mut *inner {
            Status::Init(init) => init.bind(addr),
            Status::Listen(_) | Status::Connected(_) => {
                return_errno_with_message!(
                    Errno::EINVAL,
                    "cannot bind a listening or connected socket"
                );
            } // FIXME: Maybe binding a connected sockted should also be allowed?
        }
    }

    fn connect(&self, sockaddr: SocketAddr) -> Result<()> {
        let mut inner = self.0.write();
        match &*inner {
            Status::Init(init) => {
                let remote_addr = UnixSocketAddr::try_from(sockaddr)?;
                let addr = init.bound_addr();
                if let Some(addr) = addr {
                    if addr.path() == remote_addr.path() {
                        return_errno_with_message!(
                            Errno::EINVAL,
                            "try to connect to self is invalid"
                        );
                    }
                }
                let (this_end, remote_end) = Endpoint::end_pair(init.is_nonblocking())?;
                remote_end.set_addr(remote_addr.clone());
                if let Some(addr) = addr {
                    this_end.set_addr(addr.clone());
                };
                ACTIVE_LISTENERS.push_incoming(&remote_addr, remote_end)?;
                *inner = Status::Connected(Connected::new(this_end));
                Ok(())
            }
            Status::Listen(_) => {
                return_errno_with_message!(Errno::EINVAL, "the socket is listened")
            }
            Status::Connected(_) => {
                return_errno_with_message!(Errno::EISCONN, "the socket is connected")
            }
        }
    }

    fn listen(&self, backlog: usize) -> Result<()> {
        let mut inner = self.0.write();
        match &*inner {
            Status::Init(init) => {
                let addr = init.bound_addr().ok_or(Error::with_message(
                    Errno::EINVAL,
                    "the socket is not bound",
                ))?;
                ACTIVE_LISTENERS.add_listener(addr, backlog)?;
                *inner = Status::Listen(Listen::new(addr.clone(), init.is_nonblocking()));
                return Ok(());
            }
            Status::Listen(_) => {
                return_errno_with_message!(Errno::EINVAL, "the socket is already listened")
            }
            Status::Connected(_) => {
                return_errno_with_message!(Errno::EINVAL, "the socket is already connected")
            }
        };
    }

    fn accept(&self) -> Result<(Arc<dyn FileLike>, SocketAddr)> {
        let inner = self.0.read();
        match &*inner {
            Status::Listen(listen) => {
                let is_nonblocking = listen.is_nonblocking();
                let addr = listen.addr().clone();
                drop(inner);
                // Avoid lock when waiting
                let connected = {
                    let local_endpoint = ACTIVE_LISTENERS.pop_incoming(is_nonblocking, &addr)?;
                    Connected::new(local_endpoint)
                };

                let peer_addr = match connected.peer_addr() {
                    None => SocketAddr::Unix(String::new()),
                    Some(addr) => SocketAddr::from(addr.clone()),
                };

                let socket = UnixStreamSocket(RwLock::new(Status::Connected(connected)));
                return Ok((Arc::new(socket), peer_addr));
            }
            Status::Connected(_) | Status::Init(_) => {
                return_errno_with_message!(Errno::EINVAL, "the socket is not listened")
            }
        }
    }

    fn shutdown(&self, cmd: SockShutdownCmd) -> Result<()> {
        let inner = self.0.read();
        if let Status::Connected(connected) = &*inner {
            connected.shutdown(cmd)
        } else {
            return_errno_with_message!(Errno::ENOTCONN, "the socked is not connected");
        }
    }

    fn addr(&self) -> Result<SocketAddr> {
        let inner = self.0.read();
        let addr = match &*inner {
            Status::Init(init) => init.bound_addr().map(Clone::clone),
            Status::Listen(listen) => Some(listen.addr().clone()),
            Status::Connected(connected) => connected.addr(),
        };
        addr.map(Into::<SocketAddr>::into)
            .ok_or(Error::with_message(
                Errno::EINVAL,
                "the socket does not bind to addr",
            ))
    }

    fn peer_addr(&self) -> Result<SocketAddr> {
        let inner = self.0.read();
        if let Status::Connected(connected) = &*inner {
            match connected.peer_addr() {
                None => return Ok(SocketAddr::Unix(String::new())),
                Some(peer_addr) => {
                    return Ok(SocketAddr::from(peer_addr.clone()));
                }
            }
        }
        return_errno_with_message!(Errno::EINVAL, "the socket is not connected");
    }

    fn recvfrom(&self, buf: &mut [u8], flags: SendRecvFlags) -> Result<(usize, SocketAddr)> {
        let inner = self.0.read();
        // TODO: deal with flags
        match &*inner {
            Status::Connected(connected) => {
                let read_size = connected.read(buf)?;
                let peer_addr = self.peer_addr()?;
                Ok((read_size, peer_addr))
            }
            Status::Init(_) | Status::Listen(_) => {
                return_errno_with_message!(Errno::EINVAL, "the socket is not connected")
            }
        }
    }

    fn sendto(
        &self,
        buf: &[u8],
        remote: Option<SocketAddr>,
        flags: SendRecvFlags,
    ) -> Result<usize> {
        debug_assert!(remote.is_none());
        // TODO: deal with flags
        let inner = self.0.read();
        match &*inner {
            Status::Connected(connected) => connected.write(buf),
            Status::Init(_) | Status::Listen(_) => {
                return_errno_with_message!(Errno::EINVAL, "the socket is not connected")
            }
        }
    }
}

impl Drop for UnixStreamSocket {
    fn drop(&mut self) {
        let Some(bound_addr) = self.bound_addr() else {
            return;
        };

        ACTIVE_LISTENERS.remove_listener(&bound_addr);
    }
}
