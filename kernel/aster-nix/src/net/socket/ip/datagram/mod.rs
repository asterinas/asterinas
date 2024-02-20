// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::{AtomicBool, Ordering};

use takeable::Takeable;

use self::{bound::BoundDatagram, unbound::UnboundDatagram};
use super::common::get_ephemeral_endpoint;
use crate::{
    events::{IoEvents, Observer},
    fs::{file_handle::FileLike, utils::StatusFlags},
    net::{
        iface::IpEndpoint,
        poll_ifaces,
        socket::{
            util::{send_recv_flags::SendRecvFlags, socket_addr::SocketAddr},
            Socket,
        },
    },
    prelude::*,
    process::signal::{Pollee, Poller},
};

mod bound;
mod unbound;

pub struct DatagramSocket {
    inner: RwLock<Takeable<Inner>>,
    nonblocking: AtomicBool,
    pollee: Pollee,
}

enum Inner {
    Unbound(UnboundDatagram),
    Bound(BoundDatagram),
}

impl Inner {
    fn bind(self, endpoint: &IpEndpoint) -> core::result::Result<BoundDatagram, (Error, Self)> {
        let unbound_datagram = match self {
            Inner::Unbound(unbound_datagram) => unbound_datagram,
            Inner::Bound(bound_datagram) => {
                return Err((
                    Error::with_message(Errno::EINVAL, "the socket is already bound to an address"),
                    Inner::Bound(bound_datagram),
                ));
            }
        };

        let bound_datagram = match unbound_datagram.bind(endpoint) {
            Ok(bound_datagram) => bound_datagram,
            Err((err, unbound_datagram)) => return Err((err, Inner::Unbound(unbound_datagram))),
        };
        Ok(bound_datagram)
    }

    fn bind_to_ephemeral_endpoint(
        self,
        remote_endpoint: &IpEndpoint,
    ) -> core::result::Result<BoundDatagram, (Error, Self)> {
        if let Inner::Bound(bound_datagram) = self {
            return Ok(bound_datagram);
        }

        let endpoint = get_ephemeral_endpoint(remote_endpoint);
        self.bind(&endpoint)
    }
}

impl DatagramSocket {
    pub fn new(nonblocking: bool) -> Arc<Self> {
        Arc::new_cyclic(|me| {
            let unbound_datagram = UnboundDatagram::new(me.clone() as _);
            let pollee = Pollee::new(IoEvents::empty());
            Self {
                inner: RwLock::new(Takeable::new(Inner::Unbound(unbound_datagram))),
                nonblocking: AtomicBool::new(nonblocking),
                pollee,
            }
        })
    }

    pub fn is_nonblocking(&self) -> bool {
        self.nonblocking.load(Ordering::SeqCst)
    }

    pub fn set_nonblocking(&self, nonblocking: bool) {
        self.nonblocking.store(nonblocking, Ordering::SeqCst);
    }

    fn try_bind_empheral(&self, remote_endpoint: &IpEndpoint) -> Result<()> {
        // Fast path
        if let Inner::Bound(_) = self.inner.read().as_ref() {
            return Ok(());
        }

        // Slow path
        let mut inner = self.inner.write();
        inner.borrow_result(|owned_inner| {
            let bound_datagram = match owned_inner.bind_to_ephemeral_endpoint(remote_endpoint) {
                Ok(bound_datagram) => bound_datagram,
                Err((err, err_inner)) => {
                    return (err_inner, Err(err));
                }
            };
            bound_datagram.init_pollee(&self.pollee);
            (Inner::Bound(bound_datagram), Ok(()))
        })
    }

    fn try_recvfrom(&self, buf: &mut [u8], flags: SendRecvFlags) -> Result<(usize, SocketAddr)> {
        let inner = self.inner.read();
        let Inner::Bound(bound_datagram) = inner.as_ref() else {
            return_errno_with_message!(Errno::EINVAL, "the socket is not bound");
        };
        let (recv_bytes, remote_endpoint) = bound_datagram.try_recvfrom(buf, flags)?;
        bound_datagram.update_io_events(&self.pollee);
        Ok((recv_bytes, remote_endpoint.try_into()?))
    }

    fn try_sendto(
        &self,
        buf: &[u8],
        remote: Option<IpEndpoint>,
        flags: SendRecvFlags,
    ) -> Result<usize> {
        let inner = self.inner.read();
        let Inner::Bound(bound_datagram) = inner.as_ref() else {
            return_errno_with_message!(Errno::EINVAL, "the socket is not bound");
        };
        let sent_bytes = bound_datagram.try_sendto(buf, remote, flags)?;
        bound_datagram.update_io_events(&self.pollee);
        Ok(sent_bytes)
    }

    // TODO: Support timeout
    fn wait_events<F, R>(&self, mask: IoEvents, mut cond: F) -> Result<R>
    where
        F: FnMut() -> Result<R>,
    {
        let poller = Poller::new();

        loop {
            match cond() {
                Err(err) if err.error() == Errno::EAGAIN => (),
                result => return result,
            };

            let events = self.poll(mask, Some(&poller));
            if !events.is_empty() {
                continue;
            }

            poller.wait()?;
        }
    }

    fn update_io_events(&self) {
        let inner = self.inner.read();
        let Inner::Bound(bound_datagram) = inner.as_ref() else {
            return;
        };
        bound_datagram.update_io_events(&self.pollee);
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
        self.pollee.poll(mask, poller)
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

        let mut inner = self.inner.write();
        inner.borrow_result(|owned_inner| {
            let bound_datagram = match owned_inner.bind(&endpoint) {
                Ok(bound_datagram) => bound_datagram,
                Err((err, err_inner)) => {
                    return (err_inner, Err(err));
                }
            };
            bound_datagram.init_pollee(&self.pollee);
            (Inner::Bound(bound_datagram), Ok(()))
        })
    }

    fn connect(&self, socket_addr: SocketAddr) -> Result<()> {
        let endpoint = socket_addr.try_into()?;

        self.try_bind_empheral(&endpoint)?;

        let mut inner = self.inner.write();
        let Inner::Bound(bound_datagram) = inner.as_mut() else {
            return_errno_with_message!(Errno::EINVAL, "the socket is not bound")
        };
        bound_datagram.set_remote_endpoint(&endpoint);

        Ok(())
    }

    fn addr(&self) -> Result<SocketAddr> {
        let inner = self.inner.read();
        let Inner::Bound(bound_datagram) = inner.as_ref() else {
            return_errno_with_message!(Errno::EINVAL, "the socket is not bound");
        };
        bound_datagram.local_endpoint().try_into()
    }

    fn peer_addr(&self) -> Result<SocketAddr> {
        let inner = self.inner.read();
        let Inner::Bound(bound_datagram) = inner.as_ref() else {
            return_errno_with_message!(Errno::EINVAL, "the socket is not bound");
        };
        bound_datagram.remote_endpoint()?.try_into()
    }

    // FIXME: respect RecvFromFlags
    fn recvfrom(&self, buf: &mut [u8], flags: SendRecvFlags) -> Result<(usize, SocketAddr)> {
        debug_assert!(flags.is_all_supported());

        poll_ifaces();
        if self.is_nonblocking() {
            self.try_recvfrom(buf, flags)
        } else {
            self.wait_events(IoEvents::IN, || self.try_recvfrom(buf, flags))
        }
    }

    fn sendto(
        &self,
        buf: &[u8],
        remote: Option<SocketAddr>,
        flags: SendRecvFlags,
    ) -> Result<usize> {
        debug_assert!(flags.is_all_supported());

        let remote_endpoint = match remote {
            Some(remote_addr) => Some(remote_addr.try_into()?),
            None => None,
        };
        if let Some(endpoint) = remote_endpoint {
            self.try_bind_empheral(&endpoint)?;
        }

        // TODO: Block if the send buffer is full
        let sent_bytes = self.try_sendto(buf, remote_endpoint, flags)?;
        poll_ifaces();
        Ok(sent_bytes)
    }
}

impl Observer<()> for DatagramSocket {
    fn on_events(&self, events: &()) {
        self.update_io_events();
    }
}
