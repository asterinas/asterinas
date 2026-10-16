// SPDX-License-Identifier: MPL-2.0

#![short_vis_path::add(netlink)]

use crate::prelude::*;

/// A set of group IDs.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct GroupIdSet(u32);

impl GroupIdSet {
    /// Creates a new empty `GroupIdSet`.
    pub(in netlink) const fn new_empty() -> Self {
        Self(0)
    }

    /// Creates a new `GroupIdSet` with multiple groups.
    ///
    /// Each 1 bit in `groups` represent a group.
    pub(crate) const fn new(groups: u32) -> Self {
        Self(groups)
    }

    /// Creates an iterator over all group IDs.
    pub(in netlink) const fn ids_iter(&self) -> GroupIdIter {
        GroupIdIter::new(self)
    }

    /// Adds some new groups.
    pub(in netlink) fn add_groups(&mut self, groups: GroupIdSet) {
        self.0 |= groups.0;
    }

    /// Drops some groups.
    pub(in netlink) fn drop_groups(&mut self, groups: GroupIdSet) {
        self.0 &= !groups.0;
    }

    /// Sets new groups.
    #[expect(dead_code)]
    fn set_groups(&mut self, new_groups: u32) {
        self.0 = new_groups;
    }

    /// Clears all groups.
    #[expect(dead_code)]
    fn clear(&mut self) {
        self.0 = 0;
    }

    /// Checks if the set of group IDs is empty.
    #[expect(dead_code)]
    fn is_empty(&self) -> bool {
        self.0 == 0
    }

    /// Returns the group IDs as a u32.
    pub(crate) fn as_u32(&self) -> u32 {
        self.0
    }
}

/// Iterator over a set of group IDs.
pub(in netlink) struct GroupIdIter {
    groups: u32,
}

impl GroupIdIter {
    const fn new(groups: &GroupIdSet) -> Self {
        Self { groups: groups.0 }
    }
}

impl Iterator for GroupIdIter {
    type Item = GroupId;

    fn next(&mut self) -> Option<Self::Item> {
        if self.groups > 0 {
            let group_id = self.groups.trailing_zeros();
            self.groups &= self.groups - 1;
            return Some(group_id);
        }

        None
    }
}

pub(in netlink) const MAX_GROUPS: u32 = 32;
pub(in netlink) type GroupId = u32;
