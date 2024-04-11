// SPDX-License-Identifier: MPL-2.0

use super::multicast_group::GroupIdSet;

/// The socket addr of a netlink socket.
///
/// The addr contains the port num for unitcast
/// and the group ids for multicast.
#[derive(Debug)]
pub struct NetlinkSocketAddr {
    family: FamilyId,
    port: PortNum,
    groups: GroupIdSet,
}

impl NetlinkSocketAddr {
    pub const fn new(family: FamilyId, port: Option<PortNum>, groups: Option<GroupIdSet>) -> Self {
        todo!()
    }
}

pub type FamilyId = u32;
pub type PortNum = u32;
pub type GroupId = u32;
