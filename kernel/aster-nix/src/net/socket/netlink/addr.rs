// SPDX-License-Identifier: MPL-2.0

use super::multicast_group::GroupIdSet;

/// The socket addr of a netlink socket.
///
/// The addr contains the port num for unitcast
/// and the group ids for multicast.
#[derive(Debug, Clone, Copy)]
pub struct NetlinkSocketAddr {
    port: PortNum,
    groups: GroupIdSet,
}

impl NetlinkSocketAddr {
    /// Creates a new netlink addr.
    pub const fn new(port: PortNum, groups: GroupIdSet) -> Self {
        Self { port, groups }
    }

    /// Creates a new unspecified address.
    ///
    /// Both port ID and groups are left unspecified.
    pub const fn new_unspecified() -> Self {
        Self {
            port: UNSPECIFIED_PORT,
            groups: GroupIdSet::new_empty(),
        }
    }

    /// Returns the port number
    pub const fn port(&self) -> PortNum {
        self.port
    }

    /// Returns the group id set
    pub const fn groups(&self) -> GroupIdSet {
        self.groups
    }
}

pub type FamilyId = u32;
pub type PortNum = u32;
pub type GroupId = u32;

pub const UNSPECIFIED_PORT: PortNum = 0;
