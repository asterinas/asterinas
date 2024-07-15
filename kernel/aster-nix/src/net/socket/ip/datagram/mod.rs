// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::{AtomicBool, Ordering};

use takeable::Takeable;

use self::{bound::BoundDatagram, unbound::UnboundDatagram};
use super::{common::get_ephemeral_endpoint, UNSPECIFIED_LOCAL_ENDPOINT};
use crate::{
    events::{IoEvents, Observer},
    fs::{file_handle::FileLike, utils::StatusFlags},
    net::{
        iface::IpEndpoint,
        poll_ifaces,
        socket::{
            util::{
                copy_message_from_user, copy_message_to_user, create_message_buffer,
                send_recv_flags::SendRecvFlags, socket_addr::SocketAddr, MessageHeader,
            },
            Socket,
        },
    },
    prelude::*,
    process::signal::{Pollee, Poller},
    util::IoVec,
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
            unbound_datagram.init_pollee(&pollee);
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

    fn remote_endpoint(&self) -> Option<IpEndpoint> {
        let inner = self.inner.read();

        match inner.as_ref() {
            Inner::Bound(bound_datagram) => bound_datagram.remote_endpoint(),
            Inner::Unbound(_) => None,
        }
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

    fn try_recv(&self, buf: &mut [u8], flags: SendRecvFlags) -> Result<(usize, SocketAddr)> {
        let inner = self.inner.read();

        let Inner::Bound(bound_datagram) = inner.as_ref() else {
            return_errno_with_message!(Errno::EAGAIN, "the socket is not bound");
        };

        let received = bound_datagram
            .try_recv(buf, flags)
            .map(|(recv_bytes, remote_endpoint)| {
                bound_datagram.update_io_events(&self.pollee);
                (recv_bytes, remote_endpoint.into())
            });

        drop(inner);
        poll_ifaces();

        received
    }

    fn recv(&self, buf: &mut [u8], flags: SendRecvFlags) -> Result<(usize, SocketAddr)> {
        if self.is_nonblocking() {
            self.try_recv(buf, flags)
        } else {
            self.wait_events(IoEvents::IN, || self.try_recv(buf, flags))
        }
    }

    fn try_send(&self, buf: &[u8], remote: &IpEndpoint, flags: SendRecvFlags) -> Result<usize> {
        let inner = self.inner.read();

        let Inner::Bound(bound_datagram) = inner.as_ref() else {
            return_errno_with_message!(Errno::EAGAIN, "the socket is not bound")
        };

        let sent_bytes = bound_datagram
            .try_send(buf, remote, flags)
            .map(|sent_bytes| {
                bound_datagram.update_io_events(&self.pollee);
                sent_bytes
            });

        drop(inner);
        poll_ifaces();

        sent_bytes
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
        // TODO: set correct flags
        let flags = SendRecvFlags::empty();
        self.recv(buf, flags).map(|(len, _)| len)
    }

    fn write(&self, buf: &[u8]) -> Result<usize> {
        let remote = self.remote_endpoint().ok_or_else(|| {
            Error::with_message(
                Errno::EDESTADDRREQ,
                "the destination address is not specified",
            )
        })?;

        // TODO: Set correct flags
        let flags = SendRecvFlags::empty();

        // TODO: Block if send buffer is full
        self.try_send(buf, &remote, flags)
    }

    fn poll(&self, mask: IoEvents, poller: Option<&Poller>) -> IoEvents {
        self.pollee.poll(mask, poller)
    }

    fn as_socket(self: Arc<Self>) -> Option<Arc<dyn Socket>> {
        Some(self)
    }

    fn status_flags(&self) -> StatusFlags {
        // TODO: when we fully support O_ASYNC, return the flag
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

    fn register_observer(
        &self,
        observer: Weak<dyn Observer<IoEvents>>,
        mask: IoEvents,
    ) -> Result<()> {
        self.pollee.register_observer(observer, mask);
        Ok(())
    }

    fn unregister_observer(
        &self,
        observer: &Weak<dyn Observer<IoEvents>>,
    ) -> Option<Weak<dyn Observer<IoEvents>>> {
        self.pollee.unregister_observer(observer)
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
        match inner.as_ref() {
            Inner::Unbound(_) => Ok(UNSPECIFIED_LOCAL_ENDPOINT.into()),
            Inner::Bound(bound_datagram) => Ok(bound_datagram.local_endpoint().into()),
        }
    }

    fn peer_addr(&self) -> Result<SocketAddr> {
        self.remote_endpoint()
            .map(|endpoint| endpoint.into())
            .ok_or_else(|| Error::with_message(Errno::ENOTCONN, "the socket is not connected"))
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
            addr,
            control_message,
        } = message_header;

        let remote_endpoint = match addr {
            Some(remote_addr) => {
                let endpoint = remote_addr.try_into()?;
                self.try_bind_empheral(&endpoint)?;
                endpoint
            }
            None => self.remote_endpoint().ok_or_else(|| {
                Error::with_message(
                    Errno::EDESTADDRREQ,
                    "the destination address is not specified",
                )
            })?,
        };

        if control_message.is_some() {
            // TODO: Support sending control message
            warn!("sending control message is not supported");
        }

        let buf = copy_message_from_user(io_vecs);

        // TODO: Block if the send buffer is full
        self.try_send(&buf, &remote_endpoint, flags)
    }

    fn recvmsg(&self, io_vecs: &[IoVec], flags: SendRecvFlags) -> Result<(usize, MessageHeader)> {
        // TODO: Deal with flags
        debug_assert!(flags.is_all_supported());

        let mut buf = create_message_buffer(io_vecs);

        let (received_bytes, peer_addr) = self.recv(&mut buf, flags)?;

        let copied_bytes = {
            let message = &buf[..received_bytes];
            copy_message_to_user(io_vecs, message)
        };

        // TODO: Receive control message

        let message_header = MessageHeader::new(Some(peer_addr), None);

        Ok((copied_bytes, message_header))
    }
}

impl Observer<()> for DatagramSocket {
    fn on_events(&self, _events: &()) {
        self.update_io_events();
    }
}
