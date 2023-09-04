use core::sync::atomic::{AtomicBool, Ordering};

use crate::fs::utils::{IoEvents, Poller};
use crate::net::iface::Iface;
use crate::net::iface::IpEndpoint;
use crate::net::iface::{AnyBoundSocket, AnyUnboundSocket};
use crate::net::poll_ifaces;
use crate::net::socket::ip::always_some::AlwaysSome;
use crate::net::socket::ip::common::{bind_socket, get_ephemeral_endpoint};
use crate::prelude::*;

pub struct InitStream {
    inner: RwLock<Inner>,
    is_nonblocking: AtomicBool,
}

enum Inner {
    Unbound(AlwaysSome<AnyUnboundSocket>),
    Bound(AlwaysSome<Arc<AnyBoundSocket>>),
    Connecting {
        bound_socket: Arc<AnyBoundSocket>,
        remote_endpoint: IpEndpoint,
    },
}

impl Inner {
    fn is_bound(&self) -> bool {
        match self {
            Self::Unbound(_) => false,
            Self::Bound(..) | Self::Connecting { .. } => true,
        }
    }

    fn bind(&mut self, endpoint: IpEndpoint) -> Result<()> {
        let unbound_socket = if let Inner::Unbound(unbound_socket) = self {
            unbound_socket
        } else {
            return_errno_with_message!(Errno::EINVAL, "the socket is already bound to an address");
        };
        let bound_socket =
            unbound_socket.try_take_with(|raw_socket| bind_socket(raw_socket, endpoint, false))?;
        bound_socket.update_socket_state();
        *self = Inner::Bound(AlwaysSome::new(bound_socket));
        Ok(())
    }

    fn bind_to_ephemeral_endpoint(&mut self, remote_endpoint: &IpEndpoint) -> Result<()> {
        let endpoint = get_ephemeral_endpoint(remote_endpoint);
        self.bind(endpoint)
    }

    fn do_connect(&mut self, new_remote_endpoint: IpEndpoint) -> Result<()> {
        match self {
            Inner::Unbound(_) => return_errno_with_message!(Errno::EINVAL, "the socket is invalid"),
            Inner::Connecting {
                bound_socket,
                remote_endpoint,
            } => {
                *remote_endpoint = new_remote_endpoint;
                bound_socket.do_connect(new_remote_endpoint)?;
            }
            Inner::Bound(bound_socket) => {
                bound_socket.do_connect(new_remote_endpoint)?;
                *self = Inner::Connecting {
                    bound_socket: bound_socket.take(),
                    remote_endpoint: new_remote_endpoint,
                };
            }
        }
        Ok(())
    }

    fn bound_socket(&self) -> Option<&Arc<AnyBoundSocket>> {
        match self {
            Inner::Bound(bound_socket) => Some(bound_socket),
            Inner::Connecting { bound_socket, .. } => Some(bound_socket),
            _ => None,
        }
    }

    fn poll(&self, mask: IoEvents, poller: Option<&Poller>) -> IoEvents {
        match self {
            Inner::Bound(bound_socket) => bound_socket.poll(mask, poller),
            Inner::Connecting { bound_socket, .. } => bound_socket.poll(mask, poller),
            Inner::Unbound(unbound_socket) => unbound_socket.poll(mask, poller),
        }
    }

    fn iface(&self) -> Option<Arc<dyn Iface>> {
        match self {
            Inner::Bound(bound_socket) => Some(bound_socket.iface().clone()),
            Inner::Connecting { bound_socket, .. } => Some(bound_socket.iface().clone()),
            _ => None,
        }
    }

    fn local_endpoint(&self) -> Option<IpEndpoint> {
        self.bound_socket()
            .and_then(|socket| socket.local_endpoint())
    }

    fn remote_endpoint(&self) -> Option<IpEndpoint> {
        if let Inner::Connecting {
            remote_endpoint, ..
        } = self
        {
            Some(*remote_endpoint)
        } else {
            None
        }
    }
}

impl InitStream {
    pub fn new(nonblocking: bool) -> Self {
        let socket = AnyUnboundSocket::new_tcp();
        let inner = Inner::Unbound(AlwaysSome::new(socket));
        Self {
            is_nonblocking: AtomicBool::new(nonblocking),
            inner: RwLock::new(inner),
        }
    }

    pub fn is_bound(&self) -> bool {
        self.inner.read().is_bound()
    }

    pub fn bind(&self, endpoint: IpEndpoint) -> Result<()> {
        self.inner.write().bind(endpoint)
    }

    pub fn connect(&self, remote_endpoint: &IpEndpoint) -> Result<()> {
        if !self.is_bound() {
            self.inner
                .write()
                .bind_to_ephemeral_endpoint(remote_endpoint)?
        }
        self.inner.write().do_connect(*remote_endpoint)?;
        // Wait until building connection
        let poller = Poller::new();
        loop {
            poll_ifaces();
            let events = self
                .inner
                .read()
                .poll(IoEvents::OUT | IoEvents::IN, Some(&poller));
            if events.contains(IoEvents::IN) || events.contains(IoEvents::OUT) {
                return Ok(());
            } else if !events.is_empty() {
                return_errno_with_message!(Errno::ECONNREFUSED, "connect refused");
            } else if self.is_nonblocking() {
                return_errno_with_message!(Errno::EAGAIN, "try connect again");
            } else {
                poller.wait();
            }
        }
    }

    pub fn local_endpoint(&self) -> Result<IpEndpoint> {
        self.inner
            .read()
            .local_endpoint()
            .ok_or_else(|| Error::with_message(Errno::EINVAL, "does not has local endpoint"))
    }

    pub fn remote_endpoint(&self) -> Result<IpEndpoint> {
        self.inner
            .read()
            .remote_endpoint()
            .ok_or_else(|| Error::with_message(Errno::EINVAL, "does not has remote endpoint"))
    }

    pub(super) fn poll(&self, mask: IoEvents, poller: Option<&Poller>) -> IoEvents {
        self.inner.read().poll(mask, poller)
    }

    pub fn bound_socket(&self) -> Option<Arc<AnyBoundSocket>> {
        self.inner.read().bound_socket().map(Clone::clone)
    }

    pub fn is_nonblocking(&self) -> bool {
        self.is_nonblocking.load(Ordering::Relaxed)
    }

    pub fn set_nonblocking(&self, nonblocking: bool) {
        self.is_nonblocking.store(nonblocking, Ordering::Relaxed);
    }
}
