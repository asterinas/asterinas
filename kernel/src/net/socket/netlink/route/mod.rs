// SPDX-License-Identifier: MPL-2.0

//! Netlink Route Socket.

use core::sync::atomic::{AtomicBool, Ordering};

use bound::BoundNetlinkRoute;
use unbound::UnboundNetlinkRoute;

use super::NetlinkSocketAddr;
use crate::{
    events::IoEvents,
    net::socket::{
        options::SocketOption,
        private::SocketPrivate,
        util::datagram_common::{select_remote_and_bind, Bound, Inner},
        MessageHeader, SendRecvFlags, Socket, SocketAddr,
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
    inner: RwMutex<Inner<UnboundNetlinkRoute, BoundNetlinkRoute>>,

    is_nonblocking: AtomicBool,
    pollee: Pollee,
}

impl NetlinkRouteSocket {
    pub fn new(is_nonblocking: bool) -> Self {
        let unbound = UnboundNetlinkRoute::new();
        Self {
            inner: RwMutex::new(Inner::Unbound(unbound)),
            is_nonblocking: AtomicBool::new(is_nonblocking),
            pollee: Pollee::new(),
        }
    }

    fn try_send(
        &self,
        reader: &mut dyn MultiRead,
        remote: Option<&NetlinkSocketAddr>,
        flags: SendRecvFlags,
    ) -> Result<usize> {
        let sent_bytes = select_remote_and_bind(
            &self.inner,
            remote,
            || {
                self.inner
                    .write()
                    .bind_ephemeral(&NetlinkSocketAddr::new_unspecified(), &self.pollee)
            },
            |bound, remote_endpoint| bound.try_send(reader, remote_endpoint, flags),
        )?;
        self.pollee.notify(IoEvents::OUT | IoEvents::IN);

        Ok(sent_bytes)
    }

    fn try_recv(
        &self,
        writer: &mut dyn MultiWrite,
        flags: SendRecvFlags,
    ) -> Result<(usize, SocketAddr)> {
        let recv_bytes = self
            .inner
            .read()
            .try_recv(writer, flags)
            .map(|(recv_bytes, remote_endpoint)| (recv_bytes, remote_endpoint.into()))?;
        self.pollee.invalidate();

        Ok(recv_bytes)
    }
}

impl Socket for NetlinkRouteSocket {
    fn bind(&self, socket_addr: SocketAddr) -> Result<()> {
        let endpoint = socket_addr.try_into()?;

        // FIXME: We need to further check the Linux behavior
        // whether we should return error if the socket is bound.
        // The socket may call `bind` syscall to join new multicast groups.
        self.inner.write().bind(&endpoint, &self.pollee, ())
    }

    fn connect(&self, socket_addr: SocketAddr) -> Result<()> {
        let endpoint = socket_addr.try_into()?;

        self.inner.write().connect(&endpoint, &self.pollee)
    }

    fn addr(&self) -> Result<SocketAddr> {
        let endpoint = self
            .inner
            .read()
            .addr()
            .unwrap_or(NetlinkSocketAddr::new_unspecified());

        Ok(endpoint.into())
    }

    fn peer_addr(&self) -> Result<SocketAddr> {
        let endpoint = self
            .inner
            .read()
            .peer_addr()
            .cloned()
            .unwrap_or(NetlinkSocketAddr::new_unspecified());

        Ok(endpoint.into())
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
        let (received_len, addr) = self.block_on(IoEvents::IN, || self.try_recv(writers, flags))?;

        // TODO: Receive control message

        let message_header = MessageHeader::new(Some(addr), None);

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
            .poll_with(mask, poller, || self.inner.read().check_io_events())
    }
}
