// SPDX-License-Identifier: MPL-2.0

use super::TidDirOps;
use crate::{
    fs::{
        file::{InodeType, mkmod},
        procfs::template::{
            ProcDir, ProcDirOps, ReaddirEntry, StaticDirEntry, listed_entries_from_table,
            lookup_child_from_table, visit_listed_entries,
        },
        vfs::inode::Inode,
    },
    prelude::*,
    thread::Thread,
};

mod current;

pub(super) const ATTR_DIR_NAME: &str = "attr";

/// Represents the inode at `/proc/[pid]/attr`.
#[derive(Clone)]
pub(super) struct AttrDirOps(TidDirOps);

impl AttrDirOps {
    /// Creates the inode for `/proc/[pid]/attr`.
    pub(super) fn new_inode(dir: &TidDirOps, parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        ProcDir::new(Self(dir.clone()), parent, mkmod!(a+rx))
    }

    pub(super) fn tid_dir(&self) -> &TidDirOps {
        &self.0
    }

    const STATIC_ENTRIES: &'static [StaticDirEntry<
        fn(&AttrDirOps, Weak<dyn Inode>) -> Arc<dyn Inode>,
    >] = &[(
        "current",
        InodeType::File,
        current::CurrentFileOps::new_inode,
    )];
}

impl ProcDirOps for AttrDirOps {
    fn owner_thread(&self) -> Option<Arc<Thread>> {
        self.0.thread()
    }

    fn lookup_child(&self, this_dir: &ProcDir<Self>, name: &str) -> Result<Arc<dyn Inode>> {
        if let Some(child) = lookup_child_from_table(name, Self::STATIC_ENTRIES, |constructor| {
            constructor(self, this_dir.this_weak().clone())
        }) {
            return Ok(child);
        }

        return_errno_with_message!(Errno::ENOENT, "the file does not exist");
    }

    fn visit_entries_from_offset<'a, F>(&'a self, offset: usize, visit_fn: F) -> Result<()>
    where
        F: FnMut(ReaddirEntry<'a>) -> Result<()>,
    {
        visit_listed_entries(
            offset,
            listed_entries_from_table(Self::STATIC_ENTRIES),
            visit_fn,
        )
    }
}
