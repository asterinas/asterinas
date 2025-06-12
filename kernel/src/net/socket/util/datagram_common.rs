// SPDX-License-Identifier: MPL-2.0

use ostd::sync::RwMutex;

use super::SendRecvFlags;
use crate::{
    events::IoEvents,
    process::signal::Pollee,
    return_errno_with_message,
    util::{MultiRead, MultiWrite},
    Errno, Error, Result,
};

pub trait Unbound {
    type Endpoint;
    type BindOptions;

    type Bound;

    fn bind(
        &mut self,
        endpoint: &Self::Endpoint,
        pollee: &Pollee,
        options: Self::BindOptions,
    ) -> Result<Self::Bound>;
    fn bind_ephemeral(
        &mut self,
        remote_endpoint: &Self::Endpoint,
        pollee: &Pollee,
    ) -> Result<Self::Bound>;

    fn check_io_events(&self) -> IoEvents;
}

pub trait Bound {
    type Endpoint;

    fn local_endpoint(&self) -> Self::Endpoint;
    fn bind(&mut self, _endpoint: &Self::Endpoint) -> Result<()> {
        return_errno_with_message!(Errno::EINVAL, "the socket is already bound to an address")
    }
    fn remote_endpoint(&self) -> Option<&Self::Endpoint>;
    fn set_remote_endpoint(&mut self, endpoint: &Self::Endpoint);

    fn try_recv(
        &self,
        writer: &mut dyn MultiWrite,
        flags: SendRecvFlags,
    ) -> Result<(usize, Self::Endpoint)>;
    fn try_send(
        &self,
        reader: &mut dyn MultiRead,
        remote: &Self::Endpoint,
        flags: SendRecvFlags,
    ) -> Result<usize>;

    fn check_io_events(&self) -> IoEvents;
}

pub enum Inner<UnboundSocket, BoundSocket> {
    Unbound(UnboundSocket),
    Bound(BoundSocket),
}

impl<UnboundSocket, BoundSocket> Inner<UnboundSocket, BoundSocket>
where
    UnboundSocket: Unbound<Endpoint = BoundSocket::Endpoint, Bound = BoundSocket>,
    BoundSocket: Bound,
{
    pub fn bind(
        &mut self,
        endpoint: &UnboundSocket::Endpoint,
        pollee: &Pollee,
        options: UnboundSocket::BindOptions,
    ) -> Result<()> {
        let unbound_datagram = match self {
            Inner::Unbound(unbound_datagram) => unbound_datagram,
            Inner::Bound(bound_datagram) => {
                return bound_datagram.bind(endpoint);
            }
        };

        let bound_datagram = unbound_datagram.bind(endpoint, pollee, options)?;
        *self = Inner::Bound(bound_datagram);

        Ok(())
    }

    pub fn bind_ephemeral(
        &mut self,
        remote_endpoint: &UnboundSocket::Endpoint,
        pollee: &Pollee,
    ) -> Result<()> {
        let unbound_datagram = match self {
            Inner::Unbound(unbound_datagram) => unbound_datagram,
            Inner::Bound(_) => return Ok(()),
        };

        let bound_datagram = unbound_datagram.bind_ephemeral(remote_endpoint, pollee)?;
        *self = Inner::Bound(bound_datagram);

        Ok(())
    }

    pub fn connect(
        &mut self,
        remote_endpoint: &UnboundSocket::Endpoint,
        pollee: &Pollee,
    ) -> Result<()> {
        self.bind_ephemeral(remote_endpoint, pollee)?;

        let bound_datagram = match self {
            Inner::Unbound(_) => {
                unreachable!(
                    "`bind_to_ephemeral_endpoint` succeeds so the socket cannot be unbound"
                );
            }
            Inner::Bound(bound_datagram) => bound_datagram,
        };
        bound_datagram.set_remote_endpoint(remote_endpoint);

        Ok(())
    }

    pub fn addr(&self) -> Option<UnboundSocket::Endpoint> {
        match self {
            Inner::Unbound(_) => None,
            Inner::Bound(bound_datagram) => Some(bound_datagram.local_endpoint()),
        }
    }

    pub fn peer_addr(&self) -> Option<&UnboundSocket::Endpoint> {
        match self {
            Inner::Unbound(_) => None,
            Inner::Bound(bound_datagram) => bound_datagram.remote_endpoint(),
        }
    }

    pub fn try_recv(
        &self,
        writer: &mut dyn MultiWrite,
        flags: SendRecvFlags,
    ) -> Result<(usize, UnboundSocket::Endpoint)> {
        match self {
            Inner::Unbound(_) => {
                return_errno_with_message!(Errno::EAGAIN, "the socket is not bound");
            }
            Inner::Bound(bound_datagram) => bound_datagram.try_recv(writer, flags),
        }
    }

    // If you're looking for `try_send`, there isn't one. Use `select_remote_and_bind` below and
    // call `Bound::try_send` directly.

    pub fn check_io_events(&self) -> IoEvents {
        match self {
            Inner::Unbound(unbound_datagram) => unbound_datagram.check_io_events(),
            Inner::Bound(bound_datagram) => bound_datagram.check_io_events(),
        }
    }
}

/// Selects the remote endpoint and binds if the socket is not bound.
///
/// The remote endpoint specified in the system call (e.g., `sendto`) argument is preferred,
/// otherwise the connected endpoint of the socket is used. If there are no remote endpoints
/// available, this method will fail with [`EDESTADDRREQ`].
///
/// If the remote endpoint is specified but the socket is not bound, this method will try to
/// bind the socket to an ephemeral endpoint.
///
/// If the above steps succeed, `op` will be called with the bound socket and the selected
/// remote endpoint.
///
/// [`EDESTADDRREQ`]: crate::error::Errno::EDESTADDRREQ
pub fn select_remote_and_bind<UnboundSocket, BoundSocket, B, F, R>(
    inner_mutex: &RwMutex<Inner<UnboundSocket, BoundSocket>>,
    remote: Option<&UnboundSocket::Endpoint>,
    bind_ephemeral: B,
    op: F,
) -> Result<R>
where
    UnboundSocket: Unbound<Endpoint = BoundSocket::Endpoint, Bound = BoundSocket>,
    BoundSocket: Bound,
    B: FnOnce() -> Result<()>,
    F: FnOnce(&BoundSocket, &UnboundSocket::Endpoint) -> Result<R>,
{
    let mut inner = inner_mutex.read();

    // Not really a loop, since we always break on the first iteration. But we need to use
    // `loop` here because we want to use `break` later.
    #[expect(clippy::never_loop)]
    let bound_datagram = loop {
        // Fast path: The socket is already bound.
        if let Inner::Bound(bound_datagram) = &*inner {
            break bound_datagram;
        }

        // Slow path: Try to bind the socket to an ephemeral endpoint.
        drop(inner);
        bind_ephemeral()?;
        inner = inner_mutex.read();

        // Now the socket must be bound.
        if let Inner::Bound(bound_datagram) = &*inner {
            break bound_datagram;
        }
        unreachable!("`try_bind_ephemeral` succeeds so the socket cannot be unbound");
    };

    let remote_endpoint = remote
        .or_else(|| bound_datagram.remote_endpoint())
        .ok_or_else(|| {
            Error::with_message(
                Errno::EDESTADDRREQ,
                "the destination address is not specified",
            )
        })?;

    op(bound_datagram, remote_endpoint)
}
