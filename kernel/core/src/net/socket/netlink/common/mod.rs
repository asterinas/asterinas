// SPDX-License-Identifier: MPL-2.0

pub(super) use bound::BoundNetlink;
use unbound::UnboundNetlink;

use super::{GroupIdSet, NetlinkSocketAddr};
use crate::{
    events::IoEvents,
    fs::{
        file::{FileCommon, StatusFlags},
        pseudofs::SockFs,
    },
    net::socket::{
        Socket,
        netlink::{AddMembership, DropMembership, table::SupportedNetlinkProtocol},
        options::{
            Error as SocketError, SocketOption,
            macros::{sock_option_mut, sock_option_ref},
        },
        private::SocketPrivate,
        util::{
            MessageHeader, SendRecvFlags, SocketAddr,
            datagram_common::{Bound, Inner, select_remote_and_bind},
            options::{
                GetSocketLevelOption, SetSocketLevelOption, SocketOptionSet, SocketTimeouts,
            },
        },
    },
    prelude::*,
    process::signal::{PollHandle, Pollable, Pollee},
    util::{MultiRead, MultiWrite, net::SockType},
};

mod bound;
mod unbound;

pub struct NetlinkSocket<P: SupportedNetlinkProtocol> {
    inner: RwMutex<Inner<UnboundNetlink<P>, BoundNetlink<P::Message>>>,
    options: RwLock<OptionSet>,
    socket_type: SockType,
    timeouts: SocketTimeouts,

    pollee: Pollee,
    common: FileCommon,
}

#[derive(Clone, Debug)]
struct OptionSet {
    socket: SocketOptionSet,
}

impl OptionSet {
    pub(self) fn new() -> Self {
        Self {
            socket: SocketOptionSet::new_netlink(),
        }
    }
}

impl<P: SupportedNetlinkProtocol> NetlinkSocket<P>
where
    BoundNetlink<P::Message>: Bound<Endpoint = NetlinkSocketAddr>,
{
    pub fn new(is_nonblocking: bool, socket_type: SockType) -> Arc<Self> {
        debug_assert!(socket_type == SockType::SOCK_RAW || socket_type == SockType::SOCK_DGRAM);

        let unbound = UnboundNetlink::new();
        let status_flags = if is_nonblocking {
            StatusFlags::O_NONBLOCK
        } else {
            StatusFlags::empty()
        };
        Arc::new(Self {
            inner: RwMutex::new(Inner::Unbound(unbound)),
            options: RwLock::new(OptionSet::new()),
            socket_type,
            timeouts: SocketTimeouts::new(),
            pollee: Pollee::new(),
            common: FileCommon::new(SockFs::new_path(), status_flags),
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
        writer: &mut dyn MultiWrite,
        flags: SendRecvFlags,
    ) -> Result<(usize, MessageHeader)> {
        let (received_len, addr) =
            self.block_on(IoEvents::IN, self.timeouts.recv_timeout(), || {
                self.try_recv(writer, flags)
            })?;

        // TODO: Receive control message

        let message_header = MessageHeader::new(Some(addr), Vec::new());

        Ok((received_len, message_header))
    }

    fn get_option(&self, option: &mut dyn SocketOption) -> Result<()> {
        sock_option_mut!(match option {
            socket_errors @ SocketError => {
                // TODO: Support socket errors for netlink sockets
                socket_errors.set(None);
                return Ok(());
            }
            _ => (),
        });

        let inner = self.inner.read();
        let options = self.options.read();

        // Deal with socket-level options
        options
            .socket
            .get_option(option, &(&*inner, self.socket_type, &self.timeouts))

        // TODO: Deal with netlink-level options
    }

    fn set_option(&self, option: &dyn SocketOption) -> Result<()> {
        let mut inner = self.inner.write();

        // Deal with socket-level options
        let mut options = self.options.write();
        match options
            .socket
            .set_option(option, &(&*inner, &self.timeouts))
        {
            Err(err) if err.error() == Errno::ENOPROTOOPT => (),
            res => return res.map(|_need_iface_poll| ()),
        }
        // `options` must be dropped here because `do_netlink_setsockopt` may lock other mutexes.
        drop(options);

        // Deal with netlink-level options
        do_netlink_setsockopt(option, &mut inner)
    }

    fn common(&self) -> &FileCommon {
        &self.common
    }
}

impl<P: SupportedNetlinkProtocol> SocketPrivate for NetlinkSocket<P>
where
    BoundNetlink<P::Message>: Bound<Endpoint = NetlinkSocketAddr>,
{
    fn is_nonblocking(&self) -> bool {
        self.common.is_nonblocking()
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

impl<P: SupportedNetlinkProtocol> GetSocketLevelOption
    for (
        &Inner<UnboundNetlink<P>, BoundNetlink<P::Message>>,
        SockType,
        &SocketTimeouts,
    )
{
    fn socket_type(&self) -> SockType {
        self.1
    }

    fn is_listening(&self) -> bool {
        false
    }

    fn socket_timeouts(&self) -> Option<&SocketTimeouts> {
        Some(self.2)
    }
}

impl<P: SupportedNetlinkProtocol> SetSocketLevelOption
    for (
        &Inner<UnboundNetlink<P>, BoundNetlink<P::Message>>,
        &SocketTimeouts,
    )
{
    fn socket_timeouts(&self) -> Option<&SocketTimeouts> {
        Some(self.1)
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

fn do_netlink_setsockopt<P: SupportedNetlinkProtocol>(
    option: &dyn SocketOption,
    inner: &mut Inner<UnboundNetlink<P>, BoundNetlink<P::Message>>,
) -> Result<()> {
    sock_option_ref!(match option {
        add_membership @ AddMembership => {
            let groups = add_membership.get().unwrap();
            inner.add_groups(GroupIdSet::new(*groups));
        }
        drop_membership @ DropMembership => {
            let groups = drop_membership.get().unwrap();
            inner.drop_groups(GroupIdSet::new(*groups));
        }
        _ =>
            return_errno_with_message!(Errno::ENOPROTOOPT, "the socket option to be set is unknown"),
    });

    Ok(())
}
