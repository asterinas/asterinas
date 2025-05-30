// SPDX-License-Identifier: MPL-2.0

use crate::{net::socket::netlink::addr::PortNum, prelude::*};

/// A netlink multicast group.
///
/// A group can contain multiple sockets,
/// each identified by its bound port number.
pub struct MulticastGroup {
    members: BTreeSet<PortNum>,
}

impl MulticastGroup {
    /// Creates a new multicast group.
    pub const fn new() -> Self {
        Self {
            members: BTreeSet::new(),
        }
    }

    /// Adds a new member to the multicast group.
    pub fn add_member(&mut self, port_num: PortNum) {
        self.members.insert(port_num);
    }

    /// Removes a member from the multicast group.
    pub fn remove_member(&mut self, port_num: PortNum) {
        self.members.remove(&port_num);
    }

    /// Returns all members in this group.
    pub fn members(&self) -> &BTreeSet<PortNum> {
        &self.members
    }
}

pub trait MulticastMessage: Clone {}
