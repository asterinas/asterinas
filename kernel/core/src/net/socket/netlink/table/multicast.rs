// SPDX-License-Identifier: MPL-2.0

use crate::{
    net::socket::netlink::{addr::PortNum, receiver::QueueableMessage},
    prelude::*,
};

/// A netlink multicast group.
///
/// A group can contain multiple sockets,
/// each identified by its bound port number.
pub(crate) struct MulticastGroup {
    members: BTreeSet<PortNum>,
}

impl MulticastGroup {
    /// Creates a new multicast group.
    pub(crate) const fn new() -> Self {
        Self {
            members: BTreeSet::new(),
        }
    }

    /// Adds a new member to the multicast group.
    pub(crate) fn add_member(&mut self, port_num: PortNum) {
        self.members.insert(port_num);
    }

    /// Removes a member from the multicast group.
    pub(crate) fn remove_member(&mut self, port_num: PortNum) {
        self.members.remove(&port_num);
    }

    /// Returns all members in this group.
    pub(crate) fn members(&self) -> &BTreeSet<PortNum> {
        &self.members
    }
}

pub(crate) trait MulticastMessage: QueueableMessage + Clone {}
