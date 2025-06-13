// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::{AtomicBool, Ordering};

pub(super) use bound::BoundNetlink;
use unbound::UnboundNetlink;

use super::{GroupIdSet, NetlinkSocketAddr};
use crate::{
    events::IoEvents,
    match_sock_option_ref,
    net::socket::{
        netlink::{table::SupportedNetlinkProtocol, AddMembership, DropMembership},
        options::SocketOption,
        private::SocketPrivate,
        util::{
            datagram_common::{select_remote_and_bind, Bound, Inner},
            MessageHeader, SendRecvFlags, SocketAddr,
        },
        Socket,
    },
    prelude::*,
    process::signal::{PollHandle, Pollable, Pollee},
    util::{MultiRead, MultiWrite},
};

mod bound;
mod unbound;

pub struct NetlinkSocket<P: SupportedNetlinkProtocol> {
    inner: RwMutex<Inner<UnboundNetlink<P>, BoundNetlink<P::Message>>>,

    is_nonblocking: AtomicBool,
    pollee: Pollee,
}

impl<P: SupportedNetlinkProtocol> NetlinkSocket<P>
where
    BoundNetlink<P::Message>: Bound<Endpoint = NetlinkSocketAddr>,
{
    pub fn new(is_nonblocking: bool) -> Arc<Self> {
        let unbound = UnboundNetlink::new();
        Arc::new(Self {
            inner: RwMutex::new(Inner::Unbound(unbound)),
            is_nonblocking: AtomicBool::new(is_nonblocking),
            pollee: Pollee::new(),
        })
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
        self.pollee.invalidate();

        Ok(sent_bytes)
    }

    // FIXME: This method is marked as `pub(super)` because it's invoked during kernel mode testing.
    pub(super) fn try_recv(
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

impl<P: SupportedNetlinkProtocol> Socket for NetlinkSocket<P>
where
    BoundNetlink<P::Message>: Bound<Endpoint = NetlinkSocketAddr>,
{
    fn bind(&self, socket_addr: SocketAddr) -> Result<()> {
        let endpoint = socket_addr.try_into()?;

        self.inner.write().bind(&endpoint, &self.pollee, ())
    }

    fn connect(&self, socket_addr: SocketAddr) -> Result<()> {
        let endpoint = socket_addr.try_into()?;

        self.inner.write().connect(&endpoint, &self.pollee)
    }

    fn addr(&self) -> Result<SocketAddr> {
        let endpoint = match &*self.inner.read() {
            Inner::Unbound(unbound) => unbound.addr(),
            Inner::Bound(bound) => bound.local_endpoint(),
        };

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
            control_messages,
        } = message_header;

        let remote = match addr {
            None => None,
            Some(addr) => Some(addr.try_into()?),
        };

        if !control_messages.is_empty() {
            // TODO: Support sending control message
            warn!("sending control message is not supported");
        }

        if reader.is_empty() {
            // Based on how Linux behaves, zero-sized messages are not allowed for netlink sockets.
            return_errno_with_message!(Errno::ENODATA, "there are no data to send");
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

        let message_header = MessageHeader::new(Some(addr), Vec::new());

        Ok((received_len, message_header))
    }

    fn set_option(&self, option: &dyn SocketOption) -> Result<()> {
        match do_set_netlink_option(&self.inner, option) {
            Ok(()) => Ok(()),
            Err(e) => {
                warn!(
                    "We currently ignore set option errors to pass libnl test: {:?}",
                    e
                );
                Ok(())
            }
        }
    }
}

impl<P: SupportedNetlinkProtocol> SocketPrivate for NetlinkSocket<P>
where
    BoundNetlink<P::Message>: Bound<Endpoint = NetlinkSocketAddr>,
{
    fn is_nonblocking(&self) -> bool {
        self.is_nonblocking.load(Ordering::Relaxed)
    }

    fn set_nonblocking(&self, nonblocking: bool) {
        self.is_nonblocking.store(nonblocking, Ordering::Relaxed);
    }
}

impl<P: SupportedNetlinkProtocol> Pollable for NetlinkSocket<P>
where
    BoundNetlink<P::Message>: Bound<Endpoint = NetlinkSocketAddr>,
{
    fn poll(&self, mask: IoEvents, poller: Option<&mut PollHandle>) -> IoEvents {
        self.pollee
            .poll_with(mask, poller, || self.inner.read().check_io_events())
    }
}

impl<P: SupportedNetlinkProtocol> Inner<UnboundNetlink<P>, BoundNetlink<P::Message>> {
    fn add_groups(&mut self, groups: GroupIdSet) {
        match self {
            Inner::Unbound(unbound_socket) => unbound_socket.add_groups(groups),
            Inner::Bound(bound_socket) => bound_socket.add_groups(groups),
        }
    }

    fn drop_groups(&mut self, groups: GroupIdSet) {
        match self {
            Inner::Unbound(unbound_socket) => unbound_socket.drop_groups(groups),
            Inner::Bound(bound_socket) => bound_socket.drop_groups(groups),
        }
    }
}

fn do_set_netlink_option<P: SupportedNetlinkProtocol>(
    inner: &RwMutex<Inner<UnboundNetlink<P>, BoundNetlink<P::Message>>>,
    option: &dyn SocketOption,
) -> Result<()> {
    match_sock_option_ref!(option, {
        add_membership: AddMembership => {
            let groups = add_membership.get().unwrap();
            inner.write().add_groups(GroupIdSet::new(*groups));
        },
        drop_membership: DropMembership => {
            let groups = drop_membership.get().unwrap();
            inner.write().drop_groups(GroupIdSet::new(*groups));
        },
        _ => return_errno_with_message!(Errno::ENOPROTOOPT, "the socket option to be set is unknown")
    });

    Ok(())
}
