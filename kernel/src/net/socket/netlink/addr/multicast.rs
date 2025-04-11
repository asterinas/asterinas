// SPDX-License-Identifier: MPL-2.0

use crate::prelude::*;

/// A set of group IDs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GroupIdSet(u32);

impl GroupIdSet {
    /// Creates a new empty `GroupIdSet`.
    pub const fn new_empty() -> Self {
        Self(0)
    }

    /// Creates a new `GroupIdSet` with multiple groups.
    ///
    /// Each 1 bit in `groups` represent a group.
    pub const fn new(groups: u32) -> Self {
        Self(groups)
    }

    /// Creates an iterator over all group IDs.
    pub const fn ids_iter(&self) -> GroupIdIter {
        GroupIdIter::new(self)
    }

    /// Adds a new group.
    ///
    /// If the group already exists, this method will return an error.
    pub fn add_group(&mut self, group_id: GroupId) -> Result<()> {
        if group_id >= 32 {
            return_errno_with_message!(Errno::EINVAL, "the group ID is invalid");
        }

        let mask = 1u32 << group_id;
        if self.0 & mask != 0 {
            return_errno_with_message!(Errno::EINVAL, "the group ID already exists");
        }
        self.0 |= mask;

        Ok(())
    }

    /// Sets new groups.
    pub fn set_groups(&mut self, new_groups: u32) {
        self.0 = new_groups;
    }

    /// Clears all groups.
    pub fn clear(&mut self) {
        self.0 = 0;
    }

    /// Checks if the set of group IDs is empty.
    pub fn is_empty(&self) -> bool {
        self.0 == 0
    }

    /// Returns the group IDs as a u32.
    pub fn as_u32(&self) -> u32 {
        self.0
    }
}

/// Iterator over a set of group IDs.
pub struct GroupIdIter {
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

pub const MAX_GROUPS: u32 = 32;
pub type GroupId = u32;
