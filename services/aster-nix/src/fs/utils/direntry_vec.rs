// SPDX-License-Identifier: MPL-2.0

use aster_util::slot_vec::SlotVec;

use super::Inode;
use crate::prelude::*;

pub trait DirEntryVecExt {
    /// If the entry is not found by `name`, use `f` to get the inode, then put the entry into vec.
    fn put_entry_if_not_found(&mut self, name: &str, f: impl Fn() -> Arc<dyn Inode>);

    /// Remove and returns the entry by name.
    /// Returns `None` if the entry has been removed.
    fn remove_entry_by_name(&mut self, name: &str) -> Option<(String, Arc<dyn Inode>)>;
}

impl DirEntryVecExt for SlotVec<(String, Arc<dyn Inode>)> {
    fn put_entry_if_not_found(&mut self, name: &str, f: impl Fn() -> Arc<dyn Inode>) {
        if !self.iter().any(|(child_name, _)| child_name == name) {
            let inode = f();
            self.put((String::from(name), inode));
        }
    }

    fn remove_entry_by_name(&mut self, name: &str) -> Option<(String, Arc<dyn Inode>)> {
        let idx = self
            .idxes_and_items()
            .find(|(_, (child_name, _))| child_name == name)
            .map(|(idx, _)| idx);
        if let Some(idx) = idx {
            self.remove(idx)
        } else {
            None
        }
    }
}
