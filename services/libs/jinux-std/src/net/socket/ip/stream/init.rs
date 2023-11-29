use core::sync::atomic::{AtomicBool, Ordering};

use crate::events::IoEvents;
use crate::net::iface::Iface;
use crate::net::iface::IpEndpoint;
use crate::net::iface::{AnyBoundSocket, AnyUnboundSocket};
use crate::net::socket::ip::always_some::AlwaysSome;
use crate::net::socket::ip::common::{bind_socket, get_ephemeral_endpoint};
use crate::prelude::*;
use crate::process::signal::Poller;

use super::connecting::ConnectingStream;
use super::listen::ListenStream;

pub struct InitStream {
    inner: RwLock<Inner>,
    is_nonblocking: AtomicBool,
}

enum Inner {
    Unbound(AlwaysSome<Box<AnyUnboundSocket>>),
    Bound(AlwaysSome<Arc<AnyBoundSocket>>),
}

impl Inner {
    fn is_bound(&self) -> bool {
        match self {
            Self::Unbound(_) => false,
            Self::Bound(_) => true,
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

    fn bound_socket(&self) -> Option<&Arc<AnyBoundSocket>> {
        match self {
            Inner::Bound(bound_socket) => Some(bound_socket),
            Inner::Unbound(_) => None,
        }
    }

    fn poll(&self, mask: IoEvents, poller: Option<&Poller>) -> IoEvents {
        match self {
            Inner::Bound(bound_socket) => bound_socket.poll(mask, poller),
            Inner::Unbound(unbound_socket) => unbound_socket.poll(mask, poller),
        }
    }

    fn iface(&self) -> Option<Arc<dyn Iface>> {
        match self {
            Inner::Bound(bound_socket) => Some(bound_socket.iface().clone()),
            Inner::Unbound(_) => None,
        }
    }

    fn local_endpoint(&self) -> Option<IpEndpoint> {
        self.bound_socket()
            .and_then(|socket| socket.local_endpoint())
    }
}

impl InitStream {
    pub fn new(nonblocking: bool) -> Self {
        let socket = Box::new(AnyUnboundSocket::new_tcp());
        let inner = Inner::Unbound(AlwaysSome::new(socket));
        Self {
            is_nonblocking: AtomicBool::new(nonblocking),
            inner: RwLock::new(inner),
        }
    }

    pub fn new_bound(nonblocking: bool, bound_socket: Arc<AnyBoundSocket>) -> Self {
        let inner = Inner::Bound(AlwaysSome::new(bound_socket));
        Self {
            is_nonblocking: AtomicBool::new(nonblocking),
            inner: RwLock::new(inner),
        }
    }

    pub fn bind(&self, endpoint: IpEndpoint) -> Result<()> {
        self.inner.write().bind(endpoint)
    }

    pub fn connect(&self, remote_endpoint: &IpEndpoint) -> Result<ConnectingStream> {
        if !self.inner.read().is_bound() {
            self.inner
                .write()
                .bind_to_ephemeral_endpoint(remote_endpoint)?
        }
        ConnectingStream::new(
            self.is_nonblocking(),
            self.inner.read().bound_socket().unwrap().clone(),
            *remote_endpoint,
        )
    }

    pub fn listen(&self, backlog: usize) -> Result<ListenStream> {
        let bound_socket = if let Some(bound_socket) = self.inner.read().bound_socket() {
            bound_socket.clone()
        } else {
            return_errno_with_message!(Errno::EINVAL, "cannot listen without bound")
        };
        ListenStream::new(self.is_nonblocking(), bound_socket, backlog)
    }

    pub fn local_endpoint(&self) -> Result<IpEndpoint> {
        self.inner
            .read()
            .local_endpoint()
            .ok_or_else(|| Error::with_message(Errno::EINVAL, "does not has local endpoint"))
    }

    pub fn poll(&self, mask: IoEvents, poller: Option<&Poller>) -> IoEvents {
        self.inner.read().poll(mask, poller)
    }

    pub fn is_nonblocking(&self) -> bool {
        self.is_nonblocking.load(Ordering::Relaxed)
    }

    pub fn set_nonblocking(&self, nonblocking: bool) {
        self.is_nonblocking.store(nonblocking, Ordering::Relaxed);
    }
}
