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

    /// Returns whether the group contains a member.
    #[expect(unused)]
    pub fn contains_member(&self, port_num: PortNum) -> bool {
        self.members.contains(&port_num)
    }

    /// Adds a new member to the multicast group.
    pub fn add_member(&mut self, port_num: PortNum) {
        debug_assert!(!self.members.contains(&port_num));
        self.members.insert(port_num);
    }

    /// Removes a member from the multicast group.
    pub fn remove_member(&mut self, port_num: PortNum) {
        debug_assert!(self.members.contains(&port_num));
        self.members.remove(&port_num);
    }
}
