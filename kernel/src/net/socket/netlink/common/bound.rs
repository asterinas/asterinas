// SPDX-License-Identifier: MPL-2.0

use crate::{
    events::IoEvents,
    net::socket::netlink::{
        receiver::MessageQueue, table::BoundHandle, GroupIdSet, NetlinkSocketAddr,
    },
    prelude::*,
};

pub struct BoundNetlink<Message: 'static> {
    pub(in crate::net::socket::netlink) handle: BoundHandle<Message>,
    pub(in crate::net::socket::netlink) remote_addr: NetlinkSocketAddr,
    pub(in crate::net::socket::netlink) receive_queue: MessageQueue<Message>,
}

impl<Message: 'static> BoundNetlink<Message> {
    pub(super) fn new(handle: BoundHandle<Message>, message_queue: MessageQueue<Message>) -> Self {
        Self {
            handle,
            remote_addr: NetlinkSocketAddr::new_unspecified(),
            receive_queue: message_queue,
        }
    }

    pub(in crate::net::socket::netlink) fn bind_common(
        &mut self,
        endpoint: &NetlinkSocketAddr,
    ) -> Result<()> {
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

    pub(in crate::net::socket::netlink) fn check_io_events_common(&self) -> IoEvents {
        let mut events = IoEvents::OUT;

        let receive_queue = self.receive_queue.0.lock();
        if !receive_queue.is_empty() {
            events |= IoEvents::IN;
        }

        events
    }

    pub(super) fn add_groups(&mut self, groups: GroupIdSet) {
        self.handle.add_groups(groups);
    }

    pub(super) fn drop_groups(&mut self, groups: GroupIdSet) {
        self.handle.drop_groups(groups);
    }
}
