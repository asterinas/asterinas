// SPDX-License-Identifier: MPL-2.0

use super::{
    addr::{GroupId, PortNum},
    sender::Sender,
};
use crate::prelude::*;

/// A netlink multicast group.
///
/// Each group has a unique group ID,
/// which is a u32 value containing only one bit set to 1 and all other bits set to 0.
/// Each netlink protocol can have a maximum of 32 groups.
pub struct MuilicastGroup {
    group_id: GroupId,
    members: BTreeMap<PortNum, Sender>,
}

impl MuilicastGroup {
    /// Creates a new multicast group
    pub const fn new(group_id: GroupId) -> Self {
        Self {
            group_id,
            members: BTreeMap::new(),
        }
    }

    /// Returns whether the group contains a member
    pub fn contains_member(&self, port_num: PortNum) -> bool {
        self.members.contains_key(&port_num)
    }

    /// Adds a new member to the multicast group
    pub fn add_member(&mut self, port_num: PortNum, sender: Sender) {
        debug_assert!(!self.members.contains_key(&port_num));
        self.members.insert(port_num, sender);
    }

    /// Removes a member from the multicast group
    pub fn remove_member(&mut self, port_num: PortNum) {
        debug_assert!(self.members.contains_key(&port_num));
        self.members.remove(&port_num);
    }
}

/// A set of group IDs.
#[derive(Debug, Clone, Copy, Pod)]
#[repr(C)]
pub struct GroupIdSet(u32);

impl GroupIdSet {
    /// Creates a new empty `GroupIdSet`
    pub const fn new_empty() -> Self {
        Self(0)
    }

    /// Creates a new `GroupIdSet` with multiple groups.
    ///
    /// Each 1 bit in `groups` represent a group.
    pub const fn new(groups: u32) -> Self {
        Self(groups)
    }

    /// Creates an iterator over all group ids
    pub fn ids_iter(&self) -> GroupIdIter<'_> {
        GroupIdIter::new(self)
    }

    /// Adds a new group.
    ///
    /// If the group already exists, this method will return error.
    pub fn add_group(&mut self, group_id: GroupId) -> Result<()> {
        assert!(group_id >= 0 && group_id <= 31);
        let mask = 1u32 << group_id;
        if self.0 & mask != 0 {
            return_errno_with_message!(Errno::EINVAL, "group id already exists");
        }
        self.0 |= mask;

        Ok(())
    }

    /// Sets new groups
    pub fn set_groups(&mut self, new_groups: u32) {
        self.0 = new_groups;
    }

    /// Clears all groups
    pub fn clear(&mut self) {
        self.0 = 0;
    }
}

/// Iterator over a set of group ids.
pub struct GroupIdIter<'a> {
    groups: &'a GroupIdSet,
    current: u32,
}

impl<'a> GroupIdIter<'a> {
    const fn new(groups: &'a GroupIdSet) -> Self {
        Self { groups, current: 0 }
    }
}

impl<'a> Iterator for GroupIdIter<'a> {
    type Item = GroupId;

    fn next(&mut self) -> Option<Self::Item> {
        while self.current <= 31 {
            let mask = 1u32 << self.current;
            self.current += 1;

            if self.groups.0 & mask != 0 {
                return Some(self.current - 1);
            }
        }

        None
    }
}

pub const MAX_GROUPS: u32 = 32;

#[cfg(ktest)]
mod test {
    use alloc::{collections::BTreeSet, vec, vec::Vec};

    use aster_frame::early_println;

    use super::GroupIdSet;

    fn test_iter(group_ids: Vec<u32>) {
        let group_ids = group_ids.into_iter().collect::<BTreeSet<_>>();

        let group_id_set = {
            let mut groups = 0u32;
            for group in group_ids.iter() {
                let mask = 1u32 << *group;
                groups |= mask;
            }
            GroupIdSet::new(groups)
        };

        let new_group_ids = {
            let iter = group_id_set.ids_iter();
            iter.collect::<BTreeSet<_>>()
        };

        assert_eq!(group_ids, new_group_ids)
    }

    #[ktest]
    fn group_id_iter() {
        test_iter(vec![1, 2, 3]);
        test_iter(vec![0, 2, 4, 6, 8]);
        test_iter(vec![1, 10, 20, 30, 31]);
        test_iter(vec![31, 25, 18, 24, 29]);
    }
}
