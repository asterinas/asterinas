// SPDX-License-Identifier: MPL-2.0

use aster_util::slot_vec::SlotVec;

use crate::{fs::vfs::inode::Inode, prelude::*};

pub trait DirEntryVecExt {
    /// Finds the entry by the `name`.
    fn find_entry_by_name(&self, name: &str) -> Option<&Arc<dyn Inode>>;

    /// Puts the entry given by `f` into the vector if it is not found by the `name`.
    fn put_entry_if_not_found(
        &mut self,
        name: &str,
        f: impl FnOnce() -> Arc<dyn Inode>,
    ) -> &Arc<dyn Inode>;

    /// Removes the entry by the `name`.
    fn remove_entry_by_name(&mut self, name: &str) -> Option<(String, Arc<dyn Inode>)>;
}

impl DirEntryVecExt for SlotVec<(String, Arc<dyn Inode>)> {
    fn find_entry_by_name(&self, name: &str) -> Option<&Arc<dyn Inode>> {
        if let Some((_, inode)) = self.iter().find(|(child_name, _)| child_name == name) {
            Some(inode)
        } else {
            None
        }
    }

    fn put_entry_if_not_found(
        &mut self,
        name: &str,
        f: impl FnOnce() -> Arc<dyn Inode>,
    ) -> &Arc<dyn Inode> {
        let idx = self
            .idxes_and_items()
            .find(|(_, (child_name, _))| child_name == name)
            .map(|(idx, _)| idx);
        let idx = if let Some(idx) = idx {
            idx
        } else {
            let inode = f();
            self.put((String::from(name), inode))
        };
        &self.get(idx).unwrap().1
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
