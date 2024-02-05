// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::{AtomicBool, Ordering};

use crate::events::IoEvents;
use crate::fs::file_handle::FileLike;
use crate::fs::utils::StatusFlags;
use crate::net::iface::IpEndpoint;
use crate::net::socket::util::send_recv_flags::SendRecvFlags;
use crate::net::socket::util::socket_addr::SocketAddr;
use crate::net::socket::Socket;
use crate::prelude::*;
use crate::process::signal::Poller;

use self::bound::BoundDatagram;
use self::unbound::UnboundDatagram;

use super::always_some::AlwaysSome;
use super::common::get_ephemeral_endpoint;

mod bound;
mod unbound;

pub struct DatagramSocket {
    nonblocking: AtomicBool,
    inner: RwLock<Inner>,
}

enum Inner {
    Unbound(AlwaysSome<UnboundDatagram>),
    Bound(Arc<BoundDatagram>),
}

impl Inner {
    fn is_bound(&self) -> bool {
        matches!(self, Inner::Bound { .. })
    }

    fn bind(&mut self, endpoint: IpEndpoint) -> Result<Arc<BoundDatagram>> {
        let unbound = match self {
            Inner::Unbound(unbound) => unbound,
            Inner::Bound(..) => return_errno_with_message!(
                Errno::EINVAL,
                "the socket is already bound to an address"
            ),
        };
        let bound = unbound.try_take_with(|unbound| unbound.bind(endpoint))?;
        *self = Inner::Bound(bound.clone());
        Ok(bound)
    }

    fn bind_to_ephemeral_endpoint(
        &mut self,
        remote_endpoint: &IpEndpoint,
    ) -> Result<Arc<BoundDatagram>> {
        let endpoint = get_ephemeral_endpoint(remote_endpoint);
        self.bind(endpoint)
    }

    fn poll(&self, mask: IoEvents, poller: Option<&Poller>) -> IoEvents {
        match self {
            Inner::Unbound(unbound) => unbound.poll(mask, poller),
            Inner::Bound(bound) => bound.poll(mask, poller),
        }
    }
}

impl DatagramSocket {
    pub fn new(nonblocking: bool) -> Self {
        let unbound = UnboundDatagram::new();
        Self {
            inner: RwLock::new(Inner::Unbound(AlwaysSome::new(unbound))),
            nonblocking: AtomicBool::new(nonblocking),
        }
    }

    pub fn is_bound(&self) -> bool {
        self.inner.read().is_bound()
    }

    pub fn is_nonblocking(&self) -> bool {
        self.nonblocking.load(Ordering::SeqCst)
    }

    pub fn set_nonblocking(&self, nonblocking: bool) {
        self.nonblocking.store(nonblocking, Ordering::SeqCst);
    }

    fn bound(&self) -> Result<Arc<BoundDatagram>> {
        if let Inner::Bound(bound) = &*self.inner.read() {
            Ok(bound.clone())
        } else {
            return_errno_with_message!(Errno::EINVAL, "socket does not bind to local endpoint")
        }
    }

    fn try_bind_empheral(&self, remote_endpoint: &IpEndpoint) -> Result<Arc<BoundDatagram>> {
        // Fast path
        if let Inner::Bound(bound) = &*self.inner.read() {
            return Ok(bound.clone());
        }

        // Slow path
        let mut inner = self.inner.write();
        if let Inner::Bound(bound) = &*inner {
            return Ok(bound.clone());
        }
        inner.bind_to_ephemeral_endpoint(remote_endpoint)
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

    fn as_socket(self: Arc<Self>) -> Option<Arc<dyn Socket>> {
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
    fn bind(&self, socket_addr: SocketAddr) -> Result<()> {
        let endpoint = socket_addr.try_into()?;
        self.inner.write().bind(endpoint)?;
        Ok(())
    }

    fn connect(&self, socket_addr: SocketAddr) -> Result<()> {
        let endpoint = socket_addr.try_into()?;
        let bound = self.try_bind_empheral(&endpoint)?;
        bound.set_remote_endpoint(endpoint);
        Ok(())
    }

    fn addr(&self) -> Result<SocketAddr> {
        self.bound()?.local_endpoint()?.try_into()
    }

    fn peer_addr(&self) -> Result<SocketAddr> {
        self.bound()?.remote_endpoint()?.try_into()
    }

    // FIXME: respect RecvFromFlags
    fn recvfrom(&self, buf: &mut [u8], flags: SendRecvFlags) -> Result<(usize, SocketAddr)> {
        debug_assert!(flags.is_all_supported());
        let bound = self.bound()?;
        let poller = Poller::new();
        loop {
            if let Ok((recv_len, remote_endpoint)) = bound.try_recvfrom(buf, &flags) {
                let remote_addr = remote_endpoint.try_into()?;
                return Ok((recv_len, remote_addr));
            }
            let events = bound.poll(IoEvents::IN, Some(&poller));
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
        debug_assert!(flags.is_all_supported());
        let (bound, remote_endpoint) = if let Some(addr) = remote {
            let endpoint = addr.try_into()?;
            (self.try_bind_empheral(&endpoint)?, Some(endpoint))
        } else {
            let bound = self.bound()?;
            (bound, None)
        };
        bound.try_sendto(buf, remote_endpoint, flags)
    }
}
