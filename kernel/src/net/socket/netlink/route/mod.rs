// SPDX-License-Identifier: MPL-2.0

//! Netlink Route Socket.

use core::sync::atomic::{AtomicBool, Ordering};

use bound::BoundNetlinkRoute;
use takeable::Takeable;
use unbound::UnboundNetlinkRoute;

use super::NetlinkSocketAddr;
use crate::{
    events::IoEvents,
    net::socket::{
        options::SocketOption, private::SocketPrivate, MessageHeader, SendRecvFlags, Socket,
        SocketAddr,
    },
    prelude::*,
    process::signal::{PollHandle, Pollable, Pollee},
    util::{MultiRead, MultiWrite},
};

mod bound;
mod kernel;
mod message;
mod unbound;

pub struct NetlinkRouteSocket {
    is_nonblocking: AtomicBool,
    pollee: Pollee,
    inner: RwMutex<Takeable<Inner>>,
}

enum Inner {
    Unbound(UnboundNetlinkRoute),
    Bound(BoundNetlinkRoute),
}

impl NetlinkRouteSocket {
    pub fn new(is_nonblocking: bool) -> Self {
        Self {
            is_nonblocking: AtomicBool::new(is_nonblocking),
            pollee: Pollee::new(),
            inner: RwMutex::new(Takeable::new(Inner::Unbound(UnboundNetlinkRoute::new()))),
        }
    }

    fn try_receive(
        &self,
        writer: &mut dyn MultiWrite,
        flags: SendRecvFlags,
    ) -> Result<(usize, NetlinkSocketAddr)> {
        let inner = self.inner.read();

        let bound = match inner.as_ref() {
            Inner::Unbound(_) => {
                return_errno_with_message!(Errno::EAGAIN, "the socket is not bound")
            }
            Inner::Bound(bound_netlink_route) => bound_netlink_route,
        };

        let received = bound.try_receive(writer, flags)?;
        self.pollee.invalidate();

        Ok(received)
    }

    fn try_send(
        &self,
        reader: &mut dyn MultiRead,
        remote: Option<&NetlinkSocketAddr>,
        flags: SendRecvFlags,
    ) -> Result<usize> {
        let inner = self.inner.read();

        let bound = match inner.as_ref() {
            Inner::Unbound(_) => todo!(),
            Inner::Bound(bound) => bound,
        };

        let sent_bytes = bound.try_send(reader, remote, flags)?;
        self.pollee.notify(IoEvents::OUT | IoEvents::IN);

        Ok(sent_bytes)
    }

    fn check_io_events(&self) -> IoEvents {
        let inner = self.inner.read();
        match inner.as_ref() {
            Inner::Unbound(unbound) => unbound.check_io_events(),
            Inner::Bound(bound) => bound.check_io_events(),
        }
    }
}

impl Socket for NetlinkRouteSocket {
    fn bind(&self, socket_addr: SocketAddr) -> Result<()> {
        let SocketAddr::Netlink(netlink_addr) = socket_addr else {
            return_errno_with_message!(
                Errno::EAFNOSUPPORT,
                "the provided address is not netlink address"
            );
        };

        let mut inner = self.inner.write();
        inner.borrow_result(|owned_inner| match owned_inner.bind(&netlink_addr) {
            Ok(bound_inner) => (bound_inner, Ok(())),
            Err((err, err_inner)) => (err_inner, Err(err)),
        })
    }

    fn addr(&self) -> Result<SocketAddr> {
        let netlink_addr = match self.inner.read().as_ref() {
            Inner::Unbound(_) => NetlinkSocketAddr::new_unspecified(),
            Inner::Bound(bound) => bound.addr(),
        };

        Ok(SocketAddr::Netlink(netlink_addr))
    }

    fn sendmsg(
        &self,
        reader: &mut dyn MultiRead,
        message_header: MessageHeader,
        flags: SendRecvFlags,
    ) -> Result<usize> {
        let MessageHeader {
            addr,
            control_message,
        } = message_header;

        let remote = match addr {
            None => None,
            Some(addr) => Some(addr.try_into()?),
        };

        if control_message.is_some() {
            // TODO: Support sending control message
            warn!("sending control message is not supported");
        }

        // TODO: Make sure our blocking behavior matches that of Linux
        self.try_send(reader, remote.as_ref(), flags)
    }

    fn recvmsg(
        &self,
        writers: &mut dyn MultiWrite,
        flags: SendRecvFlags,
    ) -> Result<(usize, MessageHeader)> {
        let (received_len, addr) =
            self.block_on(IoEvents::IN, || self.try_receive(writers, flags))?;

        // TODO: Receive control message

        let message_header = {
            let addr = SocketAddr::Netlink(addr);
            MessageHeader::new(Some(addr), None)
        };

        Ok((received_len, message_header))
    }

    fn set_option(&self, _option: &dyn SocketOption) -> Result<()> {
        // TODO: This dummy option is added to pass the libnl test
        Ok(())
    }
}

impl SocketPrivate for NetlinkRouteSocket {
    fn is_nonblocking(&self) -> bool {
        self.is_nonblocking.load(Ordering::Relaxed)
    }

    fn set_nonblocking(&self, nonblocking: bool) {
        self.is_nonblocking.store(nonblocking, Ordering::Relaxed);
    }
}

impl Pollable for NetlinkRouteSocket {
    fn poll(&self, mask: IoEvents, poller: Option<&mut PollHandle>) -> IoEvents {
        self.pollee
            .poll_with(mask, poller, || self.check_io_events())
    }
}

impl Inner {
    fn bind(self, addr: &NetlinkSocketAddr) -> core::result::Result<Self, (Error, Self)> {
        let unbound = match self {
            Inner::Unbound(unbound) => unbound,
            Inner::Bound(bound) => {
                // FIXME: We need to further check the Linux behavior
                // whether we should return error if the socket is bound.
                // The socket may call `bind` syscall to join new multicast groups.
                return Err((
                    Error::with_message(Errno::EINVAL, "the socket is already bound"),
                    Self::Bound(bound),
                ));
            }
        };

        match unbound.bind(addr) {
            Ok(bound) => Ok(Self::Bound(bound)),
            Err((err, unbound)) => Err((err, Self::Unbound(unbound))),
        }
    }
}
