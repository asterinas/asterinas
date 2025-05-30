// SPDX-License-Identifier: MPL-2.0

use core::marker::PhantomData;

use super::{
    addr::NetlinkProtocolId,
    table::{BoundHandle, NETLINK_SOCKET_TABLE},
    AnyNetlinkSocket, GroupIdSet, NetlinkSocketAddr,
};
use crate::{
    events::IoEvents,
    match_sock_option_ref,
    net::socket::{
        netlink::{AddMembership, DropMembership},
        options::SocketOption,
        util::datagram_common::{self, Inner},
    },
    prelude::*,
    process::signal::Pollee,
};

pub(super) struct UnboundNetlink<Message, const PROTOCOL: NetlinkProtocolId> {
    socket: AnyNetlinkSocket,
    groups: GroupIdSet,
    phantom: PhantomData<BoundNetlink<Message>>,
}

impl<Message, const PROTOCOL: NetlinkProtocolId> UnboundNetlink<Message, PROTOCOL> {
    pub(super) const fn new(socket: AnyNetlinkSocket) -> Self {
        Self {
            socket,
            groups: GroupIdSet::new_empty(),
            phantom: PhantomData,
        }
    }

    pub(super) fn addr(&self) -> NetlinkSocketAddr {
        NetlinkSocketAddr::new(0, self.groups)
    }

    pub(super) fn add_groups(&mut self, groups: GroupIdSet) {
        self.groups.add_groups(groups);
    }

    pub(super) fn drop_groups(&mut self, groups: GroupIdSet) {
        self.groups.drop_groups(groups);
    }
}

impl<Message, const PROTOCOL: NetlinkProtocolId> datagram_common::Unbound
    for UnboundNetlink<Message, PROTOCOL>
{
    type Endpoint = NetlinkSocketAddr;
    type BindOptions = ();

    type Bound = BoundNetlink<Message>;

    fn bind(
        &mut self,
        endpoint: &Self::Endpoint,
        _pollee: &Pollee,
        _options: Self::BindOptions,
    ) -> Result<Self::Bound> {
        let endpoint = endpoint.add_groups(self.groups);
        let bound_handle = NETLINK_SOCKET_TABLE.bind(PROTOCOL, &endpoint, self.socket.clone())?;

        Ok(BoundNetlink::new(bound_handle))
    }

    fn bind_ephemeral(
        &mut self,
        _remote_endpoint: &Self::Endpoint,
        _pollee: &Pollee,
    ) -> Result<Self::Bound> {
        let bound_handle = NETLINK_SOCKET_TABLE.bind(
            PROTOCOL,
            &NetlinkSocketAddr::new_unspecified(),
            self.socket.clone(),
        )?;

        Ok(BoundNetlink::new(bound_handle))
    }

    fn check_io_events(&self) -> IoEvents {
        IoEvents::OUT
    }
}

pub(super) struct BoundNetlink<Message> {
    pub(super) handle: BoundHandle,
    pub(super) remote_addr: NetlinkSocketAddr,
    pub(super) receive_queue: Mutex<VecDeque<Message>>,
}

impl<Message> BoundNetlink<Message> {
    pub(super) fn new(handle: BoundHandle) -> Self {
        Self {
            handle,
            remote_addr: NetlinkSocketAddr::new_unspecified(),
            receive_queue: Mutex::new(VecDeque::new()),
        }
    }

    pub(super) fn bind(&mut self, endpoint: &NetlinkSocketAddr) -> Result<()> {
        if endpoint.port() != self.handle.port() {
            return_errno_with_message!(
                Errno::EINVAL,
                "the socket cannot be bound to a different port"
            );
        }

        let groups = endpoint.groups();
        self.handle.bind_groups(groups);

        Ok(())
    }

    pub(super) fn enqueue_message(&self, message: Message) {
        // FIXME: We should verify the socket buffer length to ensure
        // that adding the message doesn't exceed the buffer capacity.
        let mut receive_queue = self.receive_queue.lock();
        receive_queue.push_back(message);
    }

    pub(super) fn add_groups(&mut self, groups: GroupIdSet) {
        self.handle.add_groups(groups);
    }

    pub(super) fn drop_groups(&mut self, groups: GroupIdSet) {
        self.handle.drop_groups(groups);
    }

    pub(super) fn check_io_events(&self) -> IoEvents {
        let mut events = IoEvents::OUT;

        let receive_queue = self.receive_queue.lock();
        if !receive_queue.is_empty() {
            events |= IoEvents::IN;
        }

        events
    }
}

impl<Message, const PROTOCOL: NetlinkProtocolId>
    Inner<UnboundNetlink<Message, PROTOCOL>, BoundNetlink<Message>>
{
    /// Enqueues a message to the socket.
    ///
    /// # Panics
    ///
    /// This function panics if the socket is not bound.
    pub(super) fn enqueue_message(&self, message: Message, pollee: &Pollee) -> Result<()> {
        let Inner::Bound(bound) = self else {
            unreachable!("[Internal Error]the socket is not bound");
        };
        bound.enqueue_message(message);
        pollee.notify(IoEvents::IN);

        Ok(())
    }

    pub(super) fn add_groups(&mut self, groups: GroupIdSet) {
        match self {
            Inner::Unbound(unbound_socket) => unbound_socket.add_groups(groups),
            Inner::Bound(bound_socket) => bound_socket.add_groups(groups),
        }
    }

    pub(super) fn drop_groups(&mut self, groups: GroupIdSet) {
        match self {
            Inner::Unbound(unbound_socket) => unbound_socket.drop_groups(groups),
            Inner::Bound(bound_socket) => bound_socket.drop_groups(groups),
        }
    }
}

pub(super) fn do_set_netlink_option<Message, const PROTOCOL: NetlinkProtocolId>(
    inner: &RwMutex<Inner<UnboundNetlink<Message, PROTOCOL>, BoundNetlink<Message>>>,
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
