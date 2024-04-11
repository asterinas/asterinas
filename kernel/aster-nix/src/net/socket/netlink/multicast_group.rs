// SPDX-License-Identifier: MPL-2.0

use super::{
    addr::{GroupId, PortNum},
    receiver::Receiver,
};
use crate::prelude::*;

/// A netlink multicast group.
///
/// Each group has a unique group ID,
/// which is a u32 value containing only one bit set to 1 and all other bits set to 0.
/// Each netlink protocol can have a maximum of 32 groups.
pub struct MuilicastGroup {
    group_id: GroupId,
    members: Mutex<BTreeMap<PortNum, Receiver>>,
}

impl MuilicastGroup {
    /// Creates a new multicast group
    pub fn new(id: GroupId) -> Self {
        todo!()
    }

    pub fn add_member(&self, port_num: PortNum, receiver: Receiver) {
        todo!()
    }

    pub fn remove_member(&self, port_num: PortNum) {
        todo!()
    }
}

/// A set of group IDs.
#[derive(Debug, Clone, Copy, Pod)]
#[repr(C)]
pub struct GroupIdSet(u32);

impl GroupIdSet {
    pub const fn new_empty() -> Self {
        Self(0)
    }

    pub const fn new(groups: u32) -> Self {
        Self(groups)
    }

    pub fn ids_iter(&self) -> GroupIdIter<'_> {
        todo!()
    }
}

pub struct GroupIdIter<'a> {
    groups: &'a GroupIdSet,
    current: usize,
}

impl<'a> Iterator for GroupIdIter<'a> {
    type Item = GroupId;

    fn next(&mut self) -> Option<Self::Item> {
        todo!()
    }
}
