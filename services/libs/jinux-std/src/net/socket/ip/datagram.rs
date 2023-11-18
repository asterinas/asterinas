use core::sync::atomic::{AtomicBool, Ordering};

use crate::events::IoEvents;
use crate::fs::utils::StatusFlags;
use crate::net::iface::IpEndpoint;

use crate::process::signal::Poller;
use crate::{
    fs::file_handle::FileLike,
    net::{
        iface::{AnyBoundSocket, AnyUnboundSocket, RawUdpSocket},
        poll_ifaces,
        socket::{
            util::{send_recv_flags::SendRecvFlags, sockaddr::SocketAddr},
            Socket,
        },
    },
    prelude::*,
};

use super::always_some::AlwaysSome;
use super::common::{bind_socket, get_ephemeral_endpoint};

pub struct DatagramSocket {
    nonblocking: AtomicBool,
    inner: RwLock<Inner>,
}

enum Inner {
    Unbound(AlwaysSome<AnyUnboundSocket>),
    Bound {
        bound_socket: Arc<AnyBoundSocket>,
        remote_endpoint: Option<IpEndpoint>,
    },
}

impl Inner {
    fn is_bound(&self) -> bool {
        matches!(self, Inner::Bound { .. })
    }

    fn bind(&mut self, endpoint: IpEndpoint) -> Result<()> {
        if self.is_bound() {
            return_errno_with_message!(Errno::EINVAL, "the socket is already bound to an address");
        }
        let unbound_socket = match self {
            Inner::Unbound(unbound_socket) => unbound_socket,
            _ => unreachable!(),
        };
        let bound_socket =
            unbound_socket.try_take_with(|socket| bind_socket(socket, endpoint, false))?;
        let bound_endpoint = bound_socket.local_endpoint().unwrap();
        bound_socket.raw_with(|socket: &mut RawUdpSocket| {
            socket
                .bind(bound_endpoint)
                .map_err(|_| Error::with_message(Errno::EINVAL, "cannot bind socket"))
        })?;
        *self = Inner::Bound {
            bound_socket,
            remote_endpoint: None,
        };
        // Once the socket is bound, we should update the socket state at once.
        self.update_socket_state();
        Ok(())
    }

    fn bind_to_ephemeral_endpoint(&mut self, remote_endpoint: &IpEndpoint) -> Result<()> {
        let endpoint = get_ephemeral_endpoint(remote_endpoint);
        self.bind(endpoint)
    }

    fn set_remote_endpoint(&mut self, endpoint: IpEndpoint) -> Result<()> {
        if let Inner::Bound {
            remote_endpoint, ..
        } = self
        {
            *remote_endpoint = Some(endpoint);
            Ok(())
        } else {
            return_errno_with_message!(Errno::EINVAL, "the socket is not bound");
        }
    }

    fn remote_endpoint(&self) -> Option<IpEndpoint> {
        if let Inner::Bound {
            remote_endpoint, ..
        } = self
        {
            *remote_endpoint
        } else {
            None
        }
    }

    fn local_endpoint(&self) -> Option<IpEndpoint> {
        if let Inner::Bound { bound_socket, .. } = self {
            bound_socket.local_endpoint()
        } else {
            None
        }
    }

    fn bound_socket(&self) -> Option<Arc<AnyBoundSocket>> {
        if let Inner::Bound { bound_socket, .. } = self {
            Some(bound_socket.clone())
        } else {
            None
        }
    }

    fn poll(&self, mask: IoEvents, poller: Option<&Poller>) -> IoEvents {
        match self {
            Inner::Unbound(unbound_socket) => unbound_socket.poll(mask, poller),
            Inner::Bound { bound_socket, .. } => bound_socket.poll(mask, poller),
        }
    }

    fn update_socket_state(&self) {
        if let Inner::Bound { bound_socket, .. } = self {
            bound_socket.update_socket_state();
        }
    }
}

impl DatagramSocket {
    pub fn new(nonblocking: bool) -> Self {
        let udp_socket = AnyUnboundSocket::new_udp();
        Self {
            inner: RwLock::new(Inner::Unbound(AlwaysSome::new(udp_socket))),
            nonblocking: AtomicBool::new(nonblocking),
        }
    }

    pub fn is_bound(&self) -> bool {
        self.inner.read().is_bound()
    }

    fn try_recvfrom(&self, buf: &mut [u8], flags: &SendRecvFlags) -> Result<(usize, IpEndpoint)> {
        poll_ifaces();
        let bound_socket = self.inner.read().bound_socket().unwrap();
        let recv_slice = |socket: &mut RawUdpSocket| match socket.recv_slice(buf) {
            Err(smoltcp::socket::udp::RecvError::Exhausted) => {
                return_errno_with_message!(Errno::EAGAIN, "recv buf is empty")
            }
            Ok((len, remote_endpoint)) => Ok((len, remote_endpoint)),
        };
        bound_socket.raw_with(recv_slice)
    }

    fn remote_endpoint(&self) -> Result<IpEndpoint> {
        self.inner
            .read()
            .remote_endpoint()
            .ok_or(Error::with_message(
                Errno::EINVAL,
                "udp should provide remote addr",
            ))
    }

    pub fn is_nonblocking(&self) -> bool {
        self.nonblocking.load(Ordering::SeqCst)
    }

    pub fn set_nonblocking(&self, nonblocking: bool) {
        self.nonblocking.store(nonblocking, Ordering::SeqCst);
    }
}

impl FileLike for DatagramSocket {
    fn read(&self, buf: &mut [u8]) -> Result<usize> {
        // FIXME: respect flags
        let flags = SendRecvFlags::empty();
        let (recv_len, _) = self.recvfrom(buf, flags)?;
        Ok(recv_len)
    }

    fn write(&self, buf: &[u8]) -> Result<usize> {
        // FIXME: set correct flags
        let flags = SendRecvFlags::empty();
        self.sendto(buf, None, flags)
    }

    fn poll(&self, mask: IoEvents, poller: Option<&Poller>) -> IoEvents {
        self.inner.read().poll(mask, poller)
    }

    fn as_socket(&self) -> Option<&dyn Socket> {
        Some(self)
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

impl Socket for DatagramSocket {
    fn bind(&self, sockaddr: SocketAddr) -> Result<()> {
        let endpoint = sockaddr.try_into()?;
        self.inner.write().bind(endpoint)
    }

    fn connect(&self, sockaddr: SocketAddr) -> Result<()> {
        let remote_endpoint: IpEndpoint = sockaddr.try_into()?;
        let mut inner = self.inner.write();
        if !self.is_bound() {
            inner.bind_to_ephemeral_endpoint(&remote_endpoint)?
        }
        inner.set_remote_endpoint(remote_endpoint)?;
        inner.update_socket_state();
        Ok(())
    }

    fn addr(&self) -> Result<SocketAddr> {
        if let Some(local_endpoint) = self.inner.read().local_endpoint() {
            local_endpoint.try_into()
        } else {
            return_errno_with_message!(Errno::EINVAL, "socket does not bind to local endpoint");
        }
    }

    fn peer_addr(&self) -> Result<SocketAddr> {
        if let Some(remote_endpoint) = self.inner.read().remote_endpoint() {
            remote_endpoint.try_into()
        } else {
            return_errno_with_message!(Errno::EINVAL, "remote endpoint is not specified");
        }
    }

    // FIXME: respect RecvFromFlags
    fn recvfrom(&self, buf: &mut [u8], flags: SendRecvFlags) -> Result<(usize, SocketAddr)> {
        debug_assert!(flags.is_all_supported());
        if !self.is_bound() {
            return_errno_with_message!(Errno::EINVAL, "socket does not bind to local endpoint");
        }
        let poller = Poller::new();
        let bound_socket = self.inner.read().bound_socket().unwrap();
        loop {
            if let Ok((recv_len, remote_endpoint)) = self.try_recvfrom(buf, &flags) {
                let remote_addr = remote_endpoint.try_into()?;
                return Ok((recv_len, remote_addr));
            }
            let events = self.inner.read().poll(IoEvents::IN, Some(&poller));
            if !events.contains(IoEvents::IN) {
                if self.is_nonblocking() {
                    return_errno_with_message!(Errno::EAGAIN, "try to receive again");
                }
                // FIXME: deal with recvfrom timeout
                poller.wait()?;
            }
        }
    }

    fn sendto(
        &self,
        buf: &[u8],
        remote: Option<SocketAddr>,
        flags: SendRecvFlags,
    ) -> Result<usize> {
        let remote_endpoint: IpEndpoint = if let Some(remote_addr) = remote {
            remote_addr.try_into()?
        } else {
            self.remote_endpoint()?
        };
        if !self.is_bound() {
            self.inner
                .write()
                .bind_to_ephemeral_endpoint(&remote_endpoint)?;
        }
        let bound_socket = self.inner.read().bound_socket().unwrap();
        let send_slice = |socket: &mut RawUdpSocket| match socket.send_slice(buf, remote_endpoint) {
            Err(_) => return_errno_with_message!(Errno::ENOBUFS, "send udp packet fails"),
            Ok(()) => Ok(buf.len()),
        };
        let len = bound_socket.raw_with(send_slice)?;
        poll_ifaces();
        Ok(len)
    }
}
