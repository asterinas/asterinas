// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::{AtomicBool, Ordering};

use aster_bigtcp::wire::IpEndpoint;
use ostd::sync::PreemptDisabled;
use takeable::Takeable;

use self::{bound::BoundDatagram, unbound::UnboundDatagram};
use super::{common::get_ephemeral_endpoint, UNSPECIFIED_LOCAL_ENDPOINT};
use crate::{
    events::IoEvents,
    match_sock_option_mut,
    net::socket::{
        options::{Error as SocketError, SocketOption},
        private::SocketPrivate,
        util::{
            options::{SetSocketLevelOption, SocketOptionSet},
            send_recv_flags::SendRecvFlags,
            socket_addr::SocketAddr,
            MessageHeader,
        },
        Socket,
    },
    prelude::*,
    process::signal::{PollHandle, Pollable, Pollee},
    util::{MultiRead, MultiWrite},
};

mod bound;
mod observer;
mod unbound;

pub(in crate::net) use self::observer::DatagramObserver;

#[derive(Debug, Clone)]
struct OptionSet {
    socket: SocketOptionSet,
    // TODO: UDP option set
}

impl OptionSet {
    fn new() -> Self {
        let socket = SocketOptionSet::new_udp();
        OptionSet { socket }
    }
}

pub struct DatagramSocket {
    // Lock order: `inner` first, `options` second
    inner: RwLock<Takeable<Inner>, PreemptDisabled>,
    options: RwLock<OptionSet>,

    is_nonblocking: AtomicBool,
    pollee: Pollee,
}

enum Inner {
    Unbound(UnboundDatagram),
    Bound(BoundDatagram),
}

impl Inner {
    fn bind(
        self,
        endpoint: &IpEndpoint,
        can_reuse: bool,
        observer: DatagramObserver,
    ) -> core::result::Result<BoundDatagram, (Error, Self)> {
        let unbound_datagram = match self {
            Inner::Unbound(unbound_datagram) => unbound_datagram,
            Inner::Bound(bound_datagram) => {
                return Err((
                    Error::with_message(Errno::EINVAL, "the socket is already bound to an address"),
                    Inner::Bound(bound_datagram),
                ));
            }
        };

        let bound_datagram = match unbound_datagram.bind(endpoint, can_reuse, observer) {
            Ok(bound_datagram) => bound_datagram,
            Err((err, unbound_datagram)) => return Err((err, Inner::Unbound(unbound_datagram))),
        };
        Ok(bound_datagram)
    }

    fn bind_to_ephemeral_endpoint(
        self,
        remote_endpoint: &IpEndpoint,
        observer: DatagramObserver,
    ) -> core::result::Result<BoundDatagram, (Error, Self)> {
        if let Inner::Bound(bound_datagram) = self {
            return Ok(bound_datagram);
        }

        let endpoint = get_ephemeral_endpoint(remote_endpoint);
        self.bind(&endpoint, false, observer)
    }
}

impl DatagramSocket {
    pub fn new(is_nonblocking: bool) -> Arc<Self> {
        let unbound_datagram = UnboundDatagram::new();
        Arc::new(Self {
            inner: RwLock::new(Takeable::new(Inner::Unbound(unbound_datagram))),
            options: RwLock::new(OptionSet::new()),
            is_nonblocking: AtomicBool::new(is_nonblocking),
            pollee: Pollee::new(),
        })
    }

    fn try_bind_ephemeral(&self, remote_endpoint: &IpEndpoint) -> Result<()> {
        // Fast path
        if let Inner::Bound(_) = self.inner.read().as_ref() {
            return Ok(());
        }

        // Slow path
        let mut inner = self.inner.write();
        inner.borrow_result(|owned_inner| {
            let bound_datagram = match owned_inner.bind_to_ephemeral_endpoint(
                remote_endpoint,
                DatagramObserver::new(self.pollee.clone()),
            ) {
                Ok(bound_datagram) => bound_datagram,
                Err((err, err_inner)) => {
                    return (err_inner, Err(err));
                }
            };
            (Inner::Bound(bound_datagram), Ok(()))
        })
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
    fn select_remote_and_bind<F, R>(&self, remote: Option<&IpEndpoint>, op: F) -> Result<R>
    where
        F: FnOnce(&BoundDatagram, &IpEndpoint) -> Result<R>,
    {
        let mut inner = self.inner.read();

        // Not really a loop, since we always break on the first iteration. But we need to use
        // `loop` here because we want to use `break` later.
        #[expect(clippy::never_loop)]
        let bound_datagram = loop {
            // Fast path: The socket is already bound.
            if let Inner::Bound(bound_datagram) = inner.as_ref() {
                break bound_datagram;
            }

            // Slow path: Try to bind the socket to an ephemeral endpoint.
            drop(inner);
            if let Some(remote_endpoint) = remote {
                self.try_bind_ephemeral(remote_endpoint)?;
            } else {
                return_errno_with_message!(
                    Errno::EDESTADDRREQ,
                    "the destination address is not specified"
                );
            }
            inner = self.inner.read();

            // Now the socket must be bound.
            if let Inner::Bound(bound_datagram) = inner.as_ref() {
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

    fn try_recv(
        &self,
        writer: &mut dyn MultiWrite,
        flags: SendRecvFlags,
    ) -> Result<(usize, SocketAddr)> {
        let inner = self.inner.read();

        let Inner::Bound(bound_datagram) = inner.as_ref() else {
            return_errno_with_message!(Errno::EAGAIN, "the socket is not bound");
        };

        let recv_bytes = bound_datagram
            .try_recv(writer, flags)
            .map(|(recv_bytes, remote_endpoint)| (recv_bytes, remote_endpoint.into()))?;
        self.pollee.invalidate();

        Ok(recv_bytes)
    }

    fn try_send(
        &self,
        reader: &mut dyn MultiRead,
        remote: Option<&IpEndpoint>,
        flags: SendRecvFlags,
    ) -> Result<usize> {
        let (sent_bytes, iface_to_poll) =
            self.select_remote_and_bind(remote, |bound_datagram, remote_endpoint| {
                let sent_bytes = bound_datagram.try_send(reader, remote_endpoint, flags)?;
                let iface_to_poll = bound_datagram.iface().clone();
                Ok((sent_bytes, iface_to_poll))
            })?;

        self.pollee.invalidate();
        iface_to_poll.poll();

        Ok(sent_bytes)
    }

    fn check_io_events(&self) -> IoEvents {
        let inner = self.inner.read();

        match inner.as_ref() {
            Inner::Unbound(unbound_datagram) => unbound_datagram.check_io_events(),
            Inner::Bound(bound_socket) => bound_socket.check_io_events(),
        }
    }
}

impl Pollable for DatagramSocket {
    fn poll(&self, mask: IoEvents, poller: Option<&mut PollHandle>) -> IoEvents {
        self.pollee
            .poll_with(mask, poller, || self.check_io_events())
    }
}

impl SocketPrivate for DatagramSocket {
    fn is_nonblocking(&self) -> bool {
        self.is_nonblocking.load(Ordering::Relaxed)
    }

    fn set_nonblocking(&self, is_nonblocking: bool) {
        self.is_nonblocking.store(is_nonblocking, Ordering::Relaxed);
    }
}

impl Socket for DatagramSocket {
    fn bind(&self, socket_addr: SocketAddr) -> Result<()> {
        let endpoint = socket_addr.try_into()?;

        let mut inner = self.inner.write();
        let can_reuse = self.options.read().socket.reuse_addr();
        inner.borrow_result(|owned_inner| {
            let bound_datagram = match owned_inner.bind(
                &endpoint,
                can_reuse,
                DatagramObserver::new(self.pollee.clone()),
            ) {
                Ok(bound_datagram) => bound_datagram,
                Err((err, err_inner)) => {
                    return (err_inner, Err(err));
                }
            };
            (Inner::Bound(bound_datagram), Ok(()))
        })
    }

    fn connect(&self, socket_addr: SocketAddr) -> Result<()> {
        let endpoint = socket_addr.try_into()?;

        self.try_bind_ephemeral(&endpoint)?;

        let mut inner = self.inner.write();
        let Inner::Bound(bound_datagram) = inner.as_mut() else {
            return_errno_with_message!(Errno::EINVAL, "the socket is not bound")
        };
        bound_datagram.set_remote_endpoint(&endpoint);

        Ok(())
    }

    fn addr(&self) -> Result<SocketAddr> {
        let inner = self.inner.read();
        match inner.as_ref() {
            Inner::Unbound(_) => Ok(UNSPECIFIED_LOCAL_ENDPOINT.into()),
            Inner::Bound(bound_datagram) => Ok(bound_datagram.local_endpoint().into()),
        }
    }

    fn peer_addr(&self) -> Result<SocketAddr> {
        let inner = self.inner.read();

        let remote_endpoint = match inner.as_ref() {
            Inner::Unbound(_) => None,
            Inner::Bound(bound_datagram) => bound_datagram.remote_endpoint(),
        };

        remote_endpoint
            .map(|endpoint| (*endpoint).into())
            .ok_or_else(|| Error::with_message(Errno::ENOTCONN, "the socket is not connected"))
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
            addr,
            control_message,
        } = message_header;

        let endpoint = match addr {
            Some(addr) => Some(addr.try_into()?),
            None => None,
        };

        if control_message.is_some() {
            // TODO: Support sending control message
            warn!("sending control message is not supported");
        }

        // TODO: Block if the send buffer is full
        self.try_send(reader, endpoint.as_ref(), flags)
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

        let (received_bytes, peer_addr) =
            self.block_on(IoEvents::IN, || self.try_recv(writer, flags))?;

        // TODO: Receive control message

        let message_header = MessageHeader::new(Some(peer_addr), None);

        Ok((received_bytes, message_header))
    }

    fn get_option(&self, option: &mut dyn SocketOption) -> Result<()> {
        match_sock_option_mut!(option, {
            socket_errors: SocketError => {
                // TODO: Support socket errors for UDP sockets
                socket_errors.set(None);
                return Ok(());
            },
            _ => ()
        });

        self.options.read().socket.get_option(option)
    }

    fn set_option(&self, option: &dyn SocketOption) -> Result<()> {
        let inner = self.inner.read();
        let mut options = self.options.write();

        match options.socket.set_option(option, inner.as_ref()) {
            Err(e) => Err(e),
            Ok(need_iface_poll) => {
                let iface_to_poll = need_iface_poll
                    .then(|| match inner.as_ref() {
                        Inner::Unbound(_) => None,
                        Inner::Bound(bound_datagram) => Some(bound_datagram.iface().clone()),
                    })
                    .flatten();

                drop(inner);
                drop(options);

                if let Some(iface) = iface_to_poll {
                    iface.poll();
                }

                Ok(())
            }
        }
    }
}

impl SetSocketLevelOption for Inner {}
